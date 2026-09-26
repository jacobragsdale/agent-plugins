using System.Security.Claims;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Microsoft.EntityFrameworkCore;

namespace Marketplace.Api.Auth;

/// <summary>A team the caller belongs to.</summary>
public sealed record TeamMembership(string Namespace, string DisplayName, bool Owner);

/// <summary>The caller as the marketplace sees it, derived from the Windows logon.</summary>
public sealed record MarketplaceIdentity(
    string Account,
    string Namespace,
    string DisplayName,
    bool IsAdmin,
    IReadOnlyList<string> Namespaces,
    IReadOnlyList<string> Groups)
{
    public const string OfficialNamespace = "official";

    /// <summary>Settled from the database by <c>ResolveMarketplaceIdentityAsync</c>; their namespaces are in <see cref="Namespaces"/>.</summary>
    public IReadOnlyList<TeamMembership> Teams { get; init; } = [];

    public bool Owns(string ns) => Namespaces.Contains(ns, StringComparer.Ordinal) || IsAdmin;

    public bool InTeam(string ns) => Teams.Any(team => team.Namespace == ns);

    /// <summary>Owners manage a team's members, invite link, name, and visibility; admins may too.</summary>
    public bool IsTeamOwner(string ns) => IsAdmin || Teams.Any(team => team.Namespace == ns && team.Owner);

    /// <summary>AD group names are case-insensitive, so every group comparison is.</summary>
    public bool InGroup(string group) => Groups.Contains(group, StringComparer.OrdinalIgnoreCase);

    /// <summary>Whether a stored team member entry names the caller; see <see cref="IdentityResolver.EntryMatches"/>.</summary>
    public bool Is(string entry) => IdentityResolver.EntryMatches(entry, Account);
}

public static partial class IdentityResolver
{
    public static MarketplaceIdentity Resolve(ClaimsPrincipal principal, AuthOptions options)
    {
        var account = principal.Identity?.Name?.Trim();
        if (string.IsNullOrEmpty(account))
        {
            throw new InvalidOperationException("The authenticated principal has no name.");
        }

        var username = Username(account);
        var ns = NamespaceFor(username);
        if (ns == MarketplaceIdentity.OfficialNamespace)
        {
            ns = NamespaceFor("u-" + username);
        }

        var groups = principal.FindAll(ClaimTypes.Role)
            .Select(claim => claim.Value.Trim())
            .Where(group => group.Length > 0)
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .ToArray();
        var isAdmin = options.AdminAccounts.Any(candidate => AccountMatches(candidate, account, username))
            || (options.AdminGroup is { Length: > 0 } group && groups.Contains(group, StringComparer.OrdinalIgnoreCase));
        var namespaces = new List<string> { ns };
        if (isAdmin || options.OfficialPublishers.Any(candidate => AccountMatches(candidate, account, username)))
        {
            namespaces.Add(MarketplaceIdentity.OfficialNamespace);
        }

        var displayName = principal.FindFirst(ClaimTypes.GivenName) is { Value.Length: > 0 } given
            ? $"{given.Value} {principal.FindFirst(ClaimTypes.Surname)?.Value}".Trim()
            : username;
        return new MarketplaceIdentity(account, ns, displayName, isAdmin, namespaces, groups);
    }

    /// <summary>
    /// Settles the caller's personal namespace against the publishers table, then adds their teams. The
    /// derived name can be shared: <c>christopher.johnson</c> and <c>christopher.johnston</c> both truncate
    /// to <c>christopher-john</c>. The first account to publish claims it; everyone else gets the first
    /// numbered alternative (<c>christopher-jo-2</c>, …) that they have claimed or that is still free. A
    /// derived name that is a team's becomes <c>u-&lt;name&gt;</c>.
    /// </summary>
    public static async Task<MarketplaceIdentity> SettleAsync(MarketplaceIdentity identity, MarketplaceDbContext db, CancellationToken cancellationToken)
    {
        var derived = identity.Namespace;
        if (await db.Publishers.AnyAsync(publisher => publisher.Namespace == derived && publisher.Kind == PublisherKind.Team, cancellationToken))
        {
            derived = NamespaceFor("u-" + Username(identity.Account));
        }

        var candidates = PersonalCandidates(derived);
        var owners = await db.Publishers.AsNoTracking()
            .Where(publisher => candidates.Contains(publisher.Namespace))
            .ToDictionaryAsync(publisher => publisher.Namespace, publisher => publisher.Account, StringComparer.Ordinal, cancellationToken);
        var ns = candidates.FirstOrDefault(candidate => owners.TryGetValue(candidate, out var owner) && IsClaimant(owner, identity.Account))
            ?? candidates.FirstOrDefault(candidate => !owners.ContainsKey(candidate));

        var account = identity.Account.ToLowerInvariant();
        var username = Username(identity.Account).ToLowerInvariant();
        var teams = await db.TeamMembers.AsNoTracking()
            .Where(member => member.Account.ToLower() == account || member.Account.ToLower() == username)
            .Join(db.Publishers, member => member.Namespace, publisher => publisher.Namespace, (member, publisher) => new { member.Namespace, publisher.DisplayName, member.IsOwner })
            .ToListAsync(cancellationToken);
        var memberships = teams
            .GroupBy(team => team.Namespace, StringComparer.Ordinal)
            .Select(group => new TeamMembership(group.Key, group.First().DisplayName, group.Any(team => team.IsOwner)))
            .OrderBy(team => team.Namespace, StringComparer.Ordinal)
            .ToArray();

        // ponytail: nine accounts deriving one name leaves the tenth without a personal namespace; add Auth:NamespaceOverrides if it happens.
        IEnumerable<string> personal = ns is null ? [] : [ns];
        return identity with
        {
            Namespace = ns ?? identity.Namespace,
            Namespaces = [.. personal, .. identity.Namespaces.Skip(1), .. memberships.Select(team => team.Namespace)],
            Teams = memberships,
        };
    }

