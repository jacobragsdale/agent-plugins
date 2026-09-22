using System.Reflection;
using System.Text;
using Marketplace.Api.Access;
using Marketplace.Api.Auth;
using Marketplace.Api.Catalog;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Marketplace.Api.Events;
using Marketplace.Api.Packages;
using Marketplace.Api.Storage;
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
                groups = identity.Groups,
            });
        }).WithName("Me");

        authenticated.MapGet("/catalog", async (HttpContext context, CatalogService catalog, CancellationToken cancellationToken) =>
        {
            var document = await catalog.CatalogAsync(context.MarketplaceIdentity(), cancellationToken);
            return Conditional(context, document.Bytes, document.Digest, "application/json");
        }).WithName("Catalog");

        authenticated.MapMethods("/sources/{ns}/archive", ["GET", "HEAD"], async (string ns, HttpContext context, MarketplaceDbContext db, AccessService access, CancellationToken cancellationToken) =>
        {
            var archive = await db.NamespaceArchives.AsNoTracking().SingleOrDefaultAsync(candidate => candidate.Namespace == ns, cancellationToken);
            if (archive is null || archive.PackageCount == 0)
            {
                return Results.NotFound();
            }

            // The stored archive holds exactly the live packages; a caller who may see them all gets it unchanged.
            var identity = context.MarketplaceIdentity();
            var rules = await access.RulesAsync(cancellationToken);
            var live = await access.LivePackagesAsync(ns, cancellationToken);
            var keep = live
                .Where(package => AccessService.IsVisible(rules, identity, ns, package.PackageId))
                .Select(package => package.PackageId)
                .ToHashSet(StringComparer.Ordinal);
            if (keep.Count == 0)
            {
                return Results.NotFound();
            }

            context.Response.Headers[HeaderNames.LastModified] = archive.GeneratedAt.ToString("R");
            context.Response.Headers[HeaderNames.ContentDisposition] = $"attachment; filename=\"{ns}-latest.zip\"";
            if (keep.Count == live.Count)
            {
                return Conditional(context, archive.Bytes, archive.Digest, "application/zip");
            }

            var variant = NamespaceArchiveBuilder.Filter(archive.Bytes, keep);
            return Conditional(context, variant.Bytes, variant.Digest, "application/zip");
        }).WithName("SourceArchive");

        authenticated.MapGet("/index", async (HttpContext context, CatalogService catalog, CancellationToken cancellationToken) =>
            Results.Ok(await catalog.IndexAsync(context.MarketplaceIdentity(), cancellationToken))).WithName("Index");

        authenticated.MapGet("/packages/{ns}/{packageId}", async (string ns, string packageId, HttpContext context, MarketplaceDbContext db, AccessService access, CancellationToken cancellationToken) =>
        {
            var package = await VisiblePackageAsync(context, db, access, ns, packageId, cancellationToken);
            return package is null ? Results.NotFound() : Results.Ok(PackageView.From(package, context.MarketplaceIdentity().Owns(ns)));
        }).WithName("Package");

        authenticated.MapGet("/packages/{ns}/{packageId}/versions/{version}/files", async (string ns, string packageId, string version, HttpContext context, MarketplaceDbContext db, AccessService access, IArtifactStore store, CancellationToken cancellationToken) =>
        {
            var archive = await ReadableArchiveAsync(context, db, access, store, ns, packageId, version, cancellationToken);
            if (archive is null)
            {
                return Results.NotFound();
            }

            using var zip = ArchiveInspector.OpenZip(new MemoryStream(archive, writable: false));
            var prefix = ArchiveInspector.RootPrefix(zip.Entries.Select(entry => entry.FullName).ToArray());
            var files = zip.Entries
                .Where(entry => entry.FullName.StartsWith(prefix, StringComparison.Ordinal) && !entry.FullName.EndsWith('/'))
                .Select(entry => new PackageFile(entry.FullName[prefix.Length..], entry.Length))
                .OrderBy(file => file.Path, StringComparer.Ordinal)
                .ToArray();
            return Results.Ok(files);
        }).WithName("PackageFiles");

        authenticated.MapGet("/packages/{ns}/{packageId}/versions/{version}/files/{**path}", async (string ns, string packageId, string version, string path, HttpContext context, MarketplaceDbContext db, AccessService access, IArtifactStore store, CancellationToken cancellationToken) =>
        {
            var archive = await ReadableArchiveAsync(context, db, access, store, ns, packageId, version, cancellationToken);
            if (archive is null)
            {
                return Results.NotFound();
            }

            using var zip = ArchiveInspector.OpenZip(new MemoryStream(archive, writable: false));
            var prefix = ArchiveInspector.RootPrefix(zip.Entries.Select(entry => entry.FullName).ToArray());
            var entry = zip.GetEntry(prefix + path);
            if (entry is null || entry.FullName.EndsWith('/'))
            {
                return Results.NotFound();
            }

            if (entry.Length > MaxFileView)
            {
                return Results.Problem(statusCode: 413, title: $"{path} is larger than {MaxFileView / 1024 / 1024} MB; download the version to read it.");
            }

            var bytes = new byte[entry.Length];
            await using (var stream = entry.Open())
            {
                await stream.ReadExactlyAsync(bytes, cancellationToken);
            }

            return IsText(bytes)
                ? Results.Bytes(bytes, "text/plain; charset=utf-8")
                : Results.File(bytes, "application/octet-stream", Path.GetFileName(path));
        }).WithName("PackageFile");

        authenticated.MapPost("/packages/{ns}/{packageId}/versions", async (string ns, string packageId, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            if (!context.Request.HasFormContentType)
            {
                return Results.Problem(statusCode: 415, title: "Publish with multipart/form-data: archive or files with paths, version, tags, changelog.");
            }

            var form = await context.Request.ReadFormAsync(cancellationToken);
            var file = form.Files.GetFile("archive");
            var files = form.Files.GetFiles("files");
            var paths = form["paths"];
            if ((file is null || file.Length == 0) && files.Count == 0)
            {
                return Results.Problem(statusCode: 422, title: "The form has no archive and no files.");
            }

            if (files.Count != paths.Count)
            {
                return Results.Problem(statusCode: 422, title: "Send one paths value for each file.");
            }

            if ((file?.Length ?? 0) + files.Sum(upload => upload.Length) > ArchiveInspector.MaxArchiveBytes)
            {
                return Results.Problem(statusCode: 413, title: "The upload is larger than the 50 MB limit.");
            }

            byte[]? bytes = null;
            if (file is { Length: > 0 })
            {
                await using var stream = file.OpenReadStream();
                using var buffer = new MemoryStream((int)file.Length);
                await stream.CopyToAsync(buffer, cancellationToken);
                bytes = buffer.ToArray();
            }

            var identity = context.MarketplaceIdentity();
            var tags = (form["tags"].ToString() ?? string.Empty).Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
            try
            {
                var upload = new UploadRequest(
                    ns,
                    packageId,
                    files.Select((upload, index) => (paths[index] ?? string.Empty, upload)).ToArray(),
                    bytes,
                    Optional(form["base"]),
                    Optional(form["name"]),
                    Optional(form["description"]));
                var archive = await publish.PrepareUploadAsync(identity, upload, cancellationToken);
                var request = new PublishRequest(ns, packageId, form["version"].ToString(), tags, form["changelog"].ToString(), archive);
                var published = await publish.PublishAsync(identity, request, cancellationToken);
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

        authenticated.MapGet("/mine", async (HttpContext context, MarketplaceDbContext db, IOptions<AuthOptions> auth, CancellationToken cancellationToken) =>
        {
            var identity = context.MarketplaceIdentity();
            var namespaces = identity.Namespaces.ToArray();
            var packages = await db.Packages.AsNoTracking().Include(package => package.Versions)
                .Where(package => namespaces.Contains(package.Namespace))
                .OrderBy(package => package.Namespace).ThenBy(package => package.PackageId)
                .ToListAsync(cancellationToken);
            return Results.Ok(new MineView(
                namespaces.Select(ns => new SpaceView(ns, IdentityResolver.NamespaceDisplayName(auth.Value, identity, ns), IdentityResolver.Lane(auth.Value, ns))).ToArray(),
                packages.Select(package => PackageView.From(package, owner: true)).ToArray()));
        }).WithName("Mine");

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

        authenticated.MapGet("/stats/packages/{ns}/{packageId}", async (string ns, string packageId, HttpContext context, MarketplaceDbContext db, AccessService access, EventsService events, CancellationToken cancellationToken) =>
        {
            var package = await VisiblePackageAsync(context, db, access, ns, packageId, cancellationToken);
            return package is null ? Results.NotFound() : Results.Ok(await events.PackageStatsAsync(package.CanonicalId, cancellationToken));
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

        authenticated.MapGet("/access/{ns}/{packageId?}", async (string ns, string? packageId, HttpContext context, AccessService access, CancellationToken cancellationToken) =>
        {
            try
            {
                return Results.Ok(await access.GetAsync(context.MarketplaceIdentity(), ns, packageId, cancellationToken));
            }
            catch (PublishRejectedException rejected)
            {
                return Rejected(rejected);
            }
        }).WithName("Access");

        authenticated.MapPut("/access/{ns}/{packageId?}", async (string ns, string? packageId, AccessRequest request, HttpContext context, AccessService access, CancellationToken cancellationToken) =>
        {
            try
            {
                return Results.Ok(await access.SetAsync(context.MarketplaceIdentity(), ns, packageId, request.Users, request.Groups, cancellationToken));
            }
            catch (PublishRejectedException rejected)
            {
                return Rejected(rejected);
            }
        }).WithName("SetAccess");

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
            Results.Ok(await db.Reports.AsNoTracking()
                .OrderBy(report => report.ResolvedAt != null)
                .ThenByDescending(report => report.CreatedAt)
                .Take(200)
                .ToListAsync(cancellationToken)))
            .WithName("AdminReports");

        admin.MapPost("/reports/{id:long}/resolve", async (long id, HttpContext context, MarketplaceDbContext db, TimeProvider time, CancellationToken cancellationToken) =>
        {
            var report = await db.Reports.FindAsync([id], cancellationToken);
            if (report is null)
            {
                return Results.NotFound();
            }

            if (report.ResolvedAt is null)
            {
                report.ResolvedAt = time.GetUtcNow().UtcDateTime;
                report.ResolvedBy = context.MarketplaceIdentity().Account;
                await db.SaveChangesAsync(cancellationToken);
            }

            return Results.NoContent();
        }).WithName("ResolveReport");

        admin.MapGet("/reviews", async (PublishService publish, CancellationToken cancellationToken) =>
            Results.Ok(await publish.PendingAsync(cancellationToken))).WithName("Reviews");

        admin.MapPost("/reviews/{ns}/{packageId}/{version}", async (string ns, string packageId, string version, ReviewRequest review, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            if (review.Decision is not ("approve" or "reject"))
            {
                return Results.Problem(statusCode: 422, title: "The decision is approve or reject.");
            }

            try
            {
                return await publish.ReviewAsync(context.MarketplaceIdentity(), ns, packageId, version, review.Decision == "approve", review.Note, cancellationToken)
                    ? Results.NoContent()
                    : Results.NotFound();
            }
            catch (PublishRejectedException rejected)
            {
                return Rejected(rejected);
            }
        }).WithName("Review");

        return app;
    }

    public const int MaxFileView = 1024 * 1024;

    public sealed record ReportRequest(string PackageId, string Reason);

    /// <summary>An admin's verdict on a pending version: <c>approve</c> or <c>reject</c> (with a note).</summary>
    public sealed record ReviewRequest(string Decision, string? Note);

    public sealed record PackageFile(string Path, long Size);

    public sealed record SpaceView(string Namespace, string DisplayName, string Lane);

    public sealed record MineView(SpaceView[] Spaces, PackageView[] Packages);

    public sealed record VersionView(
        string Version,
        string ArchiveDigest,
        long SizeBytes,
        string PublishedBy,
        DateTime PublishedAt,
        string? Changelog,
        bool Yanked,
        string[] ComponentKinds,
        ReviewState ReviewState,
        string? ReviewNote);

    public sealed record PackageView(string Id, string Namespace, string PackageId, string Name, string Description, string[] Tags, string? LiveVersion, VersionView[] Versions)
    {
        /// <summary>Owners and admins see every version with its review state; everyone else sees approved ones.</summary>
        public static PackageView From(Package package, bool owner) => new(
            package.CanonicalId,
            package.Namespace,
            package.PackageId,
            package.Name,
            package.Description,
            package.Tags,
            PublishService.LatestVersion(package)?.Version,
            package.Versions
                .Where(version => owner || version.ReviewState == ReviewState.Approved)
                .OrderByDescending(version => SemVer.Parse(version.Version))
                .Select(version => new VersionView(
                    version.Version,
                    version.ArchiveDigest,
                    version.SizeBytes,
                    version.PublishedBy,
                    version.PublishedAt,
                    version.Changelog,
                    version.Yanked,
                    version.ComponentKinds,
                    version.ReviewState,
                    owner ? version.ReviewNote : null))
                .ToArray());
    }

    /// <summary>The package if the caller may see it: allowed by its access rule, and live unless the caller owns it.</summary>
    private static async Task<Package?> VisiblePackageAsync(HttpContext context, MarketplaceDbContext db, AccessService access, string ns, string packageId, CancellationToken cancellationToken)
    {
        var identity = context.MarketplaceIdentity();
        if (!AccessService.IsVisible(await access.RulesAsync(cancellationToken), identity, ns, packageId))
        {
            return null;
        }

        var package = await db.Packages.AsNoTracking().Include(candidate => candidate.Versions)
            .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken);
        return package is not null && (identity.Owns(ns) || PublishService.LatestVersion(package) is not null) ? package : null;
    }

    /// <summary>A version's stored zip, if the caller may read it: owners always, others only when it is live.</summary>
    private static async Task<byte[]?> ReadableArchiveAsync(HttpContext context, MarketplaceDbContext db, AccessService access, IArtifactStore store, string ns, string packageId, string versionText, CancellationToken cancellationToken)
    {
        var package = await VisiblePackageAsync(context, db, access, ns, packageId, cancellationToken);
        var version = package?.Versions.SingleOrDefault(candidate => candidate.Version == versionText);
        if (version is null || !(context.MarketplaceIdentity().Owns(ns) || version is { ReviewState: ReviewState.Approved, Yanked: false }))
        {
            return null;
        }

        return await store.GetAsync(version.StoragePath, cancellationToken);
    }

    private static string? Optional(Microsoft.Extensions.Primitives.StringValues value) =>
        value.ToString() is { Length: > 0 } text ? text.Trim() : null;

    /// <summary>UTF-8 without NUL bytes reads as text; anything else downloads.</summary>
    private static bool IsText(byte[] bytes)
    {
        if (Array.IndexOf(bytes, (byte)0) >= 0)
        {
            return false;
        }

        try
        {
            _ = StrictUtf8.GetCharCount(bytes);
            return true;
        }
        catch (DecoderFallbackException)
        {
            return false;
        }
    }

    private static readonly UTF8Encoding StrictUtf8 = new(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);

    /// <summary>The allowlist for a namespace or package. Both lists empty makes it public.</summary>
    public sealed record AccessRequest(string[]? Users, string[]? Groups);

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
        context.Response.Headers[HeaderNames.CacheControl] = "private, no-cache";
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
