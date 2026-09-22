using System.Security.Claims;
using System.Text;
using System.Text.RegularExpressions;
using Marketplace.Api.Configuration;

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
