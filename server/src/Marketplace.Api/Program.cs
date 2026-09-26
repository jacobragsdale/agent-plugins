using System.Text.Json;
using System.Text.Json.Serialization;
using Marketplace.Api;
using Marketplace.Api.Access;
using Marketplace.Api.Auth;
using Marketplace.Api.Catalog;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Marketplace.Api.Endpoints;
using Marketplace.Api.Events;
using Marketplace.Api.Packages;
using Marketplace.Api.Storage;
using Marketplace.Api.Teams;
using Microsoft.AspNetCore.Authentication;
using Microsoft.AspNetCore.Authentication.Negotiate;
using Microsoft.AspNetCore.Http.Features;
using Microsoft.AspNetCore.HttpOverrides;
using Microsoft.EntityFrameworkCore;

var builder = WebApplication.CreateBuilder(args);

builder.Services.Configure<ServerOptions>(builder.Configuration.GetSection(ServerOptions.Section));
builder.Services.Configure<ArtifactKeeperOptions>(builder.Configuration.GetSection(ArtifactKeeperOptions.Section));
builder.Services.Configure<AuthOptions>(builder.Configuration.GetSection(AuthOptions.Section));
builder.Services.Configure<ClientOptions>(builder.Configuration.GetSection(ClientOptions.Section));
builder.Services.Configure<ValidatorOptions>(builder.Configuration.GetSection(ValidatorOptions.Section));
builder.Services.Configure<DatabaseOptions>(builder.Configuration.GetSection(DatabaseOptions.Section));
builder.Services.Configure<FormOptions>(options =>
{
    // At least the publish endpoint's request limit, so an oversized upload gets its 413 rather than a form-reading 400.
    options.MultipartBodyLengthLimit = MarketplaceEndpoints.MaxUploadRequestBytes;
    // A browser upload sends a file and a path value per file, plus a few fields.
    options.ValueCountLimit = 2 * PublishService.MaxUploadFiles + 16;
});
builder.Services.Configure<ForwardedHeadersOptions>(options =>
{
    options.ForwardedHeaders = ForwardedHeaders.XForwardedFor | ForwardedHeaders.XForwardedProto;
    options.KnownIPNetworks.Clear();
    options.KnownProxies.Clear();
});

var authOptions = builder.Configuration.GetSection(AuthOptions.Section).Get<AuthOptions>() ?? new AuthOptions();
var devHeader = builder.Environment.IsDevelopment() || authOptions.AllowDevHeader;
var schemes = new List<string>();
if (authOptions.EnableNegotiate)
{
    schemes.Add(NegotiateDefaults.AuthenticationScheme);
}

if (devHeader)
{
    schemes.Add(DevHeaderAuthenticationHandler.SchemeName);
}

if (schemes.Count == 0)
{
    throw new InvalidOperationException("No authentication scheme is enabled: set Auth:EnableNegotiate or run in Development.");
}

const string forwardingScheme = "Marketplace";
var authentication = builder.Services.AddAuthentication(options =>
{
    options.DefaultScheme = forwardingScheme;
    options.DefaultChallengeScheme = forwardingScheme;
});
authentication.AddPolicyScheme(forwardingScheme, forwardingScheme, options =>
{
    options.ForwardDefaultSelector = context =>
        devHeader && (!authOptions.EnableNegotiate || context.Request.Headers.ContainsKey(DevHeaderAuthenticationHandler.UserHeader))
            ? DevHeaderAuthenticationHandler.SchemeName
            : NegotiateDefaults.AuthenticationScheme;
});
if (authOptions.EnableNegotiate)
{
    authentication.AddNegotiate(options =>
    {
        if (authOptions.LdapDomain is { Length: > 0 } domain)
        {
            options.EnableLdap(settings => settings.Domain = domain);
        }
    });
}

if (devHeader)
{
    authentication.AddScheme<AuthenticationSchemeOptions, DevHeaderAuthenticationHandler>(DevHeaderAuthenticationHandler.SchemeName, null);
}

