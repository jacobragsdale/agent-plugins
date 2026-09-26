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
    bool Restricted,
    bool SharedWithYou);

public sealed record IndexBundle(
    string Id,
    string Namespace,
    string BundleId,
    string Name,
    string Description,
    IndexPublisher Publisher,
    string Lane,
    string[] Members,
    DateTime UpdatedAt,
    bool Restricted,
    bool SharedWithYou);

/// <summary><see cref="Revoked"/>: pulled packages the caller could see or last reported installed, for clients to uninstall.</summary>
public sealed record IndexDocument(DateTime GeneratedAt, IReadOnlyList<IndexPackage> Packages, IReadOnlyList<IndexBundle> Bundles, IReadOnlyList<string> Revoked);

public sealed class CatalogService(
    MarketplaceDbContext db,
    AccessService access,
    IOptions<ServerOptions> server,
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
        var packages = await db.Packages.AsNoTracking()
            .Include(package => package.Versions.Where(version => !version.Yanked))
            .ToListAsync(cancellationToken);
        var publishers = await db.Publishers.ToDictionaryAsync(publisher => publisher.Namespace, cancellationToken);
        var stats = await StatsAsync(cancellationToken);
        var rules = await access.RulesAsync(cancellationToken);
        var entries = new List<IndexPackage>(packages.Count);
        foreach (var package in packages.OrderBy(package => package.Namespace, StringComparer.Ordinal).ThenBy(package => package.PackageId, StringComparer.Ordinal))
        {
            var latest = PublishService.LatestVersion(package);
            if (latest is null || !AccessService.IsVisible(rules, identity, package.Namespace, package.PackageId, PublishService.Gated(package)))
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
                Publisher(package.Namespace, publisher),
                IdentityResolver.Lane(package.Namespace, publisher),
                package.Tags,
                latest.ComponentKinds,
                latest.PublishedAt,
                installs,
                installedBase,
                AccessService.Effective(rules, package.Namespace, package.PackageId).Private,
                AccessService.SharedWith(rules, identity, package.Namespace, package.PackageId)));
        }

        var visible = entries.Select(entry => entry.Id).ToHashSet(StringComparer.Ordinal);
        var bundles = (await db.Bundles.AsNoTracking().OrderBy(bundle => bundle.Namespace).ThenBy(bundle => bundle.BundleId).ToListAsync(cancellationToken))
            .Where(bundle => AccessService.IsVisible(rules, identity, bundle.Namespace, bundle.BundleId))
            .Select(bundle => (bundle, members: bundle.Members.Where(visible.Contains).ToArray()))
            .Where(pair => pair.members.Length > 0)
            .Select(pair => new IndexBundle(
                pair.bundle.CanonicalId,
                pair.bundle.Namespace,
                pair.bundle.BundleId,
                pair.bundle.Name,
                pair.bundle.Description,
                Publisher(pair.bundle.Namespace, publishers.GetValueOrDefault(pair.bundle.Namespace)),
                IdentityResolver.Lane(pair.bundle.Namespace, publishers.GetValueOrDefault(pair.bundle.Namespace)),
                pair.members,
                pair.bundle.UpdatedAt,
                AccessService.Effective(rules, pair.bundle.Namespace, pair.bundle.BundleId).Private,
                AccessService.SharedWith(rules, identity, pair.bundle.Namespace, pair.bundle.BundleId)))
            .ToList();

        // Someone who lost access still gets the removal for what their PC reported installed.
        var installed = await db.Heartbeats.AsNoTracking().Where(heartbeat => heartbeat.Account == identity.Account).Select(heartbeat => heartbeat.Installed).SingleOrDefaultAsync(cancellationToken) ?? [];
        var revoked = packages
            .Where(package => package.RevokedAt is not null
                && (installed.Contains(package.CanonicalId) || AccessService.IsVisible(rules, identity, package.Namespace, package.PackageId)))
            .Select(package => package.CanonicalId)
            .Order(StringComparer.Ordinal)
            .ToList();
        return new IndexDocument(timeProvider.GetUtcNow().UtcDateTime, entries, bundles, revoked);
    }

    private static IndexPublisher Publisher(string ns, Publisher? publisher) =>
        new(publisher?.Account ?? ns, publisher?.DisplayName ?? ns);

    /// <summary>Install events over all time and the installed base from heartbeats in the last 30 days.</summary>
    public async Task<Dictionary<string, (int Installs, int InstalledBase)>> StatsAsync(CancellationToken cancellationToken)
    {
        var installs = await db.Events
            .Where(clientEvent => clientEvent.Kind == "install" && clientEvent.PackageId != null)
            .GroupBy(clientEvent => clientEvent.PackageId!)
            .Select(group => new { PackageId = group.Key, Count = group.Count() })
            .ToListAsync(cancellationToken);
        var since = timeProvider.GetUtcNow().UtcDateTime - InstalledBaseWindow;
        // Installed is de-duplicated on ingest, so each heartbeat counts once per package.
        var installedBase = await db.Heartbeats
            .Where(heartbeat => heartbeat.OccurredAt >= since)
            .SelectMany(heartbeat => heartbeat.Installed)
            .GroupBy(id => id)
            .Select(group => new { PackageId = group.Key, Count = group.Count() })
            .ToListAsync(cancellationToken);
        var result = new Dictionary<string, (int, int)>(StringComparer.Ordinal);
        foreach (var install in installs)
        {
            result[install.PackageId] = (install.Count, 0);
        }

        foreach (var installed in installedBase)
        {
            result[installed.PackageId] = (result.GetValueOrDefault(installed.PackageId).Item1, installed.Count);
        }

        return result;
    }
}
