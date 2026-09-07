using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Marketplace.Api.Access;
using Marketplace.Api.Auth;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Marketplace.Api.Packages;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Options;

namespace Marketplace.Api.Catalog;

public sealed record CatalogDocument(byte[] Bytes, string Digest);

public sealed record IndexPublisher(string Account, string DisplayName);

public sealed record IndexPackage(
    string Id,
    string Namespace,
    string PackageId,
    string Name,
    string Description,
    string Version,
    IndexPublisher Publisher,
    string Lane,
    string[] Tags,
    string[] ComponentKinds,
    DateTime PublishedAt,
    int Installs,
    int InstalledBase,
    bool Restricted);

public sealed record IndexDocument(DateTime GeneratedAt, IReadOnlyList<IndexPackage> Packages);

public sealed class CatalogService(
    MarketplaceDbContext db,
    AccessService access,
    IOptions<ServerOptions> server,
    IOptions<AuthOptions> auth,
    TimeProvider timeProvider)
{
    private static readonly JsonSerializerOptions CatalogJson = new(JsonSerializerDefaults.Web) { WriteIndented = true };
    private static readonly TimeSpan InstalledBaseWindow = TimeSpan.FromDays(30);

    /// <summary>
    /// The catalog v1 document the desktop app already understands, one listed source per namespace.
    /// A namespace is listed only when the caller may see at least one of its packages.
    /// </summary>
    public async Task<CatalogDocument> CatalogAsync(MarketplaceIdentity identity, CancellationToken cancellationToken)
    {
        var visible = (await access.VisiblePackagesAsync(identity, null, cancellationToken))
            .GroupBy(package => package.Namespace, StringComparer.Ordinal)
            .ToDictionary(group => group.Key, group => group.Count(), StringComparer.Ordinal);
        var archives = await db.NamespaceArchives
            .Where(archive => archive.PackageCount > 0)
            .OrderBy(archive => archive.Namespace)
            .Select(archive => new { archive.Namespace, archive.GeneratedAt })
            .ToListAsync(cancellationToken);
        var publishers = await db.Publishers.ToDictionaryAsync(publisher => publisher.Namespace, cancellationToken);
        var baseUrl = server.Value.PublicBaseUrl.TrimEnd('/');
        var document = new
        {
            version = 1,
            repository = new
            {
                id = server.Value.CatalogId,
                name = server.Value.CatalogName,
                description = server.Value.CatalogDescription,
            },
            sources = archives.Where(archive => visible.ContainsKey(archive.Namespace)).Select(archive =>
            {
                var publisher = publishers.GetValueOrDefault(archive.Namespace);
                var displayName = publisher?.DisplayName ?? archive.Namespace;
                return new
                {
                    name = displayName,
                    description = $"Packages published by {displayName}.",
                    url = $"{baseUrl}/api/sources/{archive.Namespace}/archive",
                    sourceId = archive.Namespace,
                    publisher = displayName,
                    packageCount = visible[archive.Namespace],
                    updatedAt = archive.GeneratedAt,
                };
            }).ToArray(),
        };
        var bytes = Encoding.UTF8.GetBytes(JsonSerializer.Serialize(document, CatalogJson) + "\n");
        return new CatalogDocument(bytes, Convert.ToHexStringLower(SHA256.HashData(bytes)));
    }

    public async Task<IndexDocument> IndexAsync(MarketplaceIdentity identity, CancellationToken cancellationToken)
    {
        var packages = await db.Packages.Include(package => package.Versions).ToListAsync(cancellationToken);
        var publishers = await db.Publishers.ToDictionaryAsync(publisher => publisher.Namespace, cancellationToken);
        var stats = await StatsAsync(cancellationToken);
        var rules = await access.RulesAsync(cancellationToken);
        var entries = new List<IndexPackage>(packages.Count);
        foreach (var package in packages.OrderBy(package => package.Namespace, StringComparer.Ordinal).ThenBy(package => package.PackageId, StringComparer.Ordinal))
        {
            var latest = PublishService.LatestVersion(package);
            if (latest is null || !AccessService.IsVisible(rules, identity, package.Namespace, package.PackageId))
            {
                continue;
            }

            var publisher = publishers.GetValueOrDefault(package.Namespace);
            var (installs, installedBase) = stats.GetValueOrDefault(package.CanonicalId);
            entries.Add(new IndexPackage(
                package.CanonicalId,
                package.Namespace,
                package.PackageId,
                package.Name,
                package.Description,
                latest.Version,
                new IndexPublisher(publisher?.Account ?? package.Namespace, publisher?.DisplayName ?? package.Namespace),
                Lane(package.Namespace),
                package.Tags,
                latest.ComponentKinds,
                latest.PublishedAt,
                installs,
                installedBase,
                rules.ContainsKey(package.CanonicalId) || rules.ContainsKey(package.Namespace)));
        }

        return new IndexDocument(timeProvider.GetUtcNow().UtcDateTime, entries);
    }

    private string Lane(string ns) =>
        ns == MarketplaceIdentity.OfficialNamespace ? "official"
        : auth.Value.TeamNamespaces.Any(team => team.Namespace == ns) ? "team"
        : "personal";

    /// <summary>Install events over all time and the installed base from heartbeats in the last 30 days.</summary>
    public async Task<Dictionary<string, (int Installs, int InstalledBase)>> StatsAsync(CancellationToken cancellationToken)
    {
        var installs = await db.Events
            .Where(clientEvent => clientEvent.Kind == "install" && clientEvent.PackageId != null)
            .GroupBy(clientEvent => clientEvent.PackageId!)
            .Select(group => new { PackageId = group.Key, Count = group.Count() })
            .ToListAsync(cancellationToken);
        var since = timeProvider.GetUtcNow().UtcDateTime - InstalledBaseWindow;
        var installedSets = await db.Heartbeats
            .Where(heartbeat => heartbeat.OccurredAt >= since)
            .Select(heartbeat => heartbeat.Installed)
            .ToListAsync(cancellationToken);
        var result = new Dictionary<string, (int, int)>(StringComparer.Ordinal);
        foreach (var install in installs)
        {
            result[install.PackageId] = (install.Count, 0);
        }

        foreach (var installed in installedSets)
        {
            foreach (var id in installed.Distinct(StringComparer.Ordinal))
            {
                var current = result.GetValueOrDefault(id);
                result[id] = (current.Item1, current.Item2 + 1);
            }
        }

        return result;
    }
}
