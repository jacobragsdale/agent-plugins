using Marketplace.Api.Access;
using Marketplace.Api.Auth;
using Marketplace.Api.Catalog;
using Marketplace.Api.Data;
using Microsoft.EntityFrameworkCore;

namespace Marketplace.Api.Packages;

public sealed record BundleRequest(string? Name, string? Description, string[]? Members);

/// <summary><see cref="Members"/> are the ones the caller can see; <see cref="HiddenMembers"/> counts the rest.</summary>
public sealed record BundleView(
    string Id,
    string Namespace,
    string BundleId,
    string Name,
    string Description,
    string[] Members,
    int HiddenMembers,
    IndexPublisher Publisher,
    string Lane,
    DateTime UpdatedAt,
    string UpdatedBy,
    bool Owned,
    Visibility Visibility,
    Visibility Effective);

/// <summary>
/// Named lists of live packages, from any namespace, that install together. A bundle has no archive and
/// no versions; its id shares its namespace's package id space, and its members follow their own access.
/// </summary>
public sealed class BundleService(MarketplaceDbContext db, AccessService access, PublishService publish, TimeProvider timeProvider)
{
    public const int MaxMembers = 50;

    public async Task<BundleView> SaveAsync(MarketplaceIdentity identity, string ns, string bundleId, BundleRequest request, CancellationToken cancellationToken)
    {
        PublishService.CheckTarget(identity, ns, bundleId);
        var name = request.Name?.Trim() ?? string.Empty;
        if (name.Length is 0 or > 120)
        {
            throw new ProblemException(422, "A bundle's name is 1 to 120 characters.");
        }

        var description = request.Description?.Trim() ?? string.Empty;
        if (description.Length > 1024)
        {
            throw new ProblemException(422, "A bundle's description is at most 1,024 characters.");
        }

        var members = (request.Members ?? []).Select(member => member?.Trim() ?? string.Empty).Where(member => member.Length > 0).Distinct(StringComparer.Ordinal).ToArray();
        if (members.Length is 0 or > MaxMembers)
        {
            throw new ProblemException(422, $"A bundle has 1 to {MaxMembers} packages.");
        }

        var visible = (await access.VisiblePackagesAsync(identity, null, cancellationToken)).Select(package => $"{package.Namespace}/{package.PackageId}").ToHashSet(StringComparer.Ordinal);
        if (members.Where(member => !visible.Contains(member)).ToArray() is { Length: > 0 } unknown)
        {
            throw new ProblemException(422, unknown.Length == 1
                ? $"{unknown[0]} is not a live package you can see."
                : $"{string.Join(", ", unknown)} are not live packages you can see.");
        }

        var now = timeProvider.GetUtcNow().UtcDateTime;
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);
        if (await db.Packages.AnyAsync(package => package.Namespace == ns && package.PackageId == bundleId, cancellationToken))
        {
            throw new ProblemException(409, $"{ns} already has a package called {bundleId}. Pick another name.");
        }

        await publish.ClaimPublisherAsync(identity, ns, now, cancellationToken);
        var bundle = await db.Bundles.FindAsync([ns, bundleId], cancellationToken);
        if (bundle is null)
        {
            bundle = db.Bundles.Add(new Bundle { Namespace = ns, BundleId = bundleId, Name = name, UpdatedBy = identity.Account, CreatedAt = now }).Entity;
        }

        bundle.Name = name;
        bundle.Description = description;
        bundle.Members = members;
        bundle.UpdatedBy = identity.Account;
        bundle.UpdatedAt = now;
        db.Audit(identity.Account, "bundle.save", bundle.CanonicalId, $"{members.Length} packages", now);
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        return await GetAsync(identity, ns, bundleId, cancellationToken);
    }

    public async Task<BundleView> GetAsync(MarketplaceIdentity identity, string ns, string bundleId, CancellationToken cancellationToken)
    {
        var bundle = await db.Bundles.AsNoTracking().SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.BundleId == bundleId, cancellationToken);
        var rules = await access.RulesAsync(cancellationToken);
        if (bundle is null || !AccessService.IsVisible(rules, identity, ns, bundleId))
        {
            throw ProblemException.NotFound($"The bundle {ns}/{bundleId}");
        }

        return (await ViewsAsync(identity, [bundle], cancellationToken))[0];
    }

    public async Task DeleteAsync(MarketplaceIdentity identity, string ns, string bundleId, CancellationToken cancellationToken)
    {
        if (!identity.Owns(ns))
        {
            throw ProblemException.NotOwner(identity.Account, ns);
        }

        var bundle = await db.Bundles.FindAsync([ns, bundleId], cancellationToken) ?? throw ProblemException.NotFound($"The bundle {ns}/{bundleId}");
        var target = bundle.CanonicalId;
        db.Bundles.Remove(bundle);
        await db.AccessRules.Where(rule => rule.Target == target).ExecuteDeleteAsync(cancellationToken);
        await db.Links.Where(link => link.Target == target).ExecuteDeleteAsync(cancellationToken);
        db.Audit(identity.Account, "bundle.delete", target, null, timeProvider.GetUtcNow().UtcDateTime);
        await db.SaveChangesAsync(cancellationToken);
    }

    /// <summary>Bundles in the caller's namespaces.</summary>
    public async Task<BundleView[]> MineAsync(MarketplaceIdentity identity, CancellationToken cancellationToken)
    {
        var namespaces = identity.Namespaces.ToArray();
        var bundles = await db.Bundles.AsNoTracking().Where(bundle => namespaces.Contains(bundle.Namespace)).OrderBy(bundle => bundle.Namespace).ThenBy(bundle => bundle.BundleId).ToListAsync(cancellationToken);
        return await ViewsAsync(identity, bundles, cancellationToken);
    }

    private async Task<BundleView[]> ViewsAsync(MarketplaceIdentity identity, IReadOnlyList<Bundle> bundles, CancellationToken cancellationToken)
    {
        var rules = await access.RulesAsync(cancellationToken);
        var visible = (await access.VisiblePackagesAsync(identity, null, cancellationToken)).Select(package => $"{package.Namespace}/{package.PackageId}").ToHashSet(StringComparer.Ordinal);
        var namespaces = bundles.Select(bundle => bundle.Namespace).Distinct().ToArray();
        var publishers = await db.Publishers.AsNoTracking().Where(publisher => namespaces.Contains(publisher.Namespace)).ToDictionaryAsync(publisher => publisher.Namespace, cancellationToken);
        return bundles.Select(bundle =>
        {
            var publisher = publishers.GetValueOrDefault(bundle.Namespace);
            var shown = bundle.Members.Where(visible.Contains).ToArray();
            return new BundleView(
                bundle.CanonicalId,
                bundle.Namespace,
                bundle.BundleId,
                bundle.Name,
                bundle.Description,
                shown,
                bundle.Members.Length - shown.Length,
                new IndexPublisher(publisher?.Account ?? bundle.Namespace, publisher?.DisplayName ?? bundle.Namespace),
                IdentityResolver.Lane(bundle.Namespace, publisher),
                bundle.UpdatedAt,
                bundle.UpdatedBy,
                identity.Owns(bundle.Namespace),
                rules.GetValueOrDefault(bundle.CanonicalId)?.Visibility ?? Visibility.Inherit,
                AccessService.Effective(rules, bundle.Namespace, bundle.BundleId).Private ? Visibility.Private : Visibility.Public);
        }).ToArray();
    }
}
