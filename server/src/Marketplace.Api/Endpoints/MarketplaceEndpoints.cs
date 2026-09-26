using System.Reflection;
using System.Security.Cryptography;
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

/// <summary>
/// The HTTP surface. Handlers return only their success result; every failure is a thrown
/// <see cref="ProblemException"/> that <see cref="ProblemExceptionHandler"/> writes as a problem document.
/// </summary>
public static class MarketplaceEndpoints
{
    public const string AdminPolicy = "Admin";
    public const int MaxFileView = 1024 * 1024;
    public const int MaxReportReason = 2048;
    public const long MaxUploadRequestBytes = ArchiveInspector.MaxArchiveBytes + 1024 * 1024;

    public static IEndpointRouteBuilder MapMarketplace(this IEndpointRouteBuilder app, IReadOnlyList<string> authSchemes)
    {
        var api = app.MapGroup("/api");
        var serverVersion = typeof(MarketplaceEndpoints).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion.Split('+')[0]
            ?? typeof(MarketplaceEndpoints).Assembly.GetName().Version?.ToString(3)
            ?? "0.0.0";

        api.MapGet("/health", async Task<Ok<HealthView>> (IOptions<ClientOptions> client, IHostEnvironment environment, MarketplaceDbContext db, CancellationToken cancellationToken) =>
        {
            if (!await db.Database.CanConnectAsync(cancellationToken))
            {
                throw ProblemException.DatabaseUnavailable();
            }

            return TypedResults.Ok(new HealthView(serverVersion, client.Value.MinimumVersion, client.Value.LatestVersion, environment.EnvironmentName, authSchemes));
        }).AllowAnonymous().WithName("Health").ProducesProblem(503);

        // Every authenticated route settles the caller's identity, personal namespace included, before it runs.
        // Credentials are ambient (Negotiate), so a browser's cross-site write is refused; the CLI and desktop send no Sec-Fetch-Site.
        var authenticated = api.MapGroup("").RequireAuthorization().AddEndpointFilter(async (invocation, next) =>
        {
            var request = invocation.HttpContext.Request;
            if (!HttpMethods.IsGet(request.Method) && !HttpMethods.IsHead(request.Method)
                && request.Headers.TryGetValue("Sec-Fetch-Site", out var site) && site.ToString() is not ("same-origin" or "none"))
            {
                throw new ProblemException(403, "The marketplace refuses changes sent from another website. Use the marketplace portal, the desktop app, or the CLI.");
            }

            await invocation.HttpContext.ResolveMarketplaceIdentityAsync();
            return await next(invocation);
        });

        authenticated.MapGet("/me", (HttpContext context) =>
        {
            var identity = context.MarketplaceIdentity();
            return TypedResults.Ok(new MeView(identity.Account, identity.Namespace, identity.DisplayName, identity.Namespaces, identity.IsAdmin, identity.Groups));
        }).WithName("Me");

        authenticated.MapGet("/catalog", async (HttpContext context, CatalogService catalog, CancellationToken cancellationToken) =>
        {
            var document = await catalog.CatalogAsync(context.MarketplaceIdentity(), cancellationToken);
            return Conditional(context, document.Bytes, document.Digest, "application/json");
        }).WithName("Catalog").Produces(200, contentType: "application/json");

        authenticated.MapMethods("/sources/{ns}/archive", ["GET", "HEAD"], async (string ns, HttpContext context, MarketplaceDbContext db, AccessService access, CancellationToken cancellationToken) =>
        {
            // HEAD and a matching If-None-Match are answered from the stored digest; the bytes load only for a body.
            var archive = await db.NamespaceArchives
                .Where(candidate => candidate.Namespace == ns)
                .Select(candidate => new { candidate.Digest, candidate.PackageCount, candidate.GeneratedAt, candidate.Bytes.Length })
                .SingleOrDefaultAsync(cancellationToken);
            if (archive is null || archive.PackageCount == 0)
            {
                throw ProblemException.NotFound($"The source {ns}");
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
                throw ProblemException.NotFound($"The source {ns}");
            }

            // A subset's ETag names the full archive and the kept ids, so it is known without building the subset.
            var filtered = keep.Count != live.Count;
            var digest = filtered
                ? Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(string.Join('\n', keep.Order(StringComparer.Ordinal).Prepend(archive.Digest)))))
                : archive.Digest;
            context.Response.Headers[HeaderNames.LastModified] = archive.GeneratedAt.ToString("R");
            context.Response.Headers[HeaderNames.ContentDisposition] = $"attachment; filename=\"{ns}-latest.zip\"";
            if (Unchanged(context, digest, "application/zip", filtered ? null : archive.Length) is { } answered)
            {
                return answered;
            }

            var bytes = await db.NamespaceArchives.Where(candidate => candidate.Namespace == ns).Select(candidate => candidate.Bytes).SingleOrDefaultAsync(cancellationToken)
                ?? throw ProblemException.NotFound($"The source {ns}");
            return Results.Bytes(filtered ? NamespaceArchiveBuilder.Filter(bytes, keep).Bytes : bytes, "application/zip");
        }).WithName("SourceArchive").Produces(200, contentType: "application/zip").ProducesProblem(404);

        authenticated.MapGet("/index", async (HttpContext context, CatalogService catalog, CancellationToken cancellationToken) =>
            TypedResults.Ok(await catalog.IndexAsync(context.MarketplaceIdentity(), cancellationToken))).WithName("Index");

        authenticated.MapGet("/packages/{ns}/{packageId}", async (string ns, string packageId, HttpContext context, MarketplaceDbContext db, AccessService access, CancellationToken cancellationToken) =>
        {
            var package = await VisiblePackageAsync(context, db, access, ns, packageId, cancellationToken);
            return TypedResults.Ok(PackageView.From(package, context.MarketplaceIdentity().Owns(ns)));
        }).WithName("Package").ProducesProblem(404);

        authenticated.MapMethods("/packages/{ns}/{packageId}/versions/{version}/archive", ["GET", "HEAD"], async (string ns, string packageId, string version, HttpContext context, MarketplaceDbContext db, AccessService access, IArtifactStore store, CancellationToken cancellationToken) =>
        {
            var (stored, bytes) = await ReadableArchiveAsync(context, db, access, store, ns, packageId, version, cancellationToken);
            context.Response.Headers[HeaderNames.ContentDisposition] = $"attachment; filename=\"{ns}-{packageId}-{stored.Version}.zip\"";
            return Conditional(context, bytes, stored.ArchiveDigest, "application/zip");
        }).WithName("VersionArchive").Produces(200, contentType: "application/zip").ProducesProblem(404);

        authenticated.MapGet("/packages/{ns}/{packageId}/versions/{version}/files", async (string ns, string packageId, string version, HttpContext context, MarketplaceDbContext db, AccessService access, IArtifactStore store, CancellationToken cancellationToken) =>
        {
            var (_, archive) = await ReadableArchiveAsync(context, db, access, store, ns, packageId, version, cancellationToken);
            using var zip = ArchiveInspector.OpenZip(new MemoryStream(archive, writable: false));
            var prefix = ArchiveInspector.RootPrefix(zip.Entries.Select(entry => entry.FullName).ToArray());
            var files = zip.Entries
                .Where(entry => entry.FullName.StartsWith(prefix, StringComparison.Ordinal) && !entry.FullName.EndsWith('/'))
                .Select(entry => new PackageFile(entry.FullName[prefix.Length..], entry.Length))
                .OrderBy(file => file.Path, StringComparer.Ordinal)
                .ToArray();
            return TypedResults.Ok(files);
        }).WithName("PackageFiles").ProducesProblem(404);

        authenticated.MapGet("/packages/{ns}/{packageId}/versions/{version}/files/{**path}", async (string ns, string packageId, string version, string path, HttpContext context, MarketplaceDbContext db, AccessService access, IArtifactStore store, CancellationToken cancellationToken) =>
        {
            var (_, archive) = await ReadableArchiveAsync(context, db, access, store, ns, packageId, version, cancellationToken);

            // Not disposed: the archive only wraps a MemoryStream, and a large entry streams from it after this returns.
            var zip = ArchiveInspector.OpenZip(new MemoryStream(archive, writable: false));
            var prefix = ArchiveInspector.RootPrefix(zip.Entries.Select(entry => entry.FullName).ToArray());
            var entry = zip.GetEntry(prefix + path);
            if (entry is null || entry.FullName.EndsWith('/'))
            {
                throw ProblemException.NotFound($"{path} in {ns}/{packageId} {version}");
            }

            if (entry.Length > MaxFileView)
            {
                return Results.Stream(entry.Open(), "application/octet-stream", Path.GetFileName(path));
            }

            var bytes = new byte[entry.Length];
            await using (var stream = entry.Open())
            {
                await stream.ReadExactlyAsync(bytes, cancellationToken);
            }

            return IsText(bytes)
                ? Results.Bytes(bytes, "text/plain; charset=utf-8")
                : Results.File(bytes, "application/octet-stream", Path.GetFileName(path));
        }).WithName("PackageFile").Produces(200, contentType: "text/plain").Produces(200, contentType: "application/octet-stream").ProducesProblem(404);

        authenticated.MapPost("/packages/{ns}/{packageId}/versions", async Task<Created<PublishedVersion>> (string ns, string packageId, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            if (!context.Request.HasFormContentType)
            {
                throw new ProblemException(415, "Publish with multipart/form-data: an archive, or files with one paths value each, plus version, tags, and changelog.");
            }

            IFormCollection form;
            try
            {
                form = await context.Request.ReadFormAsync(cancellationToken);
            }
            catch (InvalidDataException error)
            {
                throw new ProblemException(400, $"The upload form could not be read: {error.Message}");
            }

            var file = form.Files.GetFile("archive");
            var files = form.Files.GetFiles("files");
            var paths = form["paths"];
            if ((file is null || file.Length == 0) && files.Count == 0)
            {
                throw new ProblemException(422, "The form has no archive and no files.");
            }

            if (files.Count != paths.Count)
            {
                throw new ProblemException(422, $"Send one paths value for each file: the form has {files.Count} files and {paths.Count} paths.");
            }

            if ((file?.Length ?? 0) + files.Sum(upload => upload.Length) > ArchiveInspector.MaxArchiveBytes)
            {
                throw new ProblemException(413, $"The upload is larger than the {ArchiveInspector.MaxArchiveBytes / 1024 / 1024} MB limit.");
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
            var tags = form["tags"].ToString().Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
            var upload = new UploadRequest(
                ns,
                packageId,
                files.Select((upload, index) => (paths[index] ?? string.Empty, upload)).ToArray(),
                bytes,
                Optional(form["name"]),
                Optional(form["description"]));
            var archive = await publish.PrepareUploadAsync(identity, upload, cancellationToken);
            var request = new PublishRequest(ns, packageId, form["version"].ToString(), tags, Optional(form["changelog"]), archive);
            var published = await publish.PublishAsync(identity, request, cancellationToken);
            return TypedResults.Created($"/api/packages/{ns}/{packageId}", published);
        })
            .WithName("Publish")
            .DisableAntiforgery()
            .WithMetadata(new RequestSizeLimitAttribute(MaxUploadRequestBytes))
            .ProducesProblem(403).ProducesProblem(409).ProducesProblem(413).ProducesProblem(415).ProducesProblem(422).ProducesProblem(503);

        authenticated.MapPut("/packages/{ns}/{packageId}/versions/{version}/yank", async Task<NoContent> (string ns, string packageId, string version, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            await publish.SetYankedAsync(context.MarketplaceIdentity(), ns, packageId, version, yanked: true, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("Yank").ProducesProblem(403).ProducesProblem(404);

        authenticated.MapDelete("/packages/{ns}/{packageId}/versions/{version}/yank", async Task<NoContent> (string ns, string packageId, string version, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            await publish.SetYankedAsync(context.MarketplaceIdentity(), ns, packageId, version, yanked: false, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("Unyank").ProducesProblem(403).ProducesProblem(404);

        authenticated.MapPost("/packages/{ns}/{packageId}/reports", async Task<Accepted> (string ns, string packageId, ReportRequest report, HttpContext context, MarketplaceDbContext db, AccessService access, TimeProvider time, CancellationToken cancellationToken) =>
        {
            var reason = report.Reason?.Trim() ?? string.Empty;
            if (reason.Length is 0 or > MaxReportReason)
            {
                throw new ProblemException(422, $"A report needs a reason of up to {MaxReportReason:N0} characters.");
            }

            var package = await VisiblePackageAsync(context, db, access, ns, packageId, cancellationToken);
            db.Reports.Add(new PackageReport
            {
                Account = context.MarketplaceIdentity().Account,
                PackageId = package.CanonicalId,
                Reason = reason,
                CreatedAt = time.GetUtcNow().UtcDateTime,
            });
            await db.SaveChangesAsync(cancellationToken);
            return TypedResults.Accepted((string?)null);
        }).WithName("Report").ProducesProblem(404).ProducesProblem(422);

        authenticated.MapGet("/mine", async (HttpContext context, MarketplaceDbContext db, IOptions<AuthOptions> auth, CancellationToken cancellationToken) =>
        {
            var identity = context.MarketplaceIdentity();
            var namespaces = identity.Namespaces.ToArray();
            var packages = await db.Packages.AsNoTracking().Include(package => package.Versions)
                .Where(package => namespaces.Contains(package.Namespace))
                .OrderBy(package => package.Namespace).ThenBy(package => package.PackageId)
                .ToListAsync(cancellationToken);
            return TypedResults.Ok(new MineView(
                namespaces.Select(ns => new SpaceView(ns, IdentityResolver.NamespaceDisplayName(auth.Value, identity, ns), IdentityResolver.Lane(auth.Value, ns))).ToArray(),
                packages.Select(package => PackageView.From(package, owner: true)).ToArray()));
        }).WithName("Mine");

        authenticated.MapPost("/events", async Task<Accepted<EventsAccepted>> (EventsBatch batch, HttpContext context, EventsService events, CancellationToken cancellationToken) =>
        {
            if (batch.Events is null)
            {
                throw new ProblemException(422, "The body has no events array.");
            }

            var accepted = await events.RecordAsync(context.MarketplaceIdentity().Account, batch, cancellationToken);
            return TypedResults.Accepted((string?)null, accepted);
        }).WithName("Events").ProducesProblem(400).ProducesProblem(422);

        authenticated.MapGet("/stats/packages/{ns}/{packageId}", async (string ns, string packageId, HttpContext context, MarketplaceDbContext db, AccessService access, EventsService events, CancellationToken cancellationToken) =>
        {
            var package = await VisiblePackageAsync(context, db, access, ns, packageId, cancellationToken);
            return TypedResults.Ok(await events.PackageStatsAsync(package.CanonicalId, cancellationToken));
        }).WithName("PackageStats").ProducesProblem(404);

        authenticated.MapGet("/access/{ns}/{packageId?}", async (string ns, string? packageId, HttpContext context, AccessService access, CancellationToken cancellationToken) =>
            TypedResults.Ok(await access.GetAsync(context.MarketplaceIdentity(), ns, packageId, cancellationToken)))
            .WithName("Access").ProducesProblem(403).ProducesProblem(422);

        authenticated.MapPut("/access/{ns}/{packageId?}", async (string ns, string? packageId, AccessRequest request, HttpContext context, AccessService access, CancellationToken cancellationToken) =>
            TypedResults.Ok(await access.SetAsync(context.MarketplaceIdentity(), ns, packageId, request.Users, request.Groups, cancellationToken)))
            .WithName("SetAccess").ProducesProblem(403).ProducesProblem(404).ProducesProblem(422);

        var admin = authenticated.MapGroup("/admin").RequireAuthorization(AdminPolicy);
        admin.MapGet("/summary", async (EventsService events, CancellationToken cancellationToken) =>
            TypedResults.Ok(await events.AdminSummaryAsync(cancellationToken))).WithName("AdminSummary");

        admin.MapGet("/reports", async (MarketplaceDbContext db, CancellationToken cancellationToken) =>
            TypedResults.Ok(await db.Reports.AsNoTracking()
                .OrderBy(report => report.ResolvedAt != null)
                .ThenByDescending(report => report.CreatedAt)
                .Take(200)
                .Select(report => new ReportView(report.Id, report.Account, report.PackageId, report.Reason, report.CreatedAt, report.ResolvedAt, report.ResolvedBy))
                .ToListAsync(cancellationToken)))
            .WithName("AdminReports");

        admin.MapPost("/reports/{id:long}/resolve", async Task<NoContent> (long id, HttpContext context, MarketplaceDbContext db, TimeProvider time, CancellationToken cancellationToken) =>
        {
            var report = await db.Reports.FindAsync([id], cancellationToken) ?? throw ProblemException.NotFound($"Report {id}");
            if (report.ResolvedAt is null)
            {
                report.ResolvedAt = time.GetUtcNow().UtcDateTime;
                report.ResolvedBy = context.MarketplaceIdentity().Account;
                await db.SaveChangesAsync(cancellationToken);
            }

            return TypedResults.NoContent();
        }).WithName("ResolveReport").ProducesProblem(404);

        admin.MapGet("/reviews", async (PublishService publish, CancellationToken cancellationToken) =>
            TypedResults.Ok(await publish.PendingAsync(cancellationToken))).WithName("Reviews");

        admin.MapPost("/reviews/{ns}/{packageId}/{version}", async Task<NoContent> (string ns, string packageId, string version, ReviewRequest review, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            if (review.Decision is not ("approve" or "reject"))
            {
                throw new ProblemException(422, "The decision is approve or reject.");
            }

            await publish.ReviewAsync(context.MarketplaceIdentity(), ns, packageId, version, review.Decision == "approve", review.Note, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("Review").ProducesProblem(404).ProducesProblem(409).ProducesProblem(422);

        return app;
    }

    public sealed record HealthView(string ServerVersion, string MinimumClientVersion, string LatestClientVersion, string Environment, IReadOnlyList<string> AuthSchemes);

    public sealed record MeView(string Account, string Namespace, string DisplayName, IReadOnlyList<string> Namespaces, bool Admin, IReadOnlyList<string> Groups);

    public sealed record ReportRequest(string? Reason);

    public sealed record ReportView(long Id, string Account, string PackageId, string Reason, DateTime CreatedAt, DateTime? ResolvedAt, string? ResolvedBy);

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

    /// <summary>The allowlist for a namespace or package. Both lists empty makes it public.</summary>
    public sealed record AccessRequest(string[]? Users, string[]? Groups);

    /// <summary>The package if the caller may see it: allowed by its access rule, and live unless the caller owns it.</summary>
    private static async Task<Package> VisiblePackageAsync(HttpContext context, MarketplaceDbContext db, AccessService access, string ns, string packageId, CancellationToken cancellationToken)
    {
        var identity = context.MarketplaceIdentity();
        var package = AccessService.IsVisible(await access.RulesAsync(cancellationToken), identity, ns, packageId)
            ? await db.Packages.AsNoTracking().Include(candidate => candidate.Versions)
                .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken)
            : null;
        return package is not null && (identity.Owns(ns) || PublishService.LatestVersion(package) is not null)
            ? package
            : throw ProblemException.NotFound($"The package {ns}/{packageId}");
    }

    /// <summary>A version and its stored zip, if the caller may read it: owners always, others only when it is live.</summary>
    private static async Task<(PackageVersion Version, byte[] Bytes)> ReadableArchiveAsync(HttpContext context, MarketplaceDbContext db, AccessService access, IArtifactStore store, string ns, string packageId, string versionText, CancellationToken cancellationToken)
    {
        var package = await VisiblePackageAsync(context, db, access, ns, packageId, cancellationToken);
        var version = package.Versions.SingleOrDefault(candidate => candidate.Version == versionText);
        if (version is null || !(context.MarketplaceIdentity().Owns(ns) || version is { ReviewState: ReviewState.Approved, Yanked: false }))
        {
            throw ProblemException.NotFound($"{ns}/{packageId} {versionText}");
        }

        return (version, await store.GetAsync(version.StoragePath, cancellationToken));
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

    /// <summary>Serves bytes with a strong ETag and honors If-None-Match.</summary>
    private static IResult Conditional(HttpContext context, byte[] bytes, string digest, string contentType) =>
        Unchanged(context, digest, contentType, bytes.Length) ?? Results.Bytes(bytes, contentType);

    /// <summary>
    /// Sets a strong ETag, then answers a matching If-None-Match with 304 and a HEAD with headers only
    /// (<paramref name="length"/> when known). Null means the caller sends the body.
    /// </summary>
    private static IResult? Unchanged(HttpContext context, string digest, string contentType, long? length)
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
            context.Response.ContentLength = length;
            context.Response.ContentType = contentType;
            return Results.Empty;
        }

        return null;
    }
}

public static class HttpContextIdentityExtensions
{
    private const string Key = "marketplace.identity";

    /// <summary>Resolves the caller once per request, including the database lookup that settles their personal namespace.</summary>
    public static async Task ResolveMarketplaceIdentityAsync(this HttpContext context)
    {
        var options = context.RequestServices.GetRequiredService<IOptions<AuthOptions>>().Value;
        var db = context.RequestServices.GetRequiredService<MarketplaceDbContext>();
        context.Items[Key] = await IdentityResolver.ClaimPersonalNamespaceAsync(IdentityResolver.Resolve(context.User, options), db, options, context.RequestAborted);
    }

    public static MarketplaceIdentity MarketplaceIdentity(this HttpContext context) =>
        context.Items.TryGetValue(Key, out var cached) && cached is MarketplaceIdentity identity
            ? identity
            : throw new InvalidOperationException("The caller's identity is resolved by the authenticated route group's filter, which did not run.");
}
