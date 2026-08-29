using System.Security.Claims;
using System.Text.Encodings.Web;
using Microsoft.AspNetCore.Authentication;
using Microsoft.Extensions.Options;

namespace Marketplace.Api.Auth;

/// <summary>
/// Trusts <c>X-Dev-User</c> (and optional comma-separated <c>X-Dev-Groups</c>) as the caller.
/// Registered only when the Development environment or <c>Auth:AllowDevHeader</c> enables it.
/// A production server never registers this scheme, so the header is inert there.
/// </summary>
public sealed class DevHeaderAuthenticationHandler(
    IOptionsMonitor<AuthenticationSchemeOptions> options,
    ILoggerFactory logger,
    UrlEncoder encoder) : AuthenticationHandler<AuthenticationSchemeOptions>(options, logger, encoder)
{
    public const string SchemeName = "DevHeader";
    public const string UserHeader = "X-Dev-User";
    public const string GroupsHeader = "X-Dev-Groups";

    protected override Task<AuthenticateResult> HandleAuthenticateAsync()
    {
        if (!Request.Headers.TryGetValue(UserHeader, out var values))
        {
            return Task.FromResult(AuthenticateResult.NoResult());
        }

        var account = values.ToString().Trim();
        if (account.Length is 0 or > 256)
        {
            return Task.FromResult(AuthenticateResult.Fail("X-Dev-User is empty or too long."));
        }

        var claims = new List<Claim> { new(ClaimTypes.Name, account) };
        if (Request.Headers.TryGetValue(GroupsHeader, out var groups))
        {
            claims.AddRange(groups.ToString()
                .Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
                .Select(group => new Claim(ClaimTypes.Role, group)));
        }

        var identity = new ClaimsIdentity(claims, SchemeName, ClaimTypes.Name, ClaimTypes.Role);
        var ticket = new AuthenticationTicket(new ClaimsPrincipal(identity), SchemeName);
        return Task.FromResult(AuthenticateResult.Success(ticket));
    }
}