builder.Services.AddAuthorization(options =>
{
    options.AddPolicy(MarketplaceEndpoints.AdminPolicy, policy => policy
        .RequireAuthenticatedUser()
        .RequireAssertion(context => IdentityResolver.Resolve(context.User, authOptions).IsAdmin));
});

builder.Services.AddDbContext<MarketplaceDbContext>(options =>
    options.UseNpgsql(builder.Configuration.GetConnectionString("Marketplace")));
builder.Services.AddSingleton(TimeProvider.System);
builder.Services.AddHttpClient<ArtifactKeeperStore>((services, client) =>
{
    var options = builder.Configuration.GetSection(ArtifactKeeperOptions.Section).Get<ArtifactKeeperOptions>() ?? new ArtifactKeeperOptions();
    client.BaseAddress = new Uri(options.BaseUrl.TrimEnd('/') + "/");
    client.Timeout = TimeSpan.FromSeconds(120);
}).ConfigurePrimaryHttpMessageHandler(() => new SocketsHttpHandler
{
    // The store is a singleton holding one client, so connections must re-resolve DNS when Artifact Keeper moves.
    PooledConnectionLifetime = TimeSpan.FromMinutes(2),
});
builder.Services.AddSingleton<IArtifactStore>(services => services.GetRequiredService<ArtifactKeeperStore>());
builder.Services.AddSingleton<IPackageValidator, ProcessPackageValidator>();
builder.Services.AddScoped<PublishService>();
builder.Services.AddScoped<CatalogService>();
builder.Services.AddScoped<AccessService>();
builder.Services.AddScoped<EventsService>();
builder.Services.AddScoped<TeamService>();
builder.Services.AddScoped<SuggestionService>();
builder.Services.AddScoped<BundleService>();
builder.Services.AddProblemDetails();
builder.Services.AddExceptionHandler<ProblemExceptionHandler>();
// Malformed bodies and route values throw so ProblemExceptionHandler can say what was wrong, in every environment.
builder.Services.Configure<RouteHandlerOptions>(options => options.ThrowOnBadRequest = true);
builder.Services.ConfigureHttpJsonOptions(options => options.SerializerOptions.Converters.Add(new JsonStringEnumConverter(JsonNamingPolicy.CamelCase)));
builder.Services.AddOpenApi();

var app = builder.Build();

if (devHeader && authOptions.EnableNegotiate && !app.Environment.IsDevelopment())
{
    app.Logger.LogWarning("Auth:AllowDevHeader is on beside Negotiate outside Development: anyone who can reach this server can claim any account with X-Dev-User. Use this only on a network without a domain.");
}

app.UseForwardedHeaders();
app.UseExceptionHandler();
app.UseStatusCodePages();
app.UsePortalFiles(app.Configuration.GetSection(ServerOptions.Section).Get<ServerOptions>() ?? new ServerOptions());
app.UseAuthentication();
app.UseAuthorization();
app.MapOpenApi().AllowAnonymous();
app.MapMarketplace(schemes);
app.MapPortalFallback();

var databaseOptions = app.Configuration.GetSection(DatabaseOptions.Section).Get<DatabaseOptions>() ?? new DatabaseOptions();
if (databaseOptions.MigrateOnStartup)
{
    using var scope = app.Services.CreateScope();
    var db = scope.ServiceProvider.GetRequiredService<MarketplaceDbContext>();
    var pending = await db.Database.GetPendingMigrationsAsync();
    await db.Database.MigrateAsync();

    // SelfService made pending versions live and rejected ones withdrawn, so the stored archives are stale.
    if (pending.Any(migration => migration.EndsWith("_SelfService", StringComparison.Ordinal)))
    {
        await scope.ServiceProvider.GetRequiredService<PublishService>().RegenerateAllAsync(CancellationToken.None);
    }
}

app.Run();

/// <summary>Exposes the entry point to the integration tests.</summary>
public partial class Program;
