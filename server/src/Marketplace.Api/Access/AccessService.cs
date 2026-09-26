using System.Buffers.Text;
using System.Security.Cryptography;
using Marketplace.Api.Auth;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Marketplace.Api.Packages;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Options;

namespace Marketplace.Api.Access;

public sealed record PersonView(string Account, string DisplayName);

public sealed record TeamRef(string Namespace, string DisplayName);

/// <summary>What <c>GET</c> and <c>PUT /api/access/{ns}/{id?}</c> exchange.</summary>
public sealed record ShareView(string Target, Visibility Visibility, Visibility Effective, PersonView[] Users, TeamRef[] Teams, string[] Groups, string? Link);

/// <summary>A <c>PUT</c> body. Without <see cref="Visibility"/>, any entry means private (the pre-sharing API).</summary>
public sealed record ShareRequest(Visibility? Visibility, string[]? Users, string[]? Teams, string[]? Groups);

/// <summary>A package with a live version. <see cref="Gated"/>: the public may not see it until an admin approves its MCP server.</summary>
public sealed record LivePackage(string Namespace, string PackageId, bool Gated);

/// <summary>
/// Who may see and install what. A namespace is public unless its rule says private; a package or bundle
/// follows its namespace unless its own rule says public or private. Private means the namespace's owners
/// plus the share list: accounts, teams, and AD groups. A package that follows a private namespace is
/// also shared with its own list. Anything hidden answers 404 so the desktop app treats it as gone.
/// </summary>
public sealed class AccessService(MarketplaceDbContext db, IOptions<ServerOptions> server, TimeProvider timeProvider)
{
    public const int MaxEntries = 200;
    public const int MaxEntryLength = 256;

    private Dictionary<string, AccessRule>? _rules;

    /// <summary>Every rule, keyed by target, loaded once per request.</summary>
    public async Task<IReadOnlyDictionary<string, AccessRule>> RulesAsync(CancellationToken cancellationToken) =>
        _rules ??= await db.AccessRules.AsNoTracking().ToDictionaryAsync(rule => rule.Target, StringComparer.Ordinal, cancellationToken);

    /// <summary>Whether <c>ns</c> or <c>ns/id</c> is private, and the rules whose lists share it.</summary>
    public static (bool Private, AccessRule?[] Lists) Effective(IReadOnlyDictionary<string, AccessRule> rules, string ns, string? id)
    {
        var space = rules.GetValueOrDefault(ns);
        if (id is null)
        {
            return (space?.Visibility == Visibility.Private, [space]);
        }

        var own = rules.GetValueOrDefault($"{ns}/{id}");
        return own is { Visibility: Visibility.Public or Visibility.Private }
            ? (own.Visibility == Visibility.Private, [own])
            : (space?.Visibility == Visibility.Private, [space, own]);
    }

    public static bool IsVisible(IReadOnlyDictionary<string, AccessRule> rules, MarketplaceIdentity identity, string ns, string? id, bool gated = false)
    {
        if (identity.Owns(ns))
        {
            return true;
        }

        var (isPrivate, lists) = Effective(rules, ns, id);
        return isPrivate ? lists.Any(rule => Listed(rule, identity)) : !gated;
    }

    /// <summary>Visible only because a share list names the caller: they do not own it and it is private.</summary>
    public static bool SharedWith(IReadOnlyDictionary<string, AccessRule> rules, MarketplaceIdentity identity, string ns, string? id) =>
        !identity.Owns(ns) && Effective(rules, ns, id) is (true, var lists) && lists.Any(rule => Listed(rule, identity));

    private static bool Listed(AccessRule? rule, MarketplaceIdentity identity)
    {
        if (rule is null)
        {
            return false;
        }

        var username = IdentityResolver.Username(identity.Account);
        return rule.Users.Any(user => IdentityResolver.AccountMatches(user, identity.Account, username))
            || rule.Teams.Any(identity.InTeam)
            || rule.Groups.Any(identity.InGroup);
    }

    /// <summary>Every package with a live version, the set each namespace archive was built from.</summary>
    public async Task<List<LivePackage>> LivePackagesAsync(string? ns, CancellationToken cancellationToken)
    {
        var rows = await db.Packages.AsNoTracking()
            .Where(package => package.RevokedAt == null && (ns == null || package.Namespace == ns))
            .Select(package => new
            {
                package.Namespace,
                package.PackageId,
                Approved = package.McpApprovedBy != null,
                Versions = package.Versions.Where(version => !version.Yanked).Select(version => new { version.Version, version.ComponentKinds }).ToList(),
            })
            .ToListAsync(cancellationToken);
        return rows
            .Where(row => row.Versions.Count > 0)
            .Select(row => new LivePackage(
                row.Namespace,
                row.PackageId,
                !row.Approved && row.Versions.MaxBy(version => SemVer.Parse(version.Version))!.ComponentKinds.Contains(PublishService.McpServerKind)))
            .ToList();
    }

