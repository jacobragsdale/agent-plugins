using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Marketplace.Api.Access;
using Marketplace.Api.Auth;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Marketplace.Api.Events;
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
    bool SharedWithYou,
    DateTime CreatedAt,
    string? Changelog,
    bool? McpApproved,
    string[] McpTransports);

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

/// <summary>
/// <see cref="Revoked"/>: pulled packages the caller could see or last reported installed, for clients to uninstall.
/// <see cref="GeneratedAt"/> is when the listed content last changed, so the same content is the same bytes and ETag.
/// </summary>
public sealed record IndexDocument(DateTime GeneratedAt, IReadOnlyList<IndexPackage> Packages, IReadOnlyList<IndexBundle> Bundles, IReadOnlyList<string> Revoked);

public sealed class CatalogService(
    MarketplaceDbContext db,
    AccessService access,
    EventsService events,
    IOptions<ServerOptions> server)
{
    private static readonly JsonSerializerOptions CatalogJson = new(JsonSerializerDefaults.Web) { WriteIndented = true };

    /// <summary>
    /// The catalog v1 document the desktop app already understands, one listed source per namespace.
    /// A namespace is listed only when the caller may see at least one of its packages.
    /// </summary>
    public async Task<CatalogDocument> CatalogAsync(MarketplaceIdentity identity, CancellationToken cancellationToken)
    {
        var live = await access.LivePackagesAsync(null, cancellationToken);
        var rules = await access.RulesAsync(cancellationToken);
        var liveCounts = live.GroupBy(package => package.Namespace, StringComparer.Ordinal).ToDictionary(group => group.Key, group => group.Count(), StringComparer.Ordinal);
        var visible = live
            .Where(package => AccessService.IsVisible(rules, identity, package.Namespace, package.PackageId, package.Gated))
            .GroupBy(package => package.Namespace, StringComparer.Ordinal)
            .ToDictionary(group => group.Key, group => group.Select(package => package.PackageId).ToArray(), StringComparer.Ordinal);
        var archives = await db.NamespaceArchives
            .Where(archive => archive.PackageCount > 0)
            .OrderBy(archive => archive.Namespace)
            .Select(archive => new { archive.Namespace, archive.GeneratedAt, archive.Digest })
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
                    packageCount = visible[archive.Namespace].Length,
                    updatedAt = archive.GeneratedAt,
                    // What the archive endpoint's ETag will be for this caller, so an unchanged source needs no request.
                    digest = AccessService.ArchiveDigest(archive.Digest, visible[archive.Namespace], liveCounts.GetValueOrDefault(archive.Namespace)),
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
        var stats = await events.CountsAsync(cancellationToken);
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
                AccessService.SharedWith(rules, identity, package.Namespace, package.PackageId),
                package.CreatedAt,
                latest.Changelog,
                latest.ComponentKinds.Contains(PublishService.McpServerKind) ? package.McpApprovedBy is not null : null,
                McpServerSummary.FromJson(latest.McpServersJson).Select(server => server.Transport).Distinct(StringComparer.Ordinal).Order(StringComparer.Ordinal).ToArray()));
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

        // Someone who lost access still gets the removal for what any of their PCs reported installed.
        var installed = (await db.Heartbeats.AsNoTracking().Where(heartbeat => heartbeat.Account == identity.Account).Select(heartbeat => heartbeat.Installed).ToListAsync(cancellationToken))
            .SelectMany(ids => ids)
            .ToHashSet(StringComparer.Ordinal);
        var revoked = packages
            .Where(package => package.RevokedAt is not null
                && (installed.Contains(package.CanonicalId) || AccessService.IsVisible(rules, identity, package.Namespace, package.PackageId, WasGated(package))))
            .Select(package => package.CanonicalId)
            .Order(StringComparer.Ordinal)
            .ToList();
        var changed = packages.SelectMany(package => new[] { package.UpdatedAt, package.CreatedAt, package.RevokedAt ?? default })
            .Concat(bundles.Select(bundle => bundle.UpdatedAt))
            .DefaultIfEmpty()
            .Max();
        return new IndexDocument(DateTime.SpecifyKind(changed, DateTimeKind.Utc), entries, bundles, revoked);
    }

    /// <summary>
    /// Whether the package was waiting for review when it was revoked. A revoked package has no live version, so
    /// <see cref="PublishService.Gated"/> can't say; one the caller never saw is not theirs to hear revoked.
    /// </summary>
    private static bool WasGated(Package package) =>
        package.McpApprovedBy is null
        && package.Versions.Where(version => !version.Yanked).MaxBy(version => SemVer.Parse(version.Version))?.ComponentKinds.Contains(PublishService.McpServerKind) == true;

    private static IndexPublisher Publisher(string ns, Publisher? publisher) =>
        new(publisher?.Account ?? ns, publisher?.DisplayName ?? ns);
}
