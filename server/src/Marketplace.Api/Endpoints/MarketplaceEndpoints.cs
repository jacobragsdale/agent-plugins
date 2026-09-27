using System.Collections.Concurrent;
using System.DirectoryServices.Protocols;
using System.Net;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Marketplace.Api.Access;
using Marketplace.Api.Auth;
using Marketplace.Api.Catalog;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Marketplace.Api.Events;
using Marketplace.Api.Notifications;
using Marketplace.Api.Packages;
using Marketplace.Api.Storage;
using Marketplace.Api.Teams;
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

        api.MapGet("/health", async Task<Ok<HealthView>> (IOptions<ClientOptions> client, IOptions<AuthOptions> auth, IHostEnvironment environment, MarketplaceDbContext db, IArtifactStore store, ILoggerFactory loggers, TimeProvider time, CancellationToken cancellationToken) =>
        {
            if (!await db.Database.CanConnectAsync(cancellationToken))
            {
                throw ProblemException.DatabaseUnavailable();
            }

            // The store and LDAP are probed at most once a minute, so a monitoring loop can't hammer them.
            var now = time.GetUtcNow().UtcDateTime;
            if (_probe is not { } probe || now - probe.At > TimeSpan.FromMinutes(1))
            {
                probe = _probe = new HealthProbe(now, await store.CheckAsync(cancellationToken) ? "ok" : "error", LdapStatus(auth.Value, loggers.CreateLogger("Marketplace.Api.Health")));
            }

            return TypedResults.Ok(new HealthView(serverVersion, client.Value.MinimumVersion, client.Value.LatestVersion, environment.EnvironmentName, authSchemes, auth.Value.LdapDomain is { Length: > 0 }, probe.ArtifactStore, probe.Ldap));
        }).AllowAnonymous().WithName("Health").ProducesProblem(503);

        // Every authenticated route settles the caller's identity, personal namespace and teams included, before it runs.
        // Credentials are ambient (Negotiate), so a browser's cross-site write is refused; the CLI and desktop send no Sec-Fetch-Site.
        var authenticated = api.MapGroup("").RequireAuthorization().AddEndpointFilter(async (invocation, next) =>
        {
            var request = invocation.HttpContext.Request;
            if (!HttpMethods.IsGet(request.Method) && !HttpMethods.IsHead(request.Method)
                && request.Headers.TryGetValue("Sec-Fetch-Site", out var site) && site.ToString() is not ("same-origin" or "none"))
            {
                throw new ProblemException(403, "The marketplace refuses changes sent from another website. Use the marketplace portal, the desktop app, or the CLI.");
            }

            var path = request.Path.Value ?? string.Empty;
            var reporting = (HttpMethods.IsGet(request.Method) && path == "/api/me") || (HttpMethods.IsPost(request.Method) && path == "/api/events");
            if (!reporting && TooOld(request, invocation.HttpContext.RequestServices) is { } tooOld)
            {
                throw tooOld;
            }

            await invocation.HttpContext.ResolveMarketplaceIdentityAsync();
            if (invocation.HttpContext.MarketplaceIdentity().IsBlocked && !HttpMethods.IsGet(request.Method) && !HttpMethods.IsHead(request.Method)
                && !reporting && path != "/api/notifications/read")
            {
                throw new ProblemException(403, "Your account is blocked from changing anything in the marketplace. Contact the marketplace admins.");
            }

            return await next(invocation);
        });

        authenticated.MapGet("/me", async (HttpContext context, MarketplaceDbContext db, SuggestionService suggestions, EventsService events, NotificationService notifications, CancellationToken cancellationToken) =>
        {
            var identity = context.MarketplaceIdentity();

            // ponytail: open reports are few, so they are filtered here; index Reports by namespace if that stops being true.
            var open = await db.Reports.AsNoTracking().Where(report => report.ResolvedAt == null).Select(report => report.PackageId).ToListAsync(cancellationToken);
            return TypedResults.Ok(new MeView(
                identity.Account,
                identity.Namespace,
                identity.DisplayName,
                identity.Namespaces,
                identity.IsAdmin,
                identity.Groups,
                identity.Teams,
                await suggestions.WaitingCountAsync(identity, cancellationToken),
                await events.AppAsync(identity.Account, cancellationToken),
                open.Count(id => identity.Namespaces.Contains(AccessService.Split(id).Namespace, StringComparer.Ordinal)),
                await notifications.UnreadAsync(identity, cancellationToken)));
        }).WithName("Me");

        // HEAD lets the desktop app compare validators before it downloads the catalog.
        authenticated.MapMethods("/catalog", ["GET", "HEAD"], async (HttpContext context, CatalogService catalog, CancellationToken cancellationToken) =>
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
                .Where(package => AccessService.IsVisible(rules, identity, ns, package.PackageId, package.Gated))
                .Select(package => package.PackageId)
                .ToHashSet(StringComparer.Ordinal);
            if (keep.Count == 0)
            {
                throw ProblemException.NotFound($"The source {ns}");
            }

            // A subset's ETag names the full archive and the kept ids, so it is known without building the subset.
            var filtered = keep.Count != live.Count;
            var digest = AccessService.ArchiveDigest(archive.Digest, keep, live.Count);
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

        authenticated.MapGet("/index", async (HttpContext context, CatalogService catalog, IOptions<Microsoft.AspNetCore.Http.Json.JsonOptions> json, CancellationToken cancellationToken) =>
        {
            var bytes = JsonSerializer.SerializeToUtf8Bytes(await catalog.IndexAsync(context.MarketplaceIdentity(), cancellationToken), json.Value.SerializerOptions);
            return Conditional(context, bytes, Convert.ToHexStringLower(SHA256.HashData(bytes)), "application/json");
        }).WithName("Index").Produces<IndexDocument>(200, "application/json");

        authenticated.MapGet("/packages/{ns}/{packageId}", async (string ns, string packageId, HttpContext context, MarketplaceDbContext db, AccessService access, EventsService events, CancellationToken cancellationToken) =>
        {
            var identity = context.MarketplaceIdentity();
            var package = await access.VisiblePackageAsync(identity, ns, packageId, cancellationToken);
            var stats = await events.PackageStatsAsync(package.CanonicalId, cancellationToken);
            var publisher = await db.Publishers.AsNoTracking().SingleOrDefaultAsync(candidate => candidate.Namespace == ns, cancellationToken);
            return TypedResults.Ok(PackageView.From(package, identity, await access.RulesAsync(cancellationToken), publisher, (stats.Installs, stats.InstalledBase)));
        }).WithName("Package").ProducesProblem(404);

        authenticated.MapMethods("/packages/{ns}/{packageId}/versions/{version}/archive", ["GET", "HEAD"], async (string ns, string packageId, string version, HttpContext context, AccessService access, IArtifactStore store, CancellationToken cancellationToken) =>
        {
            var (stored, bytes) = await ReadableArchiveAsync(context, access, store, ns, packageId, version, cancellationToken);
            context.Response.Headers[HeaderNames.ContentDisposition] = $"attachment; filename=\"{ns}-{packageId}-{stored.Version}.zip\"";
            return Conditional(context, bytes, stored.ArchiveDigest, "application/zip");
        }).WithName("VersionArchive").Produces(200, contentType: "application/zip").ProducesProblem(404);

        authenticated.MapGet("/packages/{ns}/{packageId}/versions/{version}/files", async (string ns, string packageId, string version, HttpContext context, AccessService access, IArtifactStore store, CancellationToken cancellationToken) =>
        {
            var (_, archive) = await ReadableArchiveAsync(context, access, store, ns, packageId, version, cancellationToken);
            return TypedResults.Ok(ListFiles(archive));
        }).WithName("PackageFiles").ProducesProblem(404);

        authenticated.MapGet("/packages/{ns}/{packageId}/versions/{version}/files/{**path}", async (string ns, string packageId, string version, string path, HttpContext context, AccessService access, IArtifactStore store, CancellationToken cancellationToken) =>
        {
            var (_, archive) = await ReadableArchiveAsync(context, access, store, ns, packageId, version, cancellationToken);
            return await FileAsync(archive, path, $"{path} in {ns}/{packageId} {version}", cancellationToken);
        }).WithName("PackageFile").Produces(200, contentType: "text/plain").Produces(200, contentType: "application/octet-stream").ProducesProblem(404);

        authenticated.MapPost("/packages/{ns}/{packageId}/versions", async Task<Results<Created<PublishedVersion>, Ok<PublishedVersion>, Ok<DryRunView>>> (string ns, string packageId, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            var identity = context.MarketplaceIdentity();
            PublishService.CheckTarget(identity, ns, packageId);
            var (form, upload) = await ReadUploadAsync(context, ns, packageId, cancellationToken);
            var visibility = Optional(form["visibility"]) switch
            {
                null => (Visibility?)null,
                "inherit" => Visibility.Inherit,
                "public" => Visibility.Public,
                "private" => Visibility.Private,
                var other => throw new ProblemException(422, $"Who can install it is inherit, public, or private, not {other}."),
            };
            var archive = await publish.PrepareUploadAsync(upload, cancellationToken);
            var tags = form["tags"].ToString().Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
            var request = new PublishRequest(ns, packageId, form["version"].ToString(), tags, Optional(form["changelog"]), archive, visibility);
            if (Optional(form["dryRun"]) is "true")
            {
                return TypedResults.Ok(await publish.DryRunAsync(identity, request, cancellationToken));
            }

            var (published, created) = await publish.PublishAsync(identity, request, cancellationToken);
            return created ? TypedResults.Created($"/api/packages/{ns}/{packageId}", published) : TypedResults.Ok(published);
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

        authenticated.MapPut("/packages/{ns}/{packageId}/revoke", async Task<NoContent> (string ns, string packageId, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            await publish.SetRevokedAsync(context.MarketplaceIdentity(), ns, packageId, revoked: true, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("Revoke").ProducesProblem(403).ProducesProblem(404);

        authenticated.MapDelete("/packages/{ns}/{packageId}/revoke", async Task<NoContent> (string ns, string packageId, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            await publish.SetRevokedAsync(context.MarketplaceIdentity(), ns, packageId, revoked: false, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("Unrevoke").ProducesProblem(403).ProducesProblem(404);

        authenticated.MapDelete("/packages/{ns}/{packageId}", async Task<NoContent> (string ns, string packageId, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            await publish.DeleteAsync(context.MarketplaceIdentity(), ns, packageId, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("DeletePackage").ProducesProblem(403).ProducesProblem(404).ProducesProblem(409);

        authenticated.MapPost("/packages/{ns}/{packageId}/reports", async Task<Accepted> (string ns, string packageId, ReportRequest report, HttpContext context, MarketplaceDbContext db, AccessService access, NotificationService notifications, TimeProvider time, CancellationToken cancellationToken) =>
        {
            var reason = report.Reason?.Trim() ?? string.Empty;
            if (reason.Length is 0 or > MaxReportReason)
            {
                throw new ProblemException(422, $"A report needs a reason of up to {MaxReportReason:N0} characters.");
            }

            var kind = report.Kind ?? PackageReport.Problem;
            if (kind is not (PackageReport.Problem or PackageReport.Feedback))
            {
                throw new ProblemException(422, "A report is a problem or feedback.");
            }

            var identity = context.MarketplaceIdentity();
            var package = await access.VisiblePackageAsync(identity, ns, packageId, cancellationToken);
            db.Reports.Add(new PackageReport
            {
                Account = identity.Account,
                PackageId = package.CanonicalId,
                Kind = kind,
                Reason = reason,
                CreatedAt = time.GetUtcNow().UtcDateTime,
            });
            var what = kind == PackageReport.Problem ? "reported a problem with" : "left feedback on";
            notifications.Notify(await notifications.OwnersAsync(ns, cancellationToken), identity.Account, "report.created", $"{identity.DisplayName} {what} {package.Name}: {reason}", $"/p/{ns}/{packageId}");
            await db.SaveChangesAsync(cancellationToken);
            if (kind == PackageReport.Problem)
            {
                notifications.Webhook($"{identity.DisplayName} reported a problem with {package.Name} ({package.CanonicalId}): {reason}", "/admin");
            }

            return TypedResults.Accepted((string?)null);
        }).WithName("Report").ProducesProblem(404).ProducesProblem(422);

        authenticated.MapGet("/packages/{ns}/{packageId}/reports", async (string ns, string packageId, HttpContext context, MarketplaceDbContext db, AccessService access, CancellationToken cancellationToken) =>
        {
            var identity = context.MarketplaceIdentity();
            var package = await access.VisiblePackageAsync(identity, ns, packageId, cancellationToken);
            if (!identity.Owns(ns))
            {
                throw new ProblemException(403, "Only the package's owners see its reports.");
            }

            return TypedResults.Ok(await Reports(db.Reports.Where(report => report.PackageId == package.CanonicalId)).ToListAsync(cancellationToken));
        }).WithName("PackageReports").ProducesProblem(403).ProducesProblem(404);

        authenticated.MapGet("/reports/mine", async (HttpContext context, MarketplaceDbContext db, CancellationToken cancellationToken) =>
        {
            var account = context.MarketplaceIdentity().Account.ToLower();
            return TypedResults.Ok(await db.Reports.AsNoTracking()
                .Where(report => report.Account.ToLower() == account)
                .OrderByDescending(report => report.CreatedAt)
                .Take(200)
                .Select(report => new ReportView(report.Id, report.Account, report.PackageId, report.Kind, report.Reason, report.CreatedAt, report.ResolvedAt, report.ResolvedBy, report.Note))
                .ToListAsync(cancellationToken));
        }).WithName("MyReports");

        authenticated.MapPost("/reports/{id:long}/resolve", async Task<NoContent> (long id, HttpContext context, MarketplaceDbContext db, NotificationService notifications, TimeProvider time, CancellationToken cancellationToken) =>
        {
            await ResolveReportAsync(context.MarketplaceIdentity(), id, await NoteAsync(context.Request, cancellationToken), db, notifications, time, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("ResolvePackageReport").ProducesProblem(403).ProducesProblem(404).ProducesProblem(422);

        authenticated.MapPost("/packages/{ns}/{packageId}/suggestions", async Task<Created<SuggestionView>> (string ns, string packageId, HttpContext context, SuggestionService suggestions, CancellationToken cancellationToken) =>
        {
            var (form, upload) = await ReadUploadAsync(context, ns, packageId, cancellationToken);
            var suggestion = await suggestions.CreateAsync(context.MarketplaceIdentity(), upload, Optional(form["message"]), cancellationToken);
            return TypedResults.Created($"/api/suggestions/{suggestion.Id}", suggestion);
        })
            .WithName("Suggest")
            .DisableAntiforgery()
            .WithMetadata(new RequestSizeLimitAttribute(MaxUploadRequestBytes))
            .ProducesProblem(404).ProducesProblem(409).ProducesProblem(413).ProducesProblem(415).ProducesProblem(422).ProducesProblem(503);

        authenticated.MapGet("/packages/{ns}/{packageId}/suggestions", async (string ns, string packageId, HttpContext context, SuggestionService suggestions, CancellationToken cancellationToken) =>
            TypedResults.Ok(await suggestions.ForPackageAsync(context.MarketplaceIdentity(), ns, packageId, cancellationToken)))
            .WithName("PackageSuggestions").ProducesProblem(404);

        authenticated.MapGet("/suggestions/{id:long}", async (long id, HttpContext context, SuggestionService suggestions, CancellationToken cancellationToken) =>
            TypedResults.Ok(await suggestions.GetAsync(context.MarketplaceIdentity(), id, cancellationToken)))
            .WithName("Suggestion").ProducesProblem(404);

        authenticated.MapGet("/suggestions/{id:long}/files", async (long id, HttpContext context, SuggestionService suggestions, CancellationToken cancellationToken) =>
            TypedResults.Ok(await suggestions.FilesAsync(context.MarketplaceIdentity(), id, cancellationToken)))
            .WithName("SuggestionFiles").ProducesProblem(404);

        authenticated.MapGet("/suggestions/{id:long}/files/{**path}", async (long id, string path, HttpContext context, SuggestionService suggestions, CancellationToken cancellationToken) =>
            await FileAsync(await suggestions.ArchiveAsync(context.MarketplaceIdentity(), id, cancellationToken), path, $"{path} in suggestion {id}", cancellationToken))
            .WithName("SuggestionFile").Produces(200, contentType: "text/plain").Produces(200, contentType: "application/octet-stream").ProducesProblem(404);

        authenticated.MapPost("/suggestions/{id:long}", async (long id, DecisionRequest decision, HttpContext context, SuggestionService suggestions, CancellationToken cancellationToken) =>
            TypedResults.Ok(await suggestions.DecideAsync(context.MarketplaceIdentity(), id, decision, cancellationToken)))
            .WithName("DecideSuggestion").ProducesProblem(403).ProducesProblem(404).ProducesProblem(409).ProducesProblem(422);

        authenticated.MapDelete("/suggestions/{id:long}", async Task<NoContent> (long id, HttpContext context, SuggestionService suggestions, CancellationToken cancellationToken) =>
        {
            await suggestions.WithdrawAsync(context.MarketplaceIdentity(), id, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("WithdrawSuggestion").ProducesProblem(403).ProducesProblem(404).ProducesProblem(409);

        authenticated.MapGet("/mine", async (HttpContext context, MarketplaceDbContext db, AccessService access, BundleService bundles, SuggestionService suggestions, EventsService events, CancellationToken cancellationToken) =>
        {
            var identity = context.MarketplaceIdentity();
            var namespaces = identity.Namespaces.ToArray();
            var packages = await db.Packages.AsNoTracking().Include(package => package.Versions)
                .Where(package => namespaces.Contains(package.Namespace))
                .OrderBy(package => package.Namespace).ThenBy(package => package.PackageId)
                .ToListAsync(cancellationToken);
            var publishers = await db.Publishers.AsNoTracking().Where(publisher => namespaces.Contains(publisher.Namespace)).ToDictionaryAsync(publisher => publisher.Namespace, cancellationToken);
            var rules = await access.RulesAsync(cancellationToken);
            var counts = await events.CountsAsync(cancellationToken);
            return TypedResults.Ok(new MineView(
                namespaces.Select(ns => SpaceView.From(identity, ns, publishers.GetValueOrDefault(ns), rules)).ToArray(),
                packages.Select(package => PackageView.From(package, identity, rules, publishers.GetValueOrDefault(package.Namespace), counts.GetValueOrDefault(package.CanonicalId))).ToArray(),
                await bundles.MineAsync(identity, cancellationToken),
                await suggestions.MineAsync(identity, cancellationToken)));
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

        authenticated.MapGet("/notifications", async (long? after, int? limit, HttpContext context, NotificationService notifications, CancellationToken cancellationToken) =>
            TypedResults.Ok(await notifications.ListAsync(context.MarketplaceIdentity(), after, limit, cancellationToken))).WithName("Notifications");

        authenticated.MapPost("/notifications/read", async Task<NoContent> (ReadRequest request, HttpContext context, NotificationService notifications, CancellationToken cancellationToken) =>
        {
            await notifications.ReadAsync(context.MarketplaceIdentity(), request.UpTo ?? long.MaxValue, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("ReadNotifications");

        authenticated.MapGet("/stats/packages/{ns}/{packageId}", async (string ns, string packageId, HttpContext context, AccessService access, EventsService events, CancellationToken cancellationToken) =>
        {
            var package = await access.VisiblePackageAsync(context.MarketplaceIdentity(), ns, packageId, cancellationToken);
            return TypedResults.Ok(await events.PackageStatsAsync(package.CanonicalId, cancellationToken));
        }).WithName("PackageStats").ProducesProblem(404);

        // A space and a package or bundle in it, as two routes so the API document lists both.
        authenticated.MapGet("/access/{ns}", async (string ns, HttpContext context, AccessService access, CancellationToken cancellationToken) =>
            TypedResults.Ok(await access.GetAsync(context.MarketplaceIdentity(), ns, null, cancellationToken)))
            .WithName("SpaceAccess").ProducesProblem(403).ProducesProblem(422);

        authenticated.MapGet("/access/{ns}/{id}", async (string ns, string id, HttpContext context, AccessService access, CancellationToken cancellationToken) =>
            TypedResults.Ok(await access.GetAsync(context.MarketplaceIdentity(), ns, id, cancellationToken)))
            .WithName("Access").ProducesProblem(403).ProducesProblem(422);

        authenticated.MapPut("/access/{ns}", async (string ns, ShareRequest request, HttpContext context, AccessService access, CancellationToken cancellationToken) =>
            TypedResults.Ok(await access.SetAsync(context.MarketplaceIdentity(), ns, null, request, cancellationToken)))
            .WithName("SetSpaceAccess").ProducesProblem(403).ProducesProblem(404).ProducesProblem(409).ProducesProblem(422);

        authenticated.MapPut("/access/{ns}/{id}", async (string ns, string id, ShareRequest request, HttpContext context, AccessService access, CancellationToken cancellationToken) =>
            TypedResults.Ok(await access.SetAsync(context.MarketplaceIdentity(), ns, id, request, cancellationToken)))
            .WithName("SetAccess").ProducesProblem(403).ProducesProblem(404).ProducesProblem(409).ProducesProblem(422);

        authenticated.MapPost("/access/{ns}/link", async (string ns, LinkRequest? request, HttpContext context, AccessService access, CancellationToken cancellationToken) =>
            TypedResults.Ok(new ShareLinkView(await access.ShareLinkAsync(context.MarketplaceIdentity(), ns, null, request?.Reset ?? false, cancellationToken))))
            .WithName("SpaceShareLink").ProducesProblem(403).ProducesProblem(422);

        authenticated.MapPost("/access/{ns}/{id}/link", async (string ns, string id, LinkRequest? request, HttpContext context, AccessService access, CancellationToken cancellationToken) =>
            TypedResults.Ok(new ShareLinkView(await access.ShareLinkAsync(context.MarketplaceIdentity(), ns, id, request?.Reset ?? false, cancellationToken))))
            .WithName("ShareLink").ProducesProblem(403).ProducesProblem(404).ProducesProblem(422);

        authenticated.MapPost("/teams", async Task<Created<TeamView>> (CreateTeamRequest request, HttpContext context, TeamService teams, CancellationToken cancellationToken) =>
        {
            var team = await teams.CreateAsync(context.MarketplaceIdentity(), request, cancellationToken);
            return TypedResults.Created($"/api/teams/{team.Namespace}", team);
        }).WithName("CreateTeam").ProducesProblem(409).ProducesProblem(422);

        authenticated.MapGet("/teams", async (HttpContext context, TeamService teams, CancellationToken cancellationToken) =>
            TypedResults.Ok(await teams.ListAsync(context.MarketplaceIdentity(), cancellationToken))).WithName("Teams");

        authenticated.MapGet("/teams/{ns}", async (string ns, HttpContext context, TeamService teams, CancellationToken cancellationToken) =>
            TypedResults.Ok(await teams.GetAsync(context.MarketplaceIdentity(), ns, cancellationToken)))
            .WithName("Team").ProducesProblem(404);

        authenticated.MapPut("/teams/{ns}", async (string ns, RenameTeamRequest request, HttpContext context, TeamService teams, CancellationToken cancellationToken) =>
            TypedResults.Ok(await teams.RenameAsync(context.MarketplaceIdentity(), ns, request, cancellationToken)))
            .WithName("RenameTeam").ProducesProblem(403).ProducesProblem(404).ProducesProblem(422);

        authenticated.MapPost("/teams/{ns}/members", async (string ns, MemberRequest request, HttpContext context, TeamService teams, CancellationToken cancellationToken) =>
            TypedResults.Ok(await teams.SetMemberAsync(context.MarketplaceIdentity(), ns, request, cancellationToken)))
            .WithName("SetTeamMember").ProducesProblem(403).ProducesProblem(404).ProducesProblem(409).ProducesProblem(422);

        authenticated.MapDelete("/teams/{ns}/members", async Task<NoContent> (string ns, string? account, HttpContext context, TeamService teams, CancellationToken cancellationToken) =>
        {
            await teams.RemoveMemberAsync(context.MarketplaceIdentity(), ns, account, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("RemoveTeamMember").ProducesProblem(403).ProducesProblem(404).ProducesProblem(409);

        authenticated.MapPost("/teams/{ns}/invite", async (string ns, LinkRequest? request, HttpContext context, TeamService teams, CancellationToken cancellationToken) =>
            TypedResults.Ok(await teams.InviteAsync(context.MarketplaceIdentity(), ns, request?.Reset ?? false, cancellationToken)))
            .WithName("TeamInvite").ProducesProblem(403).ProducesProblem(404);

        authenticated.MapDelete("/teams/{ns}", async Task<NoContent> (string ns, HttpContext context, TeamService teams, CancellationToken cancellationToken) =>
        {
            await teams.DeleteAsync(context.MarketplaceIdentity(), ns, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("DeleteTeam").ProducesProblem(403).ProducesProblem(404).ProducesProblem(409);

        authenticated.MapGet("/directory", async (string? q, HttpContext context, TeamService teams, CancellationToken cancellationToken) =>
            TypedResults.Ok(await teams.DirectoryAsync(context.MarketplaceIdentity(), q, cancellationToken))).WithName("Directory");

        authenticated.MapGet("/links/{code}", async (string code, TeamService teams, CancellationToken cancellationToken) =>
            TypedResults.Ok(await teams.PreviewLinkAsync(code, cancellationToken)))
            .WithName("Link").ProducesProblem(404);

        authenticated.MapPost("/links/{code}", async (string code, HttpContext context, TeamService teams, CancellationToken cancellationToken) =>
            TypedResults.Ok(await teams.RedeemLinkAsync(context.MarketplaceIdentity(), code, cancellationToken)))
            .WithName("RedeemLink").ProducesProblem(404);

        authenticated.MapPut("/bundles/{ns}/{id}", async (string ns, string id, BundleRequest request, HttpContext context, BundleService bundles, CancellationToken cancellationToken) =>
            TypedResults.Ok(await bundles.SaveAsync(context.MarketplaceIdentity(), ns, id, request, cancellationToken)))
            .WithName("SaveBundle").ProducesProblem(403).ProducesProblem(409).ProducesProblem(422);

        authenticated.MapGet("/bundles/{ns}/{id}", async (string ns, string id, HttpContext context, BundleService bundles, CancellationToken cancellationToken) =>
            TypedResults.Ok(await bundles.GetAsync(context.MarketplaceIdentity(), ns, id, cancellationToken)))
            .WithName("Bundle").ProducesProblem(404);

        authenticated.MapDelete("/bundles/{ns}/{id}", async Task<NoContent> (string ns, string id, HttpContext context, BundleService bundles, CancellationToken cancellationToken) =>
        {
            await bundles.DeleteAsync(context.MarketplaceIdentity(), ns, id, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("DeleteBundle").ProducesProblem(403).ProducesProblem(404);

        var admin = authenticated.MapGroup("/admin").RequireAuthorization(AdminPolicy);
        admin.MapGet("/summary", async (EventsService events, CancellationToken cancellationToken) =>
            TypedResults.Ok(await events.AdminSummaryAsync(cancellationToken))).WithName("AdminSummary");

        admin.MapGet("/reports", async (MarketplaceDbContext db, CancellationToken cancellationToken) =>
            TypedResults.Ok(await Reports(db.Reports.Where(report => report.Kind == PackageReport.Problem)).ToListAsync(cancellationToken)))
            .WithName("AdminReports");

        admin.MapPost("/reports/{id:long}/resolve", async Task<NoContent> (long id, HttpContext context, MarketplaceDbContext db, NotificationService notifications, TimeProvider time, CancellationToken cancellationToken) =>
        {
            await ResolveReportAsync(context.MarketplaceIdentity(), id, await NoteAsync(context.Request, cancellationToken), db, notifications, time, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("ResolveReport").ProducesProblem(404).ProducesProblem(422);

        admin.MapGet("/packages/{ns}/{packageId}/installs", async (string ns, string packageId, string? format, EventsService events, CancellationToken cancellationToken) =>
        {
            var installs = await events.InstallsAsync($"{ns}/{packageId}", cancellationToken);
            return format == "csv"
                ? Csv($"{ns}-{packageId}-installs.csv", [["account", "device", "version", "clientVersion", "lastSeenAt"]], installs.Select(install => new[] { install.Account, install.Device, install.Version ?? string.Empty, install.ClientVersion, install.LastSeenAt.ToString("O") }))
                : Results.Ok(installs);
        }).WithName("PackageInstalls").Produces<InstallView[]>(200);

        admin.MapGet("/audit", async (long? before, int? limit, string? format, MarketplaceDbContext db, CancellationToken cancellationToken) =>
        {
            var events = await db.AuditEvents.AsNoTracking()
                .Where(audit => before == null || audit.Id < before)
                .OrderByDescending(audit => audit.Id)
                .Take(Math.Clamp(limit ?? 100, 1, 1000))
                .Select(audit => new AuditView(audit.Id, audit.At, audit.Actor, audit.Action, audit.Target, audit.Detail))
                .ToListAsync(cancellationToken);
            return format == "csv"
                ? Csv("audit.csv", [["id", "at", "actor", "action", "target", "detail"]], events.Select(audit => new[] { audit.Id.ToString(System.Globalization.CultureInfo.InvariantCulture), audit.At.ToString("O"), audit.Actor, audit.Action, audit.Target, audit.Detail ?? string.Empty }))
                : Results.Ok(events);
        }).WithName("Audit").Produces<AuditView[]>(200);

        admin.MapGet("/blocks", async (MarketplaceDbContext db, CancellationToken cancellationToken) =>
            TypedResults.Ok(await db.Blocks.AsNoTracking().OrderBy(block => block.Account)
                .Select(block => new BlockView(block.Account, block.BlockedBy, block.BlockedAt, block.Reason))
                .ToListAsync(cancellationToken)))
            .WithName("Blocks");

        admin.MapPut("/blocks/{account}", async Task<NoContent> (string account, HttpContext context, MarketplaceDbContext db, TimeProvider time, CancellationToken cancellationToken) =>
        {
            account = account.Trim();
            var reason = context.Request.HasJsonContentType() ? (await context.Request.ReadFromJsonAsync<BlockRequest>(cancellationToken))?.Reason?.Trim() : null;
            if (account.Length is 0 or > AccessService.MaxEntryLength || reason is { Length: > 1024 })
            {
                throw new ProblemException(422, $"Name the account (up to {AccessService.MaxEntryLength} characters) and, optionally, a reason of up to 1,024.");
            }

            var identity = context.MarketplaceIdentity();
            var now = time.GetUtcNow().UtcDateTime;
            var block = await db.Blocks.FindAsync([account], cancellationToken) ?? db.Blocks.Add(new Block { Account = account, BlockedBy = identity.Account }).Entity;
            block.BlockedBy = identity.Account;
            block.BlockedAt = now;
            block.Reason = reason is { Length: > 0 } ? reason : null;
            db.Audit(identity.Account, "block", account, block.Reason, now);
            await db.SaveChangesAsync(cancellationToken);
            return TypedResults.NoContent();
        }).WithName("Block").ProducesProblem(422);

        admin.MapDelete("/blocks/{account}", async Task<NoContent> (string account, HttpContext context, MarketplaceDbContext db, TimeProvider time, CancellationToken cancellationToken) =>
        {
            var block = await db.Blocks.FindAsync([account.Trim()], cancellationToken) ?? throw ProblemException.NotFound($"A block on {account}");
            db.Blocks.Remove(block);
            db.Audit(context.MarketplaceIdentity().Account, "unblock", block.Account, null, time.GetUtcNow().UtcDateTime);
            await db.SaveChangesAsync(cancellationToken);
            return TypedResults.NoContent();
        }).WithName("Unblock").ProducesProblem(404);

        admin.MapDelete("/packages/{ns}/{packageId}/versions/{version}", async Task<NoContent> (string ns, string packageId, string version, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            await publish.PurgeAsync(context.MarketplaceIdentity(), ns, packageId, version, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("PurgeVersion").ProducesProblem(404).ProducesProblem(502);

        admin.MapPut("/namespaces/{ns}/owner", async Task<NoContent> (string ns, TransferRequest request, HttpContext context, TeamService teams, CancellationToken cancellationToken) =>
        {
            await teams.TransferAsync(context.MarketplaceIdentity(), ns, request, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("TransferNamespace").ProducesProblem(404).ProducesProblem(409).ProducesProblem(422);

        admin.MapGet("/reviews", async (PublishService publish, CancellationToken cancellationToken) =>
            TypedResults.Ok(await publish.PublicReviewsAsync(cancellationToken))).WithName("Reviews");

        admin.MapPost("/reviews/{ns}/{packageId}", async Task<NoContent> (string ns, string packageId, ReviewRequest review, HttpContext context, PublishService publish, CancellationToken cancellationToken) =>
        {
            if (review.Decision is not ("approve" or "decline"))
            {
                throw new ProblemException(422, "The decision is approve or decline.");
            }

            await publish.DecidePublicAsync(context.MarketplaceIdentity(), ns, packageId, review.Decision == "approve", review.Note, cancellationToken);
            return TypedResults.NoContent();
        }).WithName("Review").ProducesProblem(404).ProducesProblem(422);

        return app;
    }

    /// <summary><see cref="ArtifactStore"/> is <c>ok</c> or <c>error</c>; <see cref="Ldap"/> is <c>ok</c>, <c>error</c>, or <c>off</c> when no LDAP domain is set.</summary>
    public sealed record HealthView(string ServerVersion, string MinimumClientVersion, string LatestClientVersion, string Environment, IReadOnlyList<string> AuthSchemes, bool AdGroups, string ArtifactStore, string Ldap);

    private sealed record HealthProbe(DateTime At, string ArtifactStore, string Ldap);

    private static HealthProbe? _probe;

    /// <summary><see cref="App"/> is null when the account's desktop app has not reported for 30 days.</summary>
    public sealed record MeView(
        string Account,
        string Namespace,
        string DisplayName,
        IReadOnlyList<string> Namespaces,
        bool Admin,
        IReadOnlyList<string> Groups,
        IReadOnlyList<TeamMembership> Teams,
        int SuggestionsWaiting,
        AppView? App,
        int ReportsWaiting,
        int UnreadNotifications);

    /// <summary><see cref="Kind"/> is <c>problem</c> (the default) or <c>feedback</c>.</summary>
    public sealed record ReportRequest(string? Reason, string? Kind = null);

    public sealed record ReportView(long Id, string Account, string PackageId, string Kind, string Reason, DateTime CreatedAt, DateTime? ResolvedAt, string? ResolvedBy, string? Note);

    public sealed record ResolveRequest(string? Note);

    public sealed record AuditView(long Id, DateTime At, string Actor, string Action, string Target, string? Detail);

    public sealed record BlockView(string Account, string BlockedBy, DateTime BlockedAt, string? Reason);

    public sealed record BlockRequest(string? Reason);

    /// <summary>An admin's verdict on letting the public see a package's MCP server: <c>approve</c>, or <c>decline</c> with a note.</summary>
    public sealed record ReviewRequest(string Decision, string? Note);

    public sealed record LinkRequest(bool? Reset);

    public sealed record ShareLinkView(string Link);

    public sealed record PackageFile(string Path, long Size);

    /// <summary><see cref="Role"/> is <c>owner</c> or <c>member</c>: a person owns their own space, and admins own <c>official</c>.</summary>
    public sealed record SpaceView(string Namespace, string DisplayName, string Lane, Visibility Visibility, string Role)
    {
        public static SpaceView From(MarketplaceIdentity identity, string ns, Publisher? publisher, IReadOnlyDictionary<string, AccessRule> rules)
        {
            var lane = IdentityResolver.Lane(ns, publisher);
            var owner = lane switch
            {
                "team" => identity.IsTeamOwner(ns),
                "official" => identity.IsAdmin,
                _ => true,
            };
            var displayName = lane == "personal" ? identity.DisplayName : publisher?.DisplayName ?? "Official";
            return new SpaceView(ns, displayName, lane, AccessService.Effective(rules, ns, null).Private ? Visibility.Private : Visibility.Public, owner ? "owner" : "member");
        }
    }

    public sealed record MineView(SpaceView[] Spaces, PackageView[] Packages, BundleView[] Bundles, SuggestionsView Suggestions);

    public sealed record VersionView(
        string Version,
        string ArchiveDigest,
        long SizeBytes,
        string PublishedBy,
        DateTime PublishedAt,
        string? Changelog,
        bool Yanked,
        string[] ComponentKinds,
        bool Purged);

    public enum PublicReviewState
    {
        Approved,
        Declined,
        Waiting,
    }

    /// <summary>Whether the public may see the live version's MCP server; <see cref="Note"/> is an admin's reason for declining.</summary>
    public sealed record PublicReviewView(PublicReviewState State, string? Note);

    /// <summary>
    /// <see cref="PublicReview"/> is null unless the live version has an MCP server. <see cref="Installs"/> and
    /// <see cref="InstalledBase"/> count accounts; <see cref="McpServers"/> describes the live version's servers.
    /// </summary>
    public sealed record PackageView(
        string Id,
        string Namespace,
        string PackageId,
        string Name,
        string Description,
        string[] Tags,
        string? LiveVersion,
        VersionView[] Versions,
        bool Owned,
        Visibility Visibility,
        Visibility Effective,
        bool SharedWithYou,
        bool Revoked,
        PublicReviewView? PublicReview,
        bool RevokedByAdmin,
        DateTime CreatedAt,
        IndexPublisher Publisher,
        string Lane,
        int Installs,
        int InstalledBase,
        McpServerSummary[] McpServers)
    {
        public static PackageView From(Package package, MarketplaceIdentity identity, IReadOnlyDictionary<string, AccessRule> rules, Publisher? publisher, (int Installs, int InstalledBase) counts)
        {
            var live = PublishService.LatestVersion(package);
            return new PackageView(
                package.CanonicalId,
                package.Namespace,
                package.PackageId,
                package.Name,
                package.Description,
                package.Tags,
                live?.Version,
                package.Versions
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
                        version.PurgedAt is not null))
                    .ToArray(),
                identity.Owns(package.Namespace),
                rules.GetValueOrDefault(package.CanonicalId)?.Visibility ?? Visibility.Inherit,
                AccessService.Effective(rules, package.Namespace, package.PackageId).Private ? Visibility.Private : Visibility.Public,
                AccessService.SharedWith(rules, identity, package.Namespace, package.PackageId),
                package.RevokedAt is not null,
                live?.ComponentKinds.Contains(PublishService.McpServerKind) != true ? null
                    : package.McpApprovedBy is not null ? new PublicReviewView(PublicReviewState.Approved, null)
                    : package.McpDeclineNote is { } note ? new PublicReviewView(PublicReviewState.Declined, note)
                    : new PublicReviewView(PublicReviewState.Waiting, null),
                package.RevokedByAdmin,
                package.CreatedAt,
                new IndexPublisher(publisher?.Account ?? package.Namespace, publisher?.DisplayName ?? package.Namespace),
                IdentityResolver.Lane(package.Namespace, publisher),
                counts.Installs,
                counts.InstalledBase,
                McpServerSummary.FromJson(live?.McpServersJson));
        }
    }

    /// <summary>Reads a publish or suggestion form: an archive, or files with one paths value each, plus text fields.</summary>
    private static async Task<(IFormCollection Form, UploadRequest Upload)> ReadUploadAsync(HttpContext context, string ns, string packageId, CancellationToken cancellationToken)
    {
        if (!context.Request.HasFormContentType)
        {
            throw new ProblemException(415, "Upload with multipart/form-data: an archive, or files with one paths value each, plus the other fields.");
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

        return (form, new UploadRequest(
            ns,
            packageId,
            files.Select((upload, index) => (paths[index] ?? string.Empty, upload)).ToArray(),
            bytes,
            Optional(form["name"]),
            Optional(form["description"])));
    }

    /// <summary>A version and its stored zip, if the caller may read it: owners always, others only when it is live.</summary>
    private static async Task<(PackageVersion Version, byte[] Bytes)> ReadableArchiveAsync(HttpContext context, AccessService access, IArtifactStore store, string ns, string packageId, string versionText, CancellationToken cancellationToken)
    {
        var identity = context.MarketplaceIdentity();
        var package = await access.VisiblePackageAsync(identity, ns, packageId, cancellationToken);
        var version = package.Versions.SingleOrDefault(candidate => candidate.Version == versionText);
        if (version is null || version.PurgedAt is not null || !(identity.Owns(ns) || (!version.Yanked && package.RevokedAt is null)))
        {
            throw ProblemException.NotFound($"{ns}/{packageId} {versionText}");
        }

        return (version, await store.GetAsync(version.StoragePath, cancellationToken));
    }

    private static PackageFile[] ListFiles(byte[] archive)
    {
        using var zip = ArchiveInspector.OpenZip(new MemoryStream(archive, writable: false));
        var prefix = ArchiveInspector.RootPrefix(zip.Entries.Select(entry => entry.FullName).ToArray());
        return zip.Entries
            .Where(entry => entry.FullName.StartsWith(prefix, StringComparison.Ordinal) && !entry.FullName.EndsWith('/'))
            .Select(entry => new PackageFile(entry.FullName[prefix.Length..], entry.Length))
            .OrderBy(file => file.Path, StringComparer.Ordinal)
            .ToArray();
    }

    /// <summary>One file of an archive: UTF-8 text up to <see cref="MaxFileView"/> inline, anything else as a download.</summary>
    private static async Task<IResult> FileAsync(byte[] archive, string path, string label, CancellationToken cancellationToken)
    {
        // Not disposed: the archive only wraps a MemoryStream, and a large entry streams from it after this returns.
        var zip = ArchiveInspector.OpenZip(new MemoryStream(archive, writable: false));
        var prefix = ArchiveInspector.RootPrefix(zip.Entries.Select(entry => entry.FullName).ToArray());
        var entry = zip.GetEntry(prefix + path);
        if (entry is null || entry.FullName.EndsWith('/'))
        {
            throw ProblemException.NotFound(label);
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
    }

    /// <summary>Reports, open first, newest first.</summary>
    private static IQueryable<ReportView> Reports(IQueryable<PackageReport> reports) =>
        reports.AsNoTracking()
            .OrderBy(report => report.ResolvedAt != null)
            .ThenByDescending(report => report.CreatedAt)
            .Take(200)
            .Select(report => new ReportView(report.Id, report.Account, report.PackageId, report.Kind, report.Reason, report.CreatedAt, report.ResolvedAt, report.ResolvedBy, report.Note));

    /// <summary>The optional <c>{ note }</c> body of a resolve; a request with no body has none.</summary>
    private static async Task<string?> NoteAsync(HttpRequest request, CancellationToken cancellationToken) =>
        request.HasJsonContentType() ? (await request.ReadFromJsonAsync<ResolveRequest>(cancellationToken))?.Note : null;

    /// <summary>A package's owners, or an admin, resolve a report; the reporter hears about it.</summary>
    private static async Task ResolveReportAsync(MarketplaceIdentity identity, long id, string? note, MarketplaceDbContext db, NotificationService notifications, TimeProvider time, CancellationToken cancellationToken)
    {
        note = string.IsNullOrWhiteSpace(note) ? null : note.Trim();
        if (note is { Length: > MaxReportReason })
        {
            throw new ProblemException(422, $"A note is at most {MaxReportReason:N0} characters.");
        }

        var report = await db.Reports.FindAsync([id], cancellationToken);
        var (ns, packageId) = AccessService.Split(report?.PackageId ?? string.Empty);
        if (report is null || !identity.Owns(ns))
        {
            throw ProblemException.NotFound($"Report {id}");
        }

        // A problem report is also the admins' signal about a package; its owners must not be able to hide it.
        if (report.Kind == PackageReport.Problem && !identity.IsAdmin)
        {
            throw new ProblemException(403, "Only the marketplace admins close a problem report. Fix the package; they'll see the new version.");
        }

        if (report.ResolvedAt is null)
        {
            report.ResolvedAt = time.GetUtcNow().UtcDateTime;
            report.ResolvedBy = identity.Account;
            report.Note = note;
            var name = await db.Packages.Where(package => package.Namespace == ns && package.PackageId == packageId).Select(package => package.Name).SingleOrDefaultAsync(cancellationToken) ?? report.PackageId;
            notifications.Notify([report.Account], identity.Account, "report.resolved", note is null ? $"Your report about {name} was resolved." : $"Your report about {name} was resolved: {note}", $"/p/{report.PackageId}");
            db.Audit(identity.Account, "report.resolve", report.PackageId, note, report.ResolvedAt.Value);
            await db.SaveChangesAsync(cancellationToken);
        }
    }

    /// <summary>
    /// A 426 for a desktop app or CLI older than <c>Client:MinimumVersion</c>, from its
    /// <c>agent-plugins/x.y.z</c> User-Agent; null for anything else.
    /// </summary>
    private static ProblemException? TooOld(HttpRequest request, IServiceProvider services)
    {
        var agent = request.Headers.UserAgent.ToString().Split(' ').FirstOrDefault(token => token.StartsWith("agent-plugins/", StringComparison.Ordinal));
        var client = services.GetRequiredService<IOptions<ClientOptions>>().Value;
        if (agent is null || !SemVer.TryParse(agent["agent-plugins/".Length..], out var version) || !SemVer.TryParse(client.MinimumVersion, out var minimum) || version.CompareTo(minimum) >= 0)
        {
            return null;
        }

        var download = services.GetRequiredService<IOptions<ServerOptions>>().Value.PublicBaseUrl.TrimEnd('/') + "/#download";
        return new ProblemException(
            426,
            "Update Agent Plugins",
            detail: $"This version of Agent Plugins ({version}) is too old for the marketplace. Download {client.LatestVersion} from {download}.",
            extensions: new Dictionary<string, object?> { ["downloadUrl"] = download });
    }

    /// <summary>Binds to the LDAP domain the way Negotiate's group lookups do: <c>ok</c>, <c>error</c>, or <c>off</c>.</summary>
    private static string LdapStatus(AuthOptions auth, ILogger logger)
    {
        if (auth.LdapDomain is not { Length: > 0 } domain)
        {
            return "off";
        }

        try
        {
            var directory = new LdapDirectoryIdentifier(domain, fullyQualifiedDnsHostName: true, connectionless: false);
            using var connection = auth.LdapMachineAccountName is { Length: > 0 } name
                ? new LdapConnection(directory, new NetworkCredential(name, auth.LdapMachineAccountPassword))
                : new LdapConnection(directory);
            connection.SessionOptions.ProtocolVersion = 3;
            connection.Timeout = TimeSpan.FromSeconds(5);
            connection.Bind();
            return "ok";
        }
        catch (Exception error)
        {
            // Anything from DNS to a missing libldap means group lookups fail the same way.
            logger.LogWarning(error, "The LDAP health check could not bind to {Domain}.", domain);
            return "error";
        }
    }

    /// <summary>A CSV download; cells that a spreadsheet would run as a formula are quoted as text.</summary>
    private static IResult Csv(string fileName, IEnumerable<string[]> header, IEnumerable<string[]> rows)
    {
        static string Cell(string value)
        {
            if (value.Length > 0 && "=+-@\t\r".Contains(value[0]))
            {
                value = "'" + value;
            }

            return value.IndexOfAny([',', '"', '\n', '\r']) >= 0 ? $"\"{value.Replace("\"", "\"\"")}\"" : value;
        }

        var text = string.Concat(header.Concat(rows).Select(row => string.Join(',', row.Select(Cell)) + "\r\n"));
        return Results.File(Encoding.UTF8.GetBytes(text), "text/csv; charset=utf-8", fileName);
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
    private static readonly TimeSpan PersonRefresh = TimeSpan.FromDays(1);

    // ponytail: per-process memory of when each account's People row was last written; a restart writes each once more.
    private static readonly ConcurrentDictionary<string, DateTime> Touched = new(StringComparer.Ordinal);

    /// <summary>Resolves the caller once per request, including the database lookups that settle their personal namespace and teams.</summary>
    public static async Task ResolveMarketplaceIdentityAsync(this HttpContext context)
    {
        var options = context.RequestServices.GetRequiredService<IOptions<AuthOptions>>().Value;
        var db = context.RequestServices.GetRequiredService<MarketplaceDbContext>();
        var identity = await IdentityResolver.SettleAsync(IdentityResolver.Resolve(context.User, options), db, context.RequestAborted);
        context.Items[Key] = identity;

        var now = context.RequestServices.GetRequiredService<TimeProvider>().GetUtcNow().UtcDateTime;
        var key = $"{db.Database.GetConnectionString()}\n{identity.Account}";
        if (!Touched.TryGetValue(key, out var touched) || now - touched > PersonRefresh)
        {
            var displayName = identity.DisplayName.Length <= 120 ? identity.DisplayName : identity.DisplayName[..120];
            await db.Database.ExecuteSqlAsync(
                $"""
                INSERT INTO "People" ("Account", "DisplayName", "LastSeenAt") VALUES ({identity.Account}, {displayName}, {now})
                ON CONFLICT ("Account") DO UPDATE SET "DisplayName" = EXCLUDED."DisplayName", "LastSeenAt" = EXCLUDED."LastSeenAt"
                """,
                context.RequestAborted);
            Touched[key] = now;
        }
    }

    public static MarketplaceIdentity MarketplaceIdentity(this HttpContext context) =>
        context.Items.TryGetValue(Key, out var cached) && cached is MarketplaceIdentity identity
            ? identity
            : throw new InvalidOperationException("The caller's identity is resolved by the authenticated route group's filter, which did not run.");
}