    /// <summary>
    /// Whether <paramref name="owner"/>, a publisher row's account, is the caller. Exact, unlike
    /// <see cref="AccountMatches"/>: <c>CORP\jsmith</c> and <c>EUROPE\jsmith</c> are different people.
    /// </summary>
    public static bool IsClaimant(string owner, string account) => string.Equals(owner, account, StringComparison.OrdinalIgnoreCase);

    /// <summary>
    /// The derived namespace, then <c>-2</c> to <c>-9</c> variants that fit 16 characters. A team's row
    /// names the team as its account, so a team namespace is never free for a person.
    /// </summary>
    public static string[] PersonalCandidates(string derived) =>
        Enumerable.Range(2, 8)
            .Select(number => $"{derived[..Math.Min(derived.Length, 14)].TrimEnd('-')}-{number}")
            .Prepend(derived)
            .Where(candidate => candidate != MarketplaceIdentity.OfficialNamespace && SourceIdPattern().IsMatch(candidate))
            .ToArray();

    /// <summary><c>CORP\jacob</c> and <c>jacob@corp.example</c> both yield <c>jacob</c>.</summary>
    public static string Username(string account)
    {
        var backslash = account.LastIndexOf('\\');
        if (backslash >= 0)
        {
            return account[(backslash + 1)..];
        }

        var at = account.IndexOf('@');
        return at > 0 ? account[..at] : account;
    }

    /// <summary>
    /// Maps a sAMAccountName onto the manifest <c>source.id</c> charset: 2–16 lowercase ASCII
    /// letters, digits, or single hyphens, starting with a letter.
    /// </summary>
    public static string NamespaceFor(string username)
    {
        var builder = new StringBuilder(username.Length);
        var pendingHyphen = false;
        foreach (var ch in username.ToLowerInvariant())
        {
            if (ch is >= 'a' and <= 'z' or >= '0' and <= '9')
            {
                if (pendingHyphen && builder.Length > 0)
                {
                    builder.Append('-');
                }

                pendingHyphen = false;
                builder.Append(ch);
            }
            else
            {
                pendingHyphen = true;
            }
        }

        var candidate = builder.ToString();
        if (candidate.Length == 0)
        {
            // Nothing ASCII survives (иван, ___): a short hash keeps the namespace stable and distinct.
            candidate = "u-" + Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(username.ToLowerInvariant())))[..8];
        }
        else if (!char.IsAsciiLetter(candidate[0]))
        {
            candidate = "u-" + candidate;
        }

        if (candidate.Length > 16)
        {
            candidate = candidate[..16].TrimEnd('-');
        }

        if (candidate.Length < 2)
        {
            candidate += "-x";
        }

        return SourceIdPattern().IsMatch(candidate) ? candidate : throw new InvalidOperationException($"Could not derive a namespace from {username}.");
    }

    /// <summary>
    /// A configured or listed account matches the caller by full account name or by username, so
    /// <c>DOMAIN\user</c>, <c>user@domain</c>, and <c>user</c> all name the same person.
    /// </summary>
    public static bool AccountMatches(string candidate, string account, string username) =>
        string.Equals(candidate, account, StringComparison.OrdinalIgnoreCase)
        || string.Equals(Username(candidate.Trim()), username, StringComparison.OrdinalIgnoreCase);

    /// <summary>
    /// Whether a stored team member entry names <paramref name="account"/>. Stricter than
    /// <see cref="AccountMatches"/>, because membership grants publishing: an entry with a domain
    /// (<c>CORP\jane</c>, <c>jane@corp</c>) matches that account only, a bare <c>jane</c> any account
    /// whose username is jane.
    /// </summary>
    public static bool EntryMatches(string entry, string account) =>
        string.Equals(entry, account, StringComparison.OrdinalIgnoreCase)
        || (!entry.Contains('\\') && !entry.Contains('@') && string.Equals(entry, Username(account), StringComparison.OrdinalIgnoreCase));

    /// <summary>official, team, or personal, from the namespace's row; a namespace nobody claimed yet is personal.</summary>
    public static string Lane(string ns, Publisher? publisher) =>
        ns == MarketplaceIdentity.OfficialNamespace ? "official"
        : publisher?.Kind == PublisherKind.Team ? "team"
        : "personal";

    [GeneratedRegex("^[a-z](?:[a-z0-9]|-(?=[a-z0-9])){1,15}$")]
    public static partial Regex SourceIdPattern();
}