    /// <summary>The live packages the caller may see.</summary>
    public async Task<List<LivePackage>> VisiblePackagesAsync(MarketplaceIdentity identity, string? ns, CancellationToken cancellationToken)
    {
        var live = await LivePackagesAsync(ns, cancellationToken);
        var rules = await RulesAsync(cancellationToken);
        return live.Where(package => IsVisible(rules, identity, package.Namespace, package.PackageId, package.Gated)).ToList();
    }

    /// <summary>The package if the caller may see it: owners always; others when it is live and its rule allows them.</summary>
    public async Task<Package> VisiblePackageAsync(MarketplaceIdentity identity, string ns, string packageId, CancellationToken cancellationToken)
    {
        var package = await db.Packages.AsNoTracking().Include(candidate => candidate.Versions)
            .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken);
        return package is not null
            && (identity.Owns(ns)
                || (PublishService.LatestVersion(package) is not null && IsVisible(await RulesAsync(cancellationToken), identity, ns, packageId, PublishService.Gated(package))))
            ? package
            : throw ProblemException.NotFound($"The package {ns}/{packageId}");
    }

    public async Task<ShareView> GetAsync(MarketplaceIdentity identity, string ns, string? id, CancellationToken cancellationToken)
    {
        var target = Target(identity, ns, id);
        var rules = await RulesAsync(cancellationToken);
        var rule = rules.GetValueOrDefault(target);
        var link = await db.Links.AsNoTracking()
            .Where(candidate => candidate.Kind == Link.Share && candidate.Target == target)
            .Select(candidate => candidate.Code)
            .SingleOrDefaultAsync(cancellationToken);
        var people = await DisplayNamesAsync(rule?.Users ?? [], cancellationToken);
        var teamIds = rule?.Teams ?? [];
        var teams = await db.Publishers.AsNoTracking()
            .Where(publisher => teamIds.Contains(publisher.Namespace))
            .ToDictionaryAsync(publisher => publisher.Namespace, publisher => publisher.DisplayName, StringComparer.Ordinal, cancellationToken);
        return new ShareView(
            target,
            rule?.Visibility ?? (id is null ? Visibility.Public : Visibility.Inherit),
            Effective(rules, ns, id).Private ? Visibility.Private : Visibility.Public,
            (rule?.Users ?? []).Select(user => new PersonView(user, people.GetValueOrDefault(user) ?? IdentityResolver.Username(user))).ToArray(),
            (rule?.Teams ?? []).Select(team => new TeamRef(team, teams.GetValueOrDefault(team) ?? team)).ToArray(),
            rule?.Groups ?? [],
            link is null ? null : LinkUrl(link));
    }

    /// <summary>Replaces a target's visibility and share lists.</summary>
    public async Task<ShareView> SetAsync(MarketplaceIdentity identity, string ns, string? id, ShareRequest request, CancellationToken cancellationToken)
    {
        var target = Target(identity, ns, id);
        var publisher = await db.Publishers.AsNoTracking().SingleOrDefaultAsync(candidate => candidate.Namespace == ns, cancellationToken);
        if (id is null && publisher?.Kind == PublisherKind.Team && !identity.IsTeamOwner(ns))
        {
            throw new ProblemException(403, $"Only the team's owners can change who sees {publisher.DisplayName}.");
        }

        // Accounts that derive the same name all own it until one publishes; only the claimant may set its list.
        if (ns == identity.Namespace && (publisher is null || !IdentityResolver.IsClaimant(publisher.Account, identity.Account)))
        {
            throw new ProblemException(409, $"Publish to {ns} before you set who may see it.");
        }

        if (id is not null && !await ExistsAsync(ns, id, cancellationToken))
        {
            throw ProblemException.NotFound(target);
        }

        var users = Clean(request.Users, "users");
        var teams = Clean(request.Teams, "teams");
        var groups = Clean(request.Groups, "groups");
        if (users.Length + teams.Length + groups.Length > MaxEntries)
        {
            throw new ProblemException(422, $"A share list holds at most {MaxEntries} entries.");
        }

        var known = await db.Publishers.AsNoTracking()
            .Where(candidate => teams.Contains(candidate.Namespace) && candidate.Kind == PublisherKind.Team)
            .Select(candidate => candidate.Namespace)
            .ToListAsync(cancellationToken);
        if (teams.Except(known, StringComparer.Ordinal).ToArray() is { Length: > 0 } unknown)
        {
            throw new ProblemException(422, $"There is no team called {string.Join(", ", unknown)}.");
        }

        var empty = users.Length + teams.Length + groups.Length == 0;
        var visibility = request.Visibility ?? (!empty ? Visibility.Private : id is null ? Visibility.Public : Visibility.Inherit);
        if (id is null && visibility == Visibility.Inherit)
        {
            throw new ProblemException(422, "A namespace is public or private.");
        }

        var rule = await db.AccessRules.FindAsync([target], cancellationToken);
        if (empty && visibility == (id is null ? Visibility.Public : Visibility.Inherit))
        {
            if (rule is not null)
            {
                db.AccessRules.Remove(rule);
            }
        }
        else
        {
            if (rule is null)
            {
                rule = new AccessRule { Target = target, UpdatedBy = identity.Account };
                db.AccessRules.Add(rule);
            }

            rule.Visibility = visibility;
            rule.Users = users;
            rule.Teams = teams;
            rule.Groups = groups;
            rule.UpdatedBy = identity.Account;
            rule.UpdatedAt = timeProvider.GetUtcNow().UtcDateTime;
        }

        await db.SaveChangesAsync(cancellationToken);
        _rules = null;
        return await GetAsync(identity, ns, id, cancellationToken);
    }

    /// <summary>The target's share link, created on first request; <paramref name="reset"/> replaces it.</summary>
    public async Task<string> ShareLinkAsync(MarketplaceIdentity identity, string ns, string? id, bool reset, CancellationToken cancellationToken)
    {
        var target = Target(identity, ns, id);
        if (id is not null && !await ExistsAsync(ns, id, cancellationToken))
        {
            throw ProblemException.NotFound(target);
        }

        return await LinkAsync(Link.Share, target, identity.Account, reset, cancellationToken);
    }

    /// <summary>The link of <paramref name="kind"/> for a target, created on first request; <paramref name="reset"/> replaces it.</summary>
    public async Task<string> LinkAsync(string kind, string target, string account, bool reset, CancellationToken cancellationToken)
    {
        var existing = await db.Links.SingleOrDefaultAsync(link => link.Kind == kind && link.Target == target, cancellationToken);
        if (existing is not null && !reset)
        {
            return LinkUrl(existing.Code);
        }

        if (existing is not null)
        {
            db.Links.Remove(existing);
            await db.SaveChangesAsync(cancellationToken);
        }

        var code = Base64Url.EncodeToString(RandomNumberGenerator.GetBytes(16));
        db.Links.Add(new Link { Code = code, Kind = kind, Target = target, CreatedBy = account, CreatedAt = timeProvider.GetUtcNow().UtcDateTime });
        await db.SaveChangesAsync(cancellationToken);
        return LinkUrl(code);
    }

    /// <summary>Adds the caller to a target's share list, unless they can already see it. Returns whether it changed.</summary>
    public async Task<bool> AddToShareListAsync(MarketplaceIdentity identity, string target, CancellationToken cancellationToken)
    {
        var (ns, id) = Split(target);
        if (IsVisible(await RulesAsync(cancellationToken), identity, ns, id))
        {
            return false;
        }

        var rule = await db.AccessRules.FindAsync([target], cancellationToken);
        if (rule is null)
        {
            rule = new AccessRule { Target = target, Visibility = id is null ? Visibility.Private : Visibility.Inherit, UpdatedBy = identity.Account };
            db.AccessRules.Add(rule);
        }

        rule.Users = [.. rule.Users, identity.Account];
        rule.UpdatedAt = timeProvider.GetUtcNow().UtcDateTime;
        await db.SaveChangesAsync(cancellationToken);
        _rules = null;
        return true;
    }

    public string LinkUrl(string code) => $"{server.Value.PublicBaseUrl.TrimEnd('/')}/l/{code}";

    /// <summary>Display names for accounts, from everyone who has signed in.</summary>
    public async Task<Dictionary<string, string>> DisplayNamesAsync(IReadOnlyCollection<string> accounts, CancellationToken cancellationToken)
    {
        var names = await db.People.AsNoTracking()
            .Where(person => accounts.Contains(person.Account))
            .ToListAsync(cancellationToken);
        return names.ToDictionary(person => person.Account, person => person.DisplayName, StringComparer.OrdinalIgnoreCase);
    }

    public static (string Namespace, string? Id) Split(string target) =>
        target.IndexOf('/') is var slash and >= 0 ? (target[..slash], target[(slash + 1)..]) : (target, null);

    /// <summary>A package or a bundle; the two share one id space per namespace.</summary>
    private async Task<bool> ExistsAsync(string ns, string id, CancellationToken cancellationToken) =>
        await db.Packages.AnyAsync(package => package.Namespace == ns && package.PackageId == id, cancellationToken)
        || await db.Bundles.AnyAsync(bundle => bundle.Namespace == ns && bundle.BundleId == id, cancellationToken);

    /// <summary>Validates the route and checks ownership; the lists themselves reveal who has access.</summary>
    private static string Target(MarketplaceIdentity identity, string ns, string? id)
    {
        if (!IdentityResolver.SourceIdPattern().IsMatch(ns))
        {
            throw new ProblemException(422, $"{ns} is not a valid namespace.");
        }

        if (id is not null && !PublishService.PackageIdPattern().IsMatch(id))
        {
            throw new ProblemException(422, $"{id} is not a valid package or bundle id.");
        }

        if (!identity.Owns(ns))
        {
            throw ProblemException.NotOwner(identity.Account, ns);
        }

        return id is null ? ns : $"{ns}/{id}";
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
            throw new ProblemException(422, $"Each entry in {field} is at most {MaxEntryLength} characters.");
        }

        return cleaned;
    }
}
