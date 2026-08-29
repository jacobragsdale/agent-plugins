using System.IO.Compression;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Marketplace.Api.Packages;

public sealed class ArchiveRejectedException(string message) : Exception(message);

/// <summary>What a package upload contains, after safety checks and root unwrapping.</summary>
public sealed record InspectedArchive(
    string SourceId,
    string PackageId,
    JsonObject PackageManifest,
    IReadOnlyList<string> ComponentKinds,
    /// <summary>Entry names relative to the source root, in archive order.</summary>
    IReadOnlyList<string> Entries,
    /// <summary>The archive's root prefix, empty when <c>agent-plugins.json</c> is at the top level.</summary>
    string RootPrefix);

/// <summary>
/// Reads a zip upload safely: no absolute paths, no <c>..</c>, no symlinks, bounded entry count and
/// uncompressed size. Requires manifest v2 with exactly one package.
/// </summary>
public static class ArchiveInspector
{
    public const string ManifestFile = "agent-plugins.json";
    public const long MaxArchiveBytes = 50L * 1024 * 1024;
    public const long MaxUncompressedBytes = 200L * 1024 * 1024;
    public const int MaxEntries = 5000;
    private const int SymlinkMode = 0xA000;

    public static InspectedArchive Inspect(ReadOnlyMemory<byte> bytes)
    {
        if (bytes.Length > MaxArchiveBytes)
        {
            throw new ArchiveRejectedException("The archive is larger than the 50 MB limit.");
        }

        using var stream = new MemoryStream(bytes.ToArray(), writable: false);
        using var zip = OpenZip(stream);
        var names = new List<string>();
        long uncompressed = 0;
        foreach (var entry in zip.Entries)
        {
            if (++uncompressed > MaxEntries)
            {
                throw new ArchiveRejectedException($"The archive has more than {MaxEntries} entries.");
            }

            var name = SafeName(entry);
            if (name is null)
            {
                continue;
            }

            names.Add(name);
        }

        uncompressed = zip.Entries.Sum(entry => entry.Length);
        if (uncompressed > MaxUncompressedBytes)
        {
            throw new ArchiveRejectedException("The archive expands to more than 200 MB.");
        }

        var prefix = RootPrefix(names);
        var manifestEntry = zip.GetEntry(prefix + ManifestFile)
            ?? throw new ArchiveRejectedException($"The archive has no {ManifestFile} at its root.");
        JsonObject manifest;
        using (var manifestStream = manifestEntry.Open())
        {
            try
            {
                manifest = JsonNode.Parse(manifestStream) as JsonObject
                    ?? throw new ArchiveRejectedException($"{ManifestFile} is not a JSON object.");
            }
            catch (JsonException error)
            {
                throw new ArchiveRejectedException($"{ManifestFile} is not valid JSON: {error.Message}");
            }
        }

        if (manifest["version"]?.GetValueKind() != JsonValueKind.Number || manifest["version"]!.GetValue<int>() != 2)
        {
            throw new ArchiveRejectedException($"{ManifestFile} must declare version 2.");
        }

        var sourceId = manifest["source"]?["id"]?.GetValue<string>()
            ?? throw new ArchiveRejectedException($"{ManifestFile} has no source.id.");
        if (manifest["packages"] is not JsonArray packages || packages.Count != 1 || packages[0] is not JsonObject package)
        {
            throw new ArchiveRejectedException("A marketplace upload must contain exactly one package.");
        }

        var packageId = package["id"]?.GetValue<string>()
            ?? throw new ArchiveRejectedException("The package has no id.");
        var kinds = package["components"] is JsonArray components
            ? components.Select(component => component?["kind"]?.GetValue<string>() ?? "unknown").Distinct().ToArray()
            : [];
        var relative = names
            .Where(name => name.StartsWith(prefix, StringComparison.Ordinal))
            .Select(name => name[prefix.Length..])
            .Where(name => name.Length > 0)
            .ToArray();
        return new InspectedArchive(sourceId, packageId, (JsonObject)package.DeepClone(), kinds, relative, prefix);
    }

    /// <summary>Extracts the source root into <paramref name="destination"/> for validation.</summary>
    public static void ExtractTo(ReadOnlyMemory<byte> bytes, string rootPrefix, string destination)
    {
        using var stream = new MemoryStream(bytes.ToArray(), writable: false);
        using var zip = OpenZip(stream);
        var root = Path.GetFullPath(destination);
        Directory.CreateDirectory(root);
        foreach (var entry in zip.Entries)
        {
            var name = SafeName(entry);
            if (name is null || !name.StartsWith(rootPrefix, StringComparison.Ordinal))
            {
                continue;
            }

            var relative = name[rootPrefix.Length..];
            if (relative.Length == 0)
            {
                continue;
            }

            var target = Path.GetFullPath(Path.Combine(root, relative));
            if (!target.StartsWith(root + Path.DirectorySeparatorChar, StringComparison.Ordinal))
            {
                throw new ArchiveRejectedException($"Entry escapes the archive root: {entry.FullName}");
            }

            if (relative.EndsWith('/'))
            {
                Directory.CreateDirectory(target);
                continue;
            }

            Directory.CreateDirectory(Path.GetDirectoryName(target)!);
            entry.ExtractToFile(target, overwrite: false);
            if (!OperatingSystem.IsWindows() && ((entry.ExternalAttributes >> 16) & 0x49) != 0)
            {
                File.SetUnixFileMode(target, File.GetUnixFileMode(target) | UnixFileMode.UserExecute | UnixFileMode.GroupExecute | UnixFileMode.OtherExecute);
            }
        }
    }

    public static ZipArchive OpenZip(Stream stream)
    {
        try
        {
            return new ZipArchive(stream, ZipArchiveMode.Read, leaveOpen: true);
        }
        catch (InvalidDataException error)
        {
            throw new ArchiveRejectedException($"The upload is not a zip archive: {error.Message}");
        }
    }

    /// <summary>Returns the normalized entry name, or null for a directory placeholder without content.</summary>
    public static string? SafeName(ZipArchiveEntry entry)
    {
        var name = entry.FullName;
        if (name.Length == 0)
        {
            return null;
        }

        if (name.Contains('\\'))
        {
            throw new ArchiveRejectedException($"Entry uses backslashes: {name}");
        }

        if (name.StartsWith('/') || name.Length > 1 && name[1] == ':')
        {
            throw new ArchiveRejectedException($"Entry has an absolute path: {name}");
        }

        var segments = name.Split('/');
        if (segments.Any(segment => segment is ".." or "."))
        {
            throw new ArchiveRejectedException($"Entry contains a relative path segment: {name}");
        }

        if (((entry.ExternalAttributes >> 16) & 0xF000) == SymlinkMode)
        {
            throw new ArchiveRejectedException($"Symbolic links are not accepted: {name}");
        }

        return name;
    }

    /// <summary>When every entry lives under one top-level directory, that directory is the root.</summary>
    public static string RootPrefix(IReadOnlyList<string> names)
    {
        if (names.Any(name => name == ManifestFile))
        {
            return string.Empty;
        }

        string? top = null;
        foreach (var name in names)
        {
            var slash = name.IndexOf('/');
            if (slash <= 0)
            {
                return string.Empty;
            }

            var candidate = name[..slash];
            if (top is null)
            {
                top = candidate;
            }
            else if (top != candidate)
            {
                return string.Empty;
            }
        }

        return top is null ? string.Empty : top + "/";
    }
}
