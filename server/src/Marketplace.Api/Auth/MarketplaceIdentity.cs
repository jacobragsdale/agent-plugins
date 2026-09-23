using System.Security.Claims;
using System.Text;
using System.Text.RegularExpressions;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Microsoft.EntityFrameworkCore;

namespace Marketplace.Api.Auth;

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

    public bool Owns(string ns) => Namespaces.Contains(ns, StringComparer.Ordinal) || IsAdmin;

    /// <summary>AD group names are case-insensitive, so every group comparison is.</summary>
    public bool InGroup(string group) => Groups.Contains(group, StringComparer.OrdinalIgnoreCase);
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
        if (IsReserved(options, ns))
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

        namespaces.AddRange(options.TeamNamespaces
            .Where(team => groups.Contains(team.Group, StringComparer.OrdinalIgnoreCase))
            .Select(team => team.Namespace)
            .Where(candidate => !namespaces.Contains(candidate, StringComparer.Ordinal)));

        var displayName = principal.FindFirst(ClaimTypes.GivenName) is { Value.Length: > 0 } given
            ? $"{given.Value} {principal.FindFirst(ClaimTypes.Surname)?.Value}".Trim()
            : username;
        return new MarketplaceIdentity(account, ns, displayName, isAdmin, namespaces, groups);
    }

    /// <summary>
    /// Settles the caller's personal namespace against the publishers table. The derived name can be
    /// shared: <c>christopher.johnson</c> and <c>christopher.johnston</c> both truncate to
    /// <c>christopher-john</c>. The first account to publish claims it; everyone else gets the first
    /// numbered alternative (<c>christopher-jo-2</c>, …) that they have claimed or that is still free.
    /// </summary>
    public static async Task<MarketplaceIdentity> ClaimPersonalNamespaceAsync(MarketplaceIdentity identity, MarketplaceDbContext db, AuthOptions options, CancellationToken cancellationToken)
    {
        var candidates = PersonalCandidates(options, identity.Namespace);
        var owners = await db.Publishers.AsNoTracking()
            .Where(publisher => candidates.Contains(publisher.Namespace))
            .ToDictionaryAsync(publisher => publisher.Namespace, publisher => publisher.Account, StringComparer.Ordinal, cancellationToken);
        var ns = candidates.FirstOrDefault(candidate => owners.TryGetValue(candidate, out var owner) && IsClaimant(owner, identity.Account))
            ?? candidates.FirstOrDefault(candidate => !owners.ContainsKey(candidate));

        // ponytail: nine accounts deriving one name leaves the tenth without a personal namespace; add Auth:NamespaceOverrides if it happens.
        return ns is null ? identity with { Namespaces = [.. identity.Namespaces.Skip(1)] }
            : ns == identity.Namespace ? identity
            : identity with { Namespace = ns, Namespaces = [ns, .. identity.Namespaces.Skip(1)] };
    }

    /// <summary>
    /// Whether <paramref name="owner"/>, a publisher row's account, is the caller. Exact, unlike
    /// <see cref="AccountMatches"/>: <c>CORP\jsmith</c> and <c>EUROPE\jsmith</c> are different people.
    /// </summary>
    public static bool IsClaimant(string owner, string account) => string.Equals(owner, account, StringComparison.OrdinalIgnoreCase);

    /// <summary>The derived namespace, then <c>-2</c> to <c>-9</c> variants that fit 16 characters, skipping reserved names.</summary>
    public static string[] PersonalCandidates(AuthOptions options, string derived) =>
        Enumerable.Range(2, 8)
            .Select(number => $"{derived[..Math.Min(derived.Length, 14)].TrimEnd('-')}-{number}")
            .Prepend(derived)
            .Where(candidate => !IsReserved(options, candidate) && SourceIdPattern().IsMatch(candidate))
            .ToArray();

    /// <summary><c>official</c> and every team namespace belong to their lane, never to one person.</summary>
    public static bool IsReserved(AuthOptions options, string ns) =>
        ns == MarketplaceIdentity.OfficialNamespace || options.TeamNamespaces.Any(team => team.Namespace == ns);

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
        if (candidate.Length == 0 || !char.IsAsciiLetter(candidate[0]))
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

    /// <summary>official, team (a configured AD-group namespace), or personal.</summary>
    public static string Lane(AuthOptions options, string ns) =>
        ns == MarketplaceIdentity.OfficialNamespace ? "official"
        : options.TeamNamespaces.Any(team => team.Namespace == ns) ? "team"
        : "personal";

    /// <summary>How a namespace is shown: "Official", the team's configured name, or the caller's own name.</summary>
    public static string NamespaceDisplayName(AuthOptions options, MarketplaceIdentity identity, string ns) =>
        ns == MarketplaceIdentity.OfficialNamespace ? "Official"
        : options.TeamNamespaces.FirstOrDefault(team => team.Namespace == ns)?.DisplayName is { Length: > 0 } teamName ? teamName
        : identity.DisplayName;

    [GeneratedRegex("^[a-z](?:[a-z0-9]|-(?=[a-z0-9])){1,15}$")]
    public static partial Regex SourceIdPattern();
}
