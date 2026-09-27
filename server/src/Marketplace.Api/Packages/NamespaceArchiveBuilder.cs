using System.IO.Compression;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Marketplace.Api.Packages;

public sealed record NamespaceSource(string Namespace, string DisplayName, string Description);

public sealed record NamespacePackage(string PackageId, JsonObject PackageManifest, ReadOnlyMemory<byte> Archive, string RootPrefix);

/// <summary><see cref="Files"/> and <see cref="UncompressedBytes"/> are what the desktop app counts against its limits.</summary>
public sealed record BuiltArchive(byte[] Bytes, string Digest, int PackageCount, int Files = 0, long UncompressedBytes = 0);

/// <summary>
/// Merges the latest version of every package in a namespace into one manifest v2 source archive:
/// each package's tree lives under <c>{packageId}/</c> and its component paths are prefixed to match.
/// Entry timestamps are fixed so the same content yields the same digest.
/// </summary>
public static class NamespaceArchiveBuilder
{
    private static readonly DateTimeOffset FixedTimestamp = new(2000, 1, 1, 0, 0, 0, TimeSpan.Zero);
    private static readonly JsonSerializerOptions ManifestJson = new() { WriteIndented = true };

    public static BuiltArchive Build(NamespaceSource source, IReadOnlyList<NamespacePackage> packages)
    {
        var ordered = packages.OrderBy(package => package.PackageId, StringComparer.Ordinal).ToArray();
        var manifest = new JsonObject
        {
            ["version"] = 2,
            ["source"] = new JsonObject
            {
                ["id"] = source.Namespace,
                ["name"] = Clamp(source.DisplayName, 120),
                ["description"] = Clamp(source.Description, 1024),
            },
            ["packages"] = new JsonArray(ordered.Select(package => PrefixedManifest(package.PackageId, package.PackageManifest)).ToArray()),
        };

        using var output = new MemoryStream();
        var files = 1;
        long uncompressed;
        using (var zip = new ZipArchive(output, ZipArchiveMode.Create, leaveOpen: true))
        {
            var manifestBytes = Encoding.UTF8.GetBytes(manifest.ToJsonString(ManifestJson) + "\n");
            WriteEntry(zip, ArchiveInspector.ManifestFile, manifestBytes, 0);
            uncompressed = manifestBytes.Length;
            foreach (var package in ordered)
            {
                var (count, size) = CopyPackage(zip, package);
                files += count;
                uncompressed += size;
            }
        }

        var bytes = output.ToArray();
        return new BuiltArchive(bytes, Convert.ToHexStringLower(SHA256.HashData(bytes)), ordered.Length, files, uncompressed);
    }

    /// <summary>
    /// The subset of a built namespace archive that lists and contains only <paramref name="keepPackageIds"/>.
    /// Entries are copied in stored order with the same fixed timestamps, so one subset always has one digest.
    /// </summary>
    // ponytail: re-zips on every request for a partially visible namespace; cache by (namespace, digest, kept ids) if it shows in profiles.
    public static BuiltArchive Filter(byte[] fullArchive, IReadOnlySet<string> keepPackageIds)
    {
        using var input = ArchiveInspector.OpenZip(new MemoryStream(fullArchive, writable: false));
        var manifestEntry = input.GetEntry(ArchiveInspector.ManifestFile)
            ?? throw new InvalidOperationException("The namespace archive has no manifest.");
        JsonObject manifest;
        using (var manifestStream = manifestEntry.Open())
        {
            manifest = JsonNode.Parse(manifestStream) as JsonObject
                ?? throw new InvalidOperationException("The namespace archive manifest is not an object.");
        }

        var packages = manifest["packages"] as JsonArray
            ?? throw new InvalidOperationException("The namespace archive manifest has no packages.");
        var kept = packages.OfType<JsonObject>()
            .Where(package => package["id"] is JsonValue id && id.TryGetValue<string>(out var text) && keepPackageIds.Contains(text))
            .Select(package => package.DeepClone())
            .ToArray();
        manifest["packages"] = new JsonArray(kept);

        using var output = new MemoryStream();
        using (var zip = new ZipArchive(output, ZipArchiveMode.Create, leaveOpen: true))
        {
            WriteEntry(zip, ArchiveInspector.ManifestFile, Encoding.UTF8.GetBytes(manifest.ToJsonString(ManifestJson) + "\n"), 0);
            foreach (var entry in input.Entries)
            {
                var slash = entry.FullName.IndexOf('/');
                if (slash <= 0 || !keepPackageIds.Contains(entry.FullName[..slash]))
                {
                    continue;
                }

                using var content = entry.Open();
                using var buffer = new MemoryStream();
                content.CopyTo(buffer);
                WriteEntry(zip, entry.FullName, buffer.ToArray(), entry.ExternalAttributes);
            }
        }

        var bytes = output.ToArray();
        return new BuiltArchive(bytes, Convert.ToHexStringLower(SHA256.HashData(bytes)), kept.Length);
    }

    private static JsonNode PrefixedManifest(string packageId, JsonObject packageManifest)
    {
        var clone = (JsonObject)packageManifest.DeepClone();
        if (clone["components"] is JsonArray components)
        {
            foreach (var component in components.OfType<JsonObject>())
            {
                if (component["path"] is JsonValue pathValue && pathValue.TryGetValue<string>(out var path))
                {
                    component["path"] = $"{packageId}/{path.TrimStart('/')}";
                }
            }
        }

        return clone;
    }

    private static (int Files, long Bytes) CopyPackage(ZipArchive zip, NamespacePackage package)
    {
        var (files, total) = (0, 0L);
        using var stream = new MemoryStream(package.Archive.ToArray(), writable: false);
        using var input = ArchiveInspector.OpenZip(stream);
        foreach (var entry in input.Entries.OrderBy(entry => entry.FullName, StringComparer.Ordinal))
        {
            var name = ArchiveInspector.SafeName(entry);
            if (name is null || !name.StartsWith(package.RootPrefix, StringComparison.Ordinal))
            {
                continue;
            }

            var relative = name[package.RootPrefix.Length..];
            if (relative.Length == 0 || relative == ArchiveInspector.ManifestFile || relative.EndsWith('/'))
            {
                continue;
            }

            using var content = entry.Open();
            using var buffer = new MemoryStream();
            content.CopyTo(buffer);
            WriteEntry(zip, $"{package.PackageId}/{relative}", buffer.ToArray(), entry.ExternalAttributes);
            files++;
            total += buffer.Length;
        }

        return (files, total);
    }

    private static void WriteEntry(ZipArchive zip, string name, byte[] bytes, int externalAttributes)
    {
        var entry = zip.CreateEntry(name, CompressionLevel.Optimal);
        entry.LastWriteTime = FixedTimestamp;
        if (externalAttributes != 0)
        {
            entry.ExternalAttributes = externalAttributes;
        }

        using var target = entry.Open();
        target.Write(bytes);
    }

    private static string Clamp(string value, int max)
    {
        var trimmed = value.Trim();
        if (trimmed.Length == 0)
        {
            trimmed = "-";
        }

        return trimmed.Length <= max ? trimmed : trimmed[..max];
    }
}
