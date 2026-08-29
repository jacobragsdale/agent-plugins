using System.Reflection;
using Marketplace.Api.Auth;
using Marketplace.Api.Catalog;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Marketplace.Api.Events;
using Marketplace.Api.Packages;
using Microsoft.AspNetCore.Http.HttpResults;
using Microsoft.AspNetCore.Mvc;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Options;
using Microsoft.Net.Http.Headers;

namespace Marketplace.Api.Endpoints;

public static class MarketplaceEndpoints
{
    public const string AdminPolicy = "Admin";

    public static IEndpointRouteBuilder MapMarketplace(this IEndpointRouteBuilder app, IReadOnlyList<string> authSchemes)
    {
        var api = app.MapGroup("/api");
        var serverVersion = typeof(MarketplaceEndpoints).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion.Split('+')[0]
            ?? typeof(MarketplaceEndpoints).Assembly.GetName().Version?.ToString(3)
            ?? "0.0.0";

        api.MapGet("/health", (IOptions<ClientOptions> client, IHostEnvironment environment) => Results.Ok(new
        {
            serverVersion,
            minimumClientVersion = client.Value.MinimumVersion,
            latestClientVersion = client.Value.LatestVersion,
            environment = environment.EnvironmentName,
            authSchemes,
        })).AllowAnonymous().WithName("Health");

        var authenticated = api.MapGroup("").RequireAuthorization();

        authenticated.MapGet("/me", (HttpContext context) =>
        {
            var identity = context.MarketplaceIdentity();
            return Results.Ok(new
            {
                account = identity.Account,
                @namespace = identity.Namespace,
                displayName = identity.DisplayName,
                namespaces = identity.Namespaces,
                admin = identity.IsAdmin,
            });
        }).WithName("Me");

        authenticated.MapGet("/catalog", async (HttpContext context, CatalogService catalog, CancellationToken cancellationToken) =>
        {
            var document = await catalog.CatalogAsync(cancellationToken);
            return Conditional(context, document.Bytes, document.Digest, "application/json");
        }).WithName("Catalog");

        authenticated.MapMethods("/sources/{ns}/archive", ["GET", "HEAD"], async (string ns, HttpContext context, MarketplaceDbContext db, CancellationToken cancellationToken) =>
        {
            var archive = await db.NamespaceArchives.AsNoTracking().SingleOrDefaultAsync(candidate => candidate.Namespace == ns, cancellationToken);
            if (archive is null || archive.PackageCount == 0)
            {
                return Results.NotFound();
            }

            context.Response.Headers[HeaderNames.LastModified] = archive.GeneratedAt.ToString("R");
            context.Response.Headers[HeaderNames.ContentDisposition] = $"attachment; filename=\"{ns}-latest.zip\"";
            return Conditional(context, archive.Bytes, archive.Digest, "application/zip");
        }).WithName("SourceArchive");

        authenticated.MapGet("/index", async (CatalogService catalog, CancellationToken cancellationToken) =>
            Results.Ok(await catalog.IndexAsync(cancellationToken))).WithName("Index");

        authenticated.MapGet("/packages/{ns}/{packageId}", async (string ns, string packageId, MarketplaceDbContext db, CancellationToken cancellationToken) =>
        {
            var package = await db.Packages.AsNoTracking().Include(candidate => candidate.Versions)
                .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken);
            if (package is null)
            {
                return Results.NotFound();
            }

            var versions = package.Versions
                .Select(version => (version, semver: SemVer.Parse(version.Version)))
                .OrderByDescending(pair => pair.semver)
                .Select(pair => new
                {
                    version = pair.version.Version,
                    archiveDigest = pair.version.ArchiveDigest,
                    sizeBytes = pair.version.SizeBytes,
                    publishedBy = pair.version.PublishedBy,
                    publishedAt = pair.version.PublishedAt,
                    changelog = pair.version.Changelog,
                    yanked = pair.version.Yanked,
                    componentKinds = pair.version.ComponentKinds,
                })
                .ToArray();
            return Results.Ok(new
            {
                id = package.CanonicalId,
                @namespace = package.Namespace,
                packageId = package.PackageId,
                name = package.Name,
                description = package.Description,
                tags = package.Tags,
                versions,
            });
        }).WithName("Package");

        authenticated.MapPost("/packages/{ns}/{packageId}/versions", async (string ns, string packageId, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            if (!context.Request.HasFormContentType)
            {
                return Results.Problem(statusCode: 415, title: "Publish with multipart/form-data: archive, version, tags, changelog.");
            }

            var form = await context.Request.ReadFormAsync(cancellationToken);
            var file = form.Files.GetFile("archive");
            if (file is null || file.Length == 0)
            {
                return Results.Problem(statusCode: 422, title: "The form has no archive file.");
            }

            if (file.Length > ArchiveInspector.MaxArchiveBytes)
            {
                return Results.Problem(statusCode: 413, title: "The archive is larger than the 50 MB limit.");
            }

            byte[] bytes;
            await using (var stream = file.OpenReadStream())
            using (var buffer = new MemoryStream((int)file.Length))
            {
                await stream.CopyToAsync(buffer, cancellationToken);
                bytes = buffer.ToArray();
            }

            var tags = (form["tags"].ToString() ?? string.Empty).Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
            var request = new PublishRequest(ns, packageId, form["version"].ToString(), tags, form["changelog"].ToString(), bytes);
            try
            {
                var published = await publish.PublishAsync(context.MarketplaceIdentity(), request, cancellationToken);
                return Results.Created($"/api/packages/{ns}/{packageId}", published);
            }
            catch (PublishRejectedException rejected)
            {
                return Rejected(rejected);
            }
        }).WithName("Publish").DisableAntiforgery();

