using System.Security.Claims;
using Marketplace.Api.Configuration;
using Microsoft.AspNetCore.Authentication;
using Microsoft.AspNetCore.Authentication.JwtBearer;
using Microsoft.IdentityModel.JsonWebTokens;
using Microsoft.IdentityModel.Tokens;

namespace Marketplace.Api.Auth;

/// <summary>
/// CI pipelines sign in with an app-only OIDC token from an issuer in <c>Auth:Machines</c>: Entra ID through an
/// Azure DevOps service connection, or GitHub Actions directly. The caller becomes a machine account such as
/// <c>app:&lt;oid&gt;</c> or <c>github:owner/repo</c>, which publishes to the teams it is a member of and nothing else.
/// </summary>
public static class MachineAuthentication
{
    /// <summary>Marks a principal a machine token signed in. Its value is the issuer.</summary>
    public const string MachineClaim = "marketplace:machine";

    public static string SchemeName(int index) => $"Machine{index}";

    public static void AddMachines(this AuthenticationBuilder authentication, IReadOnlyList<MachineIssuer> issuers)
    {
        for (var index = 0; index < issuers.Count; index++)
        {
            var issuer = issuers[index];
            authentication.AddJwtBearer(SchemeName(index), options =>
            {
                options.Authority = issuer.Authority;
                options.Audience = issuer.Audience;
                options.MapInboundClaims = false;
                options.Events = new JwtBearerEvents
                {
                    OnTokenValidated = context =>
                    {
                        var principal = context.Principal!;
                        // A delegated token carries scopes: it is a person signed in to some app, not a pipeline.
                        if (principal.HasClaim(claim => claim.Type == "scp"))
                        {
                            context.Fail("This token belongs to a person. Publishing from CI takes an app-only token, such as the one az account get-access-token gives a service connection.");
                            return Task.CompletedTask;
                        }

                        var value = principal.FindFirst(issuer.AccountClaim)?.Value.Trim();
                        if (string.IsNullOrEmpty(value))
                        {
                            context.Fail($"The token has no {issuer.AccountClaim} claim, which is what names the pipeline.");
                            return Task.CompletedTask;
                        }

                        var identity = new ClaimsIdentity(context.Scheme.Name, ClaimTypes.Name, ClaimTypes.Role);
                        identity.AddClaim(new Claim(ClaimTypes.Name, issuer.Prefix + value));
                        identity.AddClaim(new Claim(MachineClaim, issuer.Authority));
                        context.Principal = new ClaimsPrincipal(identity);
                        return Task.CompletedTask;
                    },
                    // A pipeline log is all the person setting it up sees, so say what was wrong with the token.
                    OnChallenge = async context =>
                    {
                        context.HandleResponse();
                        var token = Read(context.HttpContext);
                        var detail = context.AuthenticateFailure switch
                        {
                            SecurityTokenExpiredException => "The token has expired. Get a new one in each run.",
                            SecurityTokenInvalidAudienceException => $"The token was minted for {(token?.Audiences.Any() == true ? string.Join(", ", token.Audiences) : "no audience")}, not {issuer.Audience}. Ask for a token for {issuer.Audience}.",
                            SecurityTokenInvalidIssuerException => $"The token comes from {token?.Issuer ?? "no issuer"}, which this marketplace does not trust. It takes tokens from {string.Join(" and ", issuers.Select(trusted => trusted.Authority))}.",
                            null => "Send a token in the Authorization: Bearer header.",
                            var error => error.Message,
                        };
                        context.Response.StatusCode = StatusCodes.Status401Unauthorized;
                        context.Response.Headers.WWWAuthenticate = "Bearer";
                        await context.HttpContext.RequestServices.GetRequiredService<IProblemDetailsService>().WriteAsync(new ProblemDetailsContext
                        {
                            HttpContext = context.HttpContext,
                            ProblemDetails = new Microsoft.AspNetCore.Mvc.ProblemDetails { Status = 401, Title = "The marketplace did not accept this token.", Detail = detail },
                        });
                    },
                };
            });
        }
    }

    /// <summary>
    /// The machine scheme for a request's bearer token, chosen by the issuer it claims, or null without one. The
    /// scheme then validates the token; one from an unknown issuer goes to the first scheme, which refuses it by name.
    /// </summary>
    public static string? SelectScheme(HttpContext context, IReadOnlyList<MachineIssuer> issuers)
    {
        if (issuers.Count == 0 || !context.Request.Headers.Authorization.ToString().StartsWith("Bearer ", StringComparison.OrdinalIgnoreCase))
        {
            return null;
        }

        var claimed = Read(context)?.Issuer;
        var index = Array.FindIndex(issuers.ToArray(), issuer => string.Equals(issuer.Authority.TrimEnd('/'), claimed?.TrimEnd('/'), StringComparison.OrdinalIgnoreCase));
        return SchemeName(Math.Max(index, 0));
    }

    /// <summary>The request's bearer token, unvalidated, or null when there is none or it is not a JWT.</summary>
    private static JsonWebToken? Read(HttpContext context)
    {
        var header = context.Request.Headers.Authorization.ToString();
        if (!header.StartsWith("Bearer ", StringComparison.OrdinalIgnoreCase))
        {
            return null;
        }

        try
        {
            return new JsonWebToken(header["Bearer ".Length..].Trim());
        }
        catch (ArgumentException)
        {
            return null;
        }
    }
}
