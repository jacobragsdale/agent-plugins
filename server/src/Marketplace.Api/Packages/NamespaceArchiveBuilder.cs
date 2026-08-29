using System.IO.Compression;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Marketplace.Api.Packages;

public sealed record NamespaceSource(string Namespace, string DisplayName, string Description);

public sealed record NamespacePackage(string PackageId, JsonObject PackageManifest, ReadOnlyMemory<byte> Archive, string RootPrefix);

public sealed record BuiltArchive(byte[] Bytes, string Digest, int PackageCount);

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
        using (var zip = new ZipArchive(output, ZipArchiveMode.Create, leaveOpen: true))
        {
            WriteEntry(zip, ArchiveInspector.ManifestFile, Encoding.UTF8.GetBytes(manifest.ToJsonString(ManifestJson) + "\n"), 0);
            foreach (var package in ordered)
            {
                CopyPackage(zip, package);
            }
        }

        var bytes = output.ToArray();
        return new BuiltArchive(bytes, Convert.ToHexStringLower(SHA256.HashData(bytes)), ordered.Length);
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

    private static void CopyPackage(ZipArchive zip, NamespacePackage package)
    {
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
        }
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
