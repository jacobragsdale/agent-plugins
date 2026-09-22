using Marketplace.Api.Auth;
using Marketplace.Api.Data;
using Marketplace.Api.Packages;
using Microsoft.EntityFrameworkCore;

namespace Marketplace.Api.Access;

/// <summary>What <c>GET</c> and <c>PUT /api/access/{ns}/{packageId?}</c> exchange. Empty lists mean public.</summary>
public sealed record AccessDocument(string Target, string[] Users, string[] Groups);

/// <summary>
/// Who may see and install what. A namespace or package with no rule is public; a rule is an allowlist of
/// accounts and AD groups; a package rule replaces its namespace rule; owners and admins always see their
/// own namespaces. Anything hidden answers 404 so the desktop app treats it as gone (ADR 0005).
/// </summary>
public sealed class AccessService(MarketplaceDbContext db, TimeProvider timeProvider)
{
    public const int MaxEntries = 200;
    public const int MaxEntryLength = 256;

    private Dictionary<string, AccessRule>? _rules;

    /// <summary>Every rule, keyed by target, loaded once per request.</summary>
    public async Task<IReadOnlyDictionary<string, AccessRule>> RulesAsync(CancellationToken cancellationToken) =>
        _rules ??= await db.AccessRules.AsNoTracking().ToDictionaryAsync(rule => rule.Target, StringComparer.Ordinal, cancellationToken);

    public static bool IsVisible(IReadOnlyDictionary<string, AccessRule> rules, MarketplaceIdentity identity, string ns, string packageId)
    {
        if (identity.Owns(ns))
        {
            return true;
        }

        var rule = rules.GetValueOrDefault($"{ns}/{packageId}") ?? rules.GetValueOrDefault(ns);
        if (rule is null)
        {
            return true;
        }

        var username = IdentityResolver.Username(identity.Account);
        return rule.Users.Any(user => IdentityResolver.AccountMatches(user, identity.Account, username))
            || rule.Groups.Any(identity.InGroup);
    }

    /// <summary>Every (namespace, package) with an approved, non-yanked version: the set each namespace archive was built from.</summary>
    public async Task<List<(string Namespace, string PackageId)>> LivePackagesAsync(string? ns, CancellationToken cancellationToken)
    {
        var rows = await db.PackageVersions.AsNoTracking()
            .Where(version => !version.Yanked && version.ReviewState == ReviewState.Approved && (ns == null || version.Package.Namespace == ns))
            .Select(version => new { version.Package.Namespace, version.Package.PackageId })
            .Distinct()
            .ToListAsync(cancellationToken);
        return rows.Select(row => (row.Namespace, row.PackageId)).ToList();
    }

    /// <summary>The live packages the caller may see.</summary>
    public async Task<List<(string Namespace, string PackageId)>> VisiblePackagesAsync(MarketplaceIdentity identity, string? ns, CancellationToken cancellationToken)
    {
        var live = await LivePackagesAsync(ns, cancellationToken);
        var rules = await RulesAsync(cancellationToken);
        return live.Where(package => IsVisible(rules, identity, package.Namespace, package.PackageId)).ToList();
    }

    public async Task<AccessDocument> GetAsync(MarketplaceIdentity identity, string ns, string? packageId, CancellationToken cancellationToken)
    {
        var target = Target(identity, ns, packageId);
        return Document(target, (await RulesAsync(cancellationToken)).GetValueOrDefault(target));
    }

    /// <summary>Replaces the allowlist for a target. Empty lists make it public again.</summary>
    public async Task<AccessDocument> SetAsync(MarketplaceIdentity identity, string ns, string? packageId, string[]? users, string[]? groups, CancellationToken cancellationToken)
    {
        var target = Target(identity, ns, packageId);
        if (packageId is not null && !await db.Packages.AnyAsync(package => package.Namespace == ns && package.PackageId == packageId, cancellationToken))
        {
            throw new PublishRejectedException(404, $"{target} is not published.");
        }

        var cleanUsers = Clean(users, "users");
        var cleanGroups = Clean(groups, "groups");
        if (cleanUsers.Length + cleanGroups.Length > MaxEntries)
        {
            throw new PublishRejectedException(422, $"An access list holds at most {MaxEntries} entries.");
        }

        var rule = await db.AccessRules.FindAsync([target], cancellationToken);
        if (cleanUsers.Length == 0 && cleanGroups.Length == 0)
        {
            if (rule is not null)
            {
                db.AccessRules.Remove(rule);
                await db.SaveChangesAsync(cancellationToken);
            }

            _rules = null;
            return Document(target, null);
        }

        if (rule is null)
        {
            rule = new AccessRule { Target = target, UpdatedBy = identity.Account };
            db.AccessRules.Add(rule);
        }

        rule.Users = cleanUsers;
        rule.Groups = cleanGroups;
        rule.UpdatedBy = identity.Account;
        rule.UpdatedAt = timeProvider.GetUtcNow().UtcDateTime;
        await db.SaveChangesAsync(cancellationToken);
        _rules = null;
        return Document(target, rule);
    }

    /// <summary>Validates the route and checks ownership; the rule list itself reveals who has access.</summary>
    private static string Target(MarketplaceIdentity identity, string ns, string? packageId)
    {
        if (!IdentityResolver.SourceIdPattern().IsMatch(ns))
        {
            throw new PublishRejectedException(422, $"{ns} is not a valid namespace.");
        }

        if (packageId is not null && !PublishService.PackageIdPattern().IsMatch(packageId))
        {
            throw new PublishRejectedException(422, $"{packageId} is not a valid package id.");
        }

        if (!identity.Owns(ns))
        {
            throw new PublishRejectedException(403, $"{identity.Account} does not own the namespace {ns}.");
        }

        return packageId is null ? ns : $"{ns}/{packageId}";
    }

    private static string[] Clean(string[]? entries, string field)
    {
        var cleaned = (entries ?? [])
            .Select(entry => entry?.Trim() ?? string.Empty)
            .Where(entry => entry.Length > 0)
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .ToArray();
        if (cleaned.Any(entry => entry.Length > MaxEntryLength))
        {
            throw new PublishRejectedException(422, $"Each entry in {field} is at most {MaxEntryLength} characters.");
        }

        return cleaned;
    }

    private static AccessDocument Document(string target, AccessRule? rule) => new(target, rule?.Users ?? [], rule?.Groups ?? []);
}
