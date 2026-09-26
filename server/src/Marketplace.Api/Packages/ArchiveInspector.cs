using System.IO.Compression;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Marketplace.Api.Packages;

/// <summary>What a package upload contains, after safety checks and root unwrapping.</summary>
public sealed record InspectedArchive(
    string SourceId,
    string PackageId,
    JsonObject PackageManifest,
    IReadOnlyList<string> ComponentKinds,
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
        using var stream = new MemoryStream(bytes.ToArray(), writable: false);
        using var zip = OpenZip(stream);
        var (_, prefix) = Scan(zip, bytes.Length);
        var manifestEntry = zip.GetEntry(prefix + ManifestFile)
            ?? throw new ProblemException(422, $"The archive has no {ManifestFile} at its root.");
        JsonObject manifest;
        try
        {
            using var manifestStream = manifestEntry.Open();
            manifest = JsonNode.Parse(manifestStream) as JsonObject
                ?? throw new ProblemException(422, $"{ManifestFile} is not a JSON object.");
        }
        catch (JsonException error)
        {
            throw new ProblemException(422, $"{ManifestFile} is not valid JSON: {error.Message}");
        }
        catch (InvalidDataException error)
        {
            throw new ProblemException(422, $"{ManifestFile} is corrupt in the archive: {error.Message}");
        }

        if (manifest["version"] is not JsonValue version || !version.TryGetValue<int>(out var number) || number != 2)
        {
            throw new ProblemException(422, $"{ManifestFile} must declare version 2.");
        }

        var sourceId = Text((manifest["source"] as JsonObject)?["id"])
            ?? throw new ProblemException(422, $"{ManifestFile} has no source.id string.");
        if (manifest["packages"] is not JsonArray packages || packages.Count != 1 || packages[0] is not JsonObject package)
        {
            throw new ProblemException(422, "A marketplace upload must contain exactly one package.");
        }

        var packageId = Text(package["id"])
            ?? throw new ProblemException(422, "The package has no id string.");
        var kinds = package["components"] is JsonArray components
            ? components.Select(component => Text((component as JsonObject)?["kind"]) ?? "unknown").Distinct().ToArray()
            : [];
        return new InspectedArchive(sourceId, packageId, (JsonObject)package.DeepClone(), kinds, prefix);
    }

    /// <summary>
    /// Applies the size, entry-count, and path checks without requiring a manifest. An archive
    /// without <c>agent-plugins.json</c> at its root holds a bare skill or skills to wrap.
    /// </summary>
    public static (string Prefix, bool HasManifest) Survey(ReadOnlyMemory<byte> bytes)
    {
        using var stream = new MemoryStream(bytes.ToArray(), writable: false);
        using var zip = OpenZip(stream);
        var (names, prefix) = Scan(zip, bytes.Length);
        return (prefix, names.Contains(prefix + ManifestFile));
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
                throw new ProblemException(422, $"Entry escapes the archive root: {entry.FullName}");
            }

            if (relative.EndsWith('/'))
            {
                Directory.CreateDirectory(target);
                continue;
            }

            Directory.CreateDirectory(Path.GetDirectoryName(target)!);
            try
            {
                entry.ExtractToFile(target, overwrite: false);
            }
            catch (InvalidDataException error)
            {
                throw new ProblemException(422, $"{entry.FullName} is corrupt in the archive: {error.Message}");
            }

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
            throw new ProblemException(422, $"The upload is not a zip archive: {error.Message}");
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

        SafePath(name);
        if (((entry.ExternalAttributes >> 16) & 0xF000) == SymlinkMode)
        {
            throw new ProblemException(422, $"Symbolic links are not accepted: {name}");
        }

        return name;
    }

    /// <summary>Rejects backslashes, absolute paths, and <c>.</c> or <c>..</c> segments in a relative path.</summary>
    public static void SafePath(string name)
    {
        if (name.Contains('\\'))
        {
            throw new ProblemException(422, $"Entry uses backslashes: {name}");
        }

        if (name.StartsWith('/') || name.Length > 1 && name[1] == ':')
        {
            throw new ProblemException(422, $"Entry has an absolute path: {name}");
        }

        if (name.Split('/').Any(segment => segment is ".." or "."))
        {
            throw new ProblemException(422, $"Entry contains a relative path segment: {name}");
        }
    }

    private static (List<string> Names, string Prefix) Scan(ZipArchive zip, long archiveBytes)
    {
        if (archiveBytes > MaxArchiveBytes)
        {
            throw new ProblemException(422, "The archive is larger than the 50 MB limit.");
        }

        if (zip.Entries.Count > MaxEntries)
        {
            throw new ProblemException(422, $"The archive has more than {MaxEntries} entries.");
        }

        var names = zip.Entries.Select(SafeName).OfType<string>().ToList();
        CheckUnique(names);
        if (zip.Entries.Sum(entry => entry.Length) > MaxUncompressedBytes)
        {
            throw new ProblemException(422, "The archive expands to more than 200 MB.");
        }

        return (names, RootPrefix(names));
    }

    /// <summary>
    /// Rejects two entries with one name, ignoring case because installs land on Windows, and a file
    /// that another entry uses as a directory. Either would fail or overwrite on extraction.
    /// </summary>
    private static void CheckUnique(List<string> names)
    {
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var files = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var name in names)
        {
            if (!seen.Add(name))
            {
                throw new ProblemException(422, $"The archive holds {name} more than once (names are compared ignoring case).");
            }

            if (!name.EndsWith('/'))
            {
                files.Add(name);
            }
        }

        foreach (var name in names)
        {
            for (var slash = name.IndexOf('/'); slash > 0; slash = name.IndexOf('/', slash + 1))
            {
                if (files.Contains(name[..slash]))
                {
                    throw new ProblemException(422, $"The archive holds {name[..slash]} as both a file and a folder.");
                }
            }
        }
    }

    /// <summary>A JSON string value, or null for anything else.</summary>
    private static string? Text(JsonNode? node) => node is JsonValue value && value.TryGetValue<string>(out var text) ? text : null;

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