        authenticated.MapPost("/packages/{ns}/{packageId}/versions/{version}/yank", async (string ns, string packageId, string version, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            try
            {
                return await publish.YankAsync(context.MarketplaceIdentity(), ns, packageId, version, cancellationToken)
                    ? Results.NoContent()
                    : Results.NotFound();
            }
            catch (PublishRejectedException rejected)
            {
                return Rejected(rejected);
            }
        }).WithName("Yank");

        authenticated.MapPost("/events", async (EventsBatch batch, HttpContext context, EventsService events, CancellationToken cancellationToken) =>
        {
            if (batch.Events is null)
            {
                return Results.Problem(statusCode: 400, title: "The body has no events array.");
            }

            try
            {
                var accepted = await events.RecordAsync(context.MarketplaceIdentity().Account, batch, cancellationToken);
                return Results.Accepted(value: accepted);
            }
            catch (ArgumentException error)
            {
                return Results.Problem(statusCode: 400, title: error.Message);
            }
        }).WithName("Events");

        authenticated.MapGet("/stats/packages/{ns}/{packageId}", async (string ns, string packageId, MarketplaceDbContext db, EventsService events, CancellationToken cancellationToken) =>
        {
            var exists = await db.Packages.AnyAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken);
            return exists ? Results.Ok(await events.PackageStatsAsync($"{ns}/{packageId}", cancellationToken)) : Results.NotFound();
        }).WithName("PackageStats");

        authenticated.MapPost("/reports", async (ReportRequest report, HttpContext context, MarketplaceDbContext db, TimeProvider time, CancellationToken cancellationToken) =>
        {
            if (string.IsNullOrWhiteSpace(report.PackageId) || string.IsNullOrWhiteSpace(report.Reason) || report.Reason.Length > 2048)
            {
                return Results.Problem(statusCode: 422, title: "A report needs a package id and a reason up to 2048 characters.");
            }

            db.Reports.Add(new PackageReport
            {
                Account = context.MarketplaceIdentity().Account,
                PackageId = report.PackageId.Trim(),
                Reason = report.Reason.Trim(),
                CreatedAt = time.GetUtcNow().UtcDateTime,
            });
            await db.SaveChangesAsync(cancellationToken);
            return Results.Accepted();
        }).WithName("Report");

        var admin = api.MapGroup("/admin").RequireAuthorization(AdminPolicy);
        admin.MapGet("/summary", async (EventsService events, CancellationToken cancellationToken) =>
        {
            var summary = await events.AdminSummaryAsync(cancellationToken);
            return Results.Ok(new
            {
                activeUsers = new { day = summary.ActiveUsers1d, week = summary.ActiveUsers7d, month = summary.ActiveUsers30d },
                publishers = summary.Publishers,
                packages = summary.Packages,
                clientVersions = summary.ClientVersions,
                agentMix = summary.AgentMix,
                topPackages = summary.TopPackages.Select(pair => new { id = pair.Id, installedBase = pair.InstalledBase }),
                preflightFailures = summary.PreflightFailures,
                openReports = summary.OpenReports,
            });
        }).WithName("AdminSummary");

        admin.MapGet("/reports", async (MarketplaceDbContext db, CancellationToken cancellationToken) =>
            Results.Ok(await db.Reports.AsNoTracking().OrderByDescending(report => report.CreatedAt).Take(200).ToListAsync(cancellationToken)))
            .WithName("AdminReports");

        return app;
    }

    public sealed record ReportRequest(string PackageId, string Reason);

    private static IResult Rejected(PublishRejectedException rejected)
    {
        var extensions = new Dictionary<string, object?>();
        if (rejected.Errors.Count > 0)
        {
            extensions["errors"] = rejected.Errors.Select(error => new { path = error.Path, message = error.Message }).ToArray();
        }

        return Results.Problem(statusCode: rejected.Status, title: rejected.Title, extensions: extensions);
    }

    /// <summary>Serves bytes with a strong ETag and honors If-None-Match.</summary>
    private static IResult Conditional(HttpContext context, byte[] bytes, string digest, string contentType)
    {
        var etag = $"\"{digest}\"";
        context.Response.Headers[HeaderNames.ETag] = etag;
        context.Response.Headers[HeaderNames.CacheControl] = "no-cache";
        if (context.Request.Headers.IfNoneMatch.Any(value => value is not null && value.Split(',').Select(tag => tag.Trim()).Contains(etag)))
        {
            return Results.StatusCode(304);
        }

        if (HttpMethods.IsHead(context.Request.Method))
        {
            context.Response.ContentLength = bytes.Length;
            context.Response.ContentType = contentType;
            return Results.Empty;
        }

        return Results.Bytes(bytes, contentType);
    }
}

public static class HttpContextIdentityExtensions
{
    private const string Key = "marketplace.identity";

    public static MarketplaceIdentity MarketplaceIdentity(this HttpContext context)
    {
        if (context.Items.TryGetValue(Key, out var cached) && cached is MarketplaceIdentity identity)
        {
            return identity;
        }

        var options = context.RequestServices.GetRequiredService<IOptions<AuthOptions>>().Value;
        identity = IdentityResolver.Resolve(context.User, options);
        context.Items[Key] = identity;
        return identity;
    }
}
