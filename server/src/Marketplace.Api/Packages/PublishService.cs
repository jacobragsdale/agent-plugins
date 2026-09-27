using System.Security.Cryptography;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Marketplace.Api.Access;
using Marketplace.Api.Auth;
using Marketplace.Api.Data;
using Marketplace.Api.Events;
using Marketplace.Api.Notifications;
using Marketplace.Api.Storage;
using Microsoft.EntityFrameworkCore;

namespace Marketplace.Api.Packages;

/// <summary><see cref="Visibility"/>, when given, becomes the package's own rule in the same transaction.</summary>
public sealed record PublishRequest(
    string Namespace,
    string PackageId,
    string Version,
    string[] Tags,
    string? Changelog,
    ReadOnlyMemory<byte> Archive,
    Visibility? Visibility = null);

/// <summary>
/// A browser upload: files with their relative paths, or a zip. Anything without a manifest is
/// wrapped into one package.
/// </summary>
public sealed record UploadRequest(
    string Namespace,
    string PackageId,
    IReadOnlyList<(string Path, IFormFile File)> Files,
    byte[]? Archive,
    string? Name,
    string? Description);

/// <summary>
/// <see cref="WaitingForPublicReview"/>: everyone but the namespace's owners sees it once an admin approves its MCP server.
/// <see cref="Warnings"/> are sentences the publisher should read, such as who stops seeing the package meanwhile.
/// </summary>
public sealed record PublishedVersion(
    string Id,
    string Namespace,
    string PackageId,
    string Version,
    string ArchiveDigest,
    long SizeBytes,
    string PublishedBy,
    DateTime PublishedAt,
    string[] ComponentKinds,
    string[] Tags,
    bool WaitingForPublicReview,
    string[] Warnings);

/// <summary><see cref="Created"/> is false for a retry that found the same version already published.</summary>
public sealed record CommitResult(PublishedVersion Version, bool Created);

/// <summary>A file of a dry run compared with the live version: <c>new</c>, <c>changed</c>, <c>removed</c>, or <c>same</c>.</summary>
public sealed record DryRunFile(string Path, string Status);

/// <summary>Everything a publish would do, checked and not stored.</summary>
public sealed record DryRunView(string Version, string[] ComponentKinds, long SizeBytes, string[] Warnings, DryRunFile[] Files);

/// <summary>A package whose MCP server waits for an admin before the public, or a broad share list, sees it.</summary>
public sealed record PublicReview(
    string Id,
    string Namespace,
    string PackageId,
    string Name,
    string Version,
    string PublishedBy,
    DateTime PublishedAt,
    string? Changelog,
    string[] ComponentKinds,
    McpServerSummary[] McpServers,
    string Audience,
    int InstalledBase);

/// <summary>
/// Publishing, withdrawing, and revoking. Every publish goes live at once; the one gate left is an admin
/// letting the public see a package's MCP server, which is visibility, not a version state (ADR 0007).
/// </summary>
public sealed partial class PublishService(
    MarketplaceDbContext db,
    IArtifactStore store,
    IPackageValidator validator,
    AccessService access,
    EventsService events,
    NotificationService notifications,
    TimeProvider timeProvider,
    ILogger<PublishService> logger)
{
    public const int MaxTags = 10;
    public const int MaxReviewNote = 2048;
    public const int MaxUploadFiles = ArchiveInspector.MaxFiles;
    public const int MaxChangelog = 4096;
    public const int MaxTagLength = 32;
    public const string McpServerKind = "mcpServer";

    public async Task<CommitResult> PublishAsync(MarketplaceIdentity identity, PublishRequest request, CancellationToken cancellationToken)
    {
        var (semver, tags, inspected) = await CheckRequestAsync(identity, request, cancellationToken);
        return (await CommitCoreAsync(identity, identity.Account, request.Namespace, request.PackageId, semver, tags, request.Changelog, request.Archive, inspected, request.Visibility, dryRun: false, cancellationToken)).Result;
    }

    /// <summary>Runs every check a publish would, including the namespace size limits, and stores nothing.</summary>
    public async Task<DryRunView> DryRunAsync(MarketplaceIdentity identity, PublishRequest request, CancellationToken cancellationToken)
    {
        var (semver, tags, inspected) = await CheckRequestAsync(identity, request, cancellationToken);
        var (result, previous) = await CommitCoreAsync(identity, identity.Account, request.Namespace, request.PackageId, semver, tags, request.Changelog, request.Archive, inspected, request.Visibility, dryRun: true, cancellationToken);
        var proposed = ArchiveInspector.ReadFiles(request.Archive.ToArray());
        var live = previous is null ? [] : ArchiveInspector.ReadFiles(await store.GetAsync(previous.StoragePath, cancellationToken));
        var files = proposed
            .Select(file => new DryRunFile(file.Key, !live.TryGetValue(file.Key, out var current) ? "new" : current.AsSpan().SequenceEqual(file.Value) ? "same" : "changed"))
            .Concat(live.Keys.Where(path => !proposed.ContainsKey(path)).Select(path => new DryRunFile(path, "removed")))
            .OrderBy(file => file.Path, StringComparer.Ordinal)
            .ToArray();
        return new DryRunView(result.Version.Version, result.Version.ComponentKinds, request.Archive.Length, result.Version.Warnings, files);
    }

    private async Task<(SemVer Semver, string[] Tags, InspectedArchive Inspected)> CheckRequestAsync(MarketplaceIdentity identity, PublishRequest request, CancellationToken cancellationToken)
    {
        CheckTarget(identity, request.Namespace, request.PackageId);
        var semver = ParseVersion(request.Version);
        var tags = NormalizeTags(request.Tags);
        if (request.Changelog is { Length: > MaxChangelog } changelog)
        {
            throw new ProblemException(422, $"The changelog is at most {MaxChangelog:N0} characters; this one has {changelog.Length:N0}.");
        }

        return (semver, tags, await CheckArchiveAsync(request.Namespace, request.PackageId, request.Archive, cancellationToken));
    }

    /// <summary>Inspects and validates a source zip for <c>ns/packageId</c>: its manifest must name them, and the Rust validator must pass it.</summary>
    public async Task<InspectedArchive> CheckArchiveAsync(string ns, string packageId, ReadOnlyMemory<byte> archive, CancellationToken cancellationToken)
    {
        var inspected = ArchiveInspector.Inspect(archive);
        if (inspected.SourceId != ns)
        {
            throw new ProblemException(422, $"agent-plugins.json declares source.id {inspected.SourceId}; the namespace is {ns}.");
        }

        if (inspected.PackageId != packageId)
        {
            throw new ProblemException(422, $"agent-plugins.json declares package {inspected.PackageId}; the request names {packageId}.");
        }

        var outcome = await ValidateAsync(archive, inspected.RootPrefix, cancellationToken);
        if (!outcome.Accepted)
        {
            var errors = outcome.Errors.Count > 0 ? outcome.Errors : [new ValidationError("", "The package failed validation.")];
            throw new ProblemException(422, "The package failed validation.", errors);
        }

        return inspected;
    }

    /// <summary>
    /// Stores a checked archive as a new version and makes it live. <paramref name="identity"/> is who acts
    /// (the owner, or an owner accepting a suggestion); <paramref name="publishedBy"/> is who it is credited to.
    /// </summary>
    public async Task<CommitResult> CommitAsync(
        MarketplaceIdentity identity,
        string publishedBy,
        string ns,
        string packageId,
        SemVer semver,
        string[] tags,
        string? changelog,
        ReadOnlyMemory<byte> archive,
        InspectedArchive inspected,
        CancellationToken cancellationToken) =>
        (await CommitCoreAsync(identity, publishedBy, ns, packageId, semver, tags, changelog, archive, inspected, null, dryRun: false, cancellationToken)).Result;

    /// <summary>
    /// The publish itself. A dry run does every step, the namespace rebuild included, then rolls the transaction
    /// back without storing the archive. Also returns the version that was live before, for a dry run's comparison.
    /// </summary>
    private async Task<(CommitResult Result, PackageVersion? Previous)> CommitCoreAsync(
        MarketplaceIdentity identity,
        string publishedBy,
        string ns,
        string packageId,
        SemVer semver,
        string[] tags,
        string? changelog,
        ReadOnlyMemory<byte> archive,
        InspectedArchive inspected,
        Visibility? visibility,
        bool dryRun,
        CancellationToken cancellationToken)
    {
        var now = timeProvider.GetUtcNow().UtcDateTime;
        var storagePath = $"{ns}/{packageId}/{semver}.zip";
        var digest = Convert.ToHexStringLower(SHA256.HashData(archive.Span));
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);

        var package = await db.Packages
            .Include(candidate => candidate.Versions)
            .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken);
        if (package?.Versions.SingleOrDefault(existing => existing.Version == semver.ToString()) is { } published)
        {
            // A retry of a publish that already went through, such as a CLI timeout after the server committed.
            if (!dryRun && published.ArchiveDigest == digest && published.PurgedAt is null)
            {
                return (new CommitResult(View(package, published, waiting: false, []), Created: false), null);
            }

            throw VersionConflict($"{ns}/{packageId} {semver} was already published, and version numbers are never reused (even after a withdrawal).", package);
        }

        if (package?.RevokedAt is not null)
        {
            throw new ProblemException(409, package.RevokedByAdmin
                ? "An admin removed this package from every PC. Ask the marketplace admins to restore it before publishing a new version."
                : "This package was removed from every PC. Restore it before publishing a new version.");
        }

        var previous = package is null ? null : LatestVersion(package);
        if (previous is not null && semver.CompareTo(SemVer.Parse(previous.Version)) < 0)
        {
            throw VersionConflict($"{ns}/{packageId} {semver} is lower than the live version {previous.Version}, so no PC would get it.", package!);
        }

        if (package is null && await db.Bundles.AnyAsync(bundle => bundle.Namespace == ns && bundle.BundleId == packageId, cancellationToken))
        {
            throw new ProblemException(409, $"{ns} already has a bundle called {packageId}. Pick another name.");
        }

        // Who could see it before, for the warning when this publish hides it behind the MCP gate.
        var rules = await access.RulesAsync(cancellationToken);
        var waitingBefore = package is not null && Gated(package) && AccessService.NeedsReview(rules, ns, packageId);

        var publisher = await ClaimPublisherAsync(identity, ns, now, cancellationToken);
        var stored = dryRun ? new StoredArtifact(storagePath, digest, archive.Length) : await StoreAsync(storagePath, archive, $"{ns}/{packageId} {semver}", cancellationToken);
        var isNew = package is null;
        if (isNew)
        {
            // A freed id starts over: installs of a package an admin deleted under this id aren't this one's.
            var canonicalId = $"{ns}/{packageId}";
            await db.Events.Where(item => item.PackageId == canonicalId).ExecuteDeleteAsync(cancellationToken);
        }

        package ??= db.Packages.Add(new Package
        {
            Namespace = ns,
            PackageId = packageId,
            Name = packageId,
            Description = packageId,
            CreatedAt = now,
        }).Entity;

        var version = new PackageVersion
        {
            Package = package,
            Version = semver.ToString(),
            StoragePath = storagePath,
            ArchiveDigest = stored.Sha256,
            SizeBytes = stored.SizeBytes,
            ManifestJson = inspected.PackageManifest.ToJsonString(),
            ComponentKinds = inspected.ComponentKinds.ToArray(),
            Tags = tags,
            PublishedBy = publishedBy,
            PublishedAt = now,
            Changelog = changelog is { Length: > 0 } text ? text : null,
            McpServersJson = McpServerSummary.ToJson(inspected.McpServers),
        };
        package.Versions.Add(version);

        // A new version is a new request to the public: a decline does not carry over. An approval covers what the
        // servers launch; a version that launches something else waits again. An admin's own MCP server needs nobody.
        package.McpDeclineNote = null;
        var spec = McpServerSummary.LaunchSpec(version.McpServersJson);
        var requeued = false;
        if (version.ComponentKinds.Contains(McpServerKind) && package.McpApprovedSpec != spec)
        {
            requeued = package.McpApprovedBy is not null && !identity.IsAdmin;
            package.McpApprovedBy = identity.IsAdmin ? identity.Account : null;
            package.McpApprovedAt = identity.IsAdmin ? now : null;
            package.McpApprovedSpec = identity.IsAdmin ? spec : null;
        }

        if (isNew || LatestVersion(package) == version)
        {
            ApplyListing(package, version, now);
        }

        if (visibility is { } chosen)
        {
            await access.ApplyVisibilityAsync(package.CanonicalId, chosen, identity.Account, now, cancellationToken);
        }

        publisher.LastPublishedAt = now;
        db.Audit(identity.Account, "publish", package.CanonicalId, publishedBy == identity.Account ? version.Version : $"{version.Version}, credited to {publishedBy}", now);
        await db.SaveChangesAsync(cancellationToken);
        await RegenerateNamespaceAsync(ns, new Dictionary<string, ReadOnlyMemory<byte>> { [storagePath] = archive }, cancellationToken);

        var waiting = Gated(package) && AccessService.NeedsReview(await access.RulesAsync(cancellationToken), ns, packageId);
        string[] warnings = !waiting ? []
            : requeued ? [$"Its MCP server now runs something different, so an admin has to approve it again. Until then only its owners see {package.Name}; PCs that have it keep the version they have."]
            : previous is not null && !waitingBefore ? [$"Only its owners see {package.Name} until an admin approves its MCP server; PCs that have it keep the version they have."]
            : [$"Only its owners see {package.Name} until an admin approves its MCP server."];
        if (dryRun)
        {
            await transaction.RollbackAsync(cancellationToken);
            return (new CommitResult(View(package, version, waiting, warnings), Created: true), previous);
        }

        await transaction.CommitAsync(cancellationToken);
        if (waiting && !waitingBefore)
        {
            notifications.Webhook($"{package.Name} ({package.CanonicalId}) has an MCP server waiting for an admin before the public sees it.", "/admin");
        }

        logger.LogInformation("{Account} published {Namespace}/{Package} {Version} for {PublishedBy}.", identity.Account, ns, packageId, semver, publishedBy);
        return (new CommitResult(View(package, version, waiting, warnings), Created: true), previous);
    }

    private static PublishedVersion View(Package package, PackageVersion version, bool waiting, string[] warnings) =>
        new(package.CanonicalId, package.Namespace, package.PackageId, version.Version, version.ArchiveDigest, version.SizeBytes, version.PublishedBy, version.PublishedAt, version.ComponentKinds, package.Tags, waiting, warnings);

    private static ProblemException VersionConflict(string message, Package package)
    {
        var next = NextPatch(package).ToString();
        return new ProblemException(409, $"{message} Publish {next} or later.", extensions: new Dictionary<string, object?> { ["suggestedVersion"] = next });
    }

    /// <summary>
    /// Turns an upload into a publishable source zip. A zip with a manifest passes through; anything
    /// else goes through <c>validate-source stage</c>, the same wrapping the CLI does. The caller checks
    /// who may upload to the target.
    /// </summary>
    public async Task<byte[]> PrepareUploadAsync(UploadRequest upload, CancellationToken cancellationToken)
    {
        CheckIds(upload.Namespace, upload.PackageId);
        if (upload.Files.Count > MaxUploadFiles)
        {
            throw new ProblemException(422, $"An upload holds at most {MaxUploadFiles:N0} files; this one has {upload.Files.Count:N0}.");
        }

        var directory = Path.Combine(Path.GetTempPath(), "marketplace-upload-" + Guid.NewGuid().ToString("N"));
        var input = Path.Combine(directory, "input");
        try
        {
            if (upload.Archive is { } archive)
            {
                var (prefix, hasManifest) = ArchiveInspector.Survey(archive);
                if (hasManifest && upload.Files.Count == 0)
                {
                    return archive;
                }

                ArchiveInspector.ExtractTo(archive, prefix, input);
            }

            // A chosen folder arrives as folder/..., so strip a shared top directory.
            var strip = ArchiveInspector.RootPrefix(upload.Files.Select(file => file.Path).ToArray());
            var root = Path.GetFullPath(input) + Path.DirectorySeparatorChar;
            var written = new List<string>(upload.Files.Count);
            foreach (var (path, file) in upload.Files)
            {
                ArchiveInspector.SafePath(path);
                var target = Path.GetFullPath(Path.Combine(root, path[strip.Length..]));
                if (!target.StartsWith(root, StringComparison.Ordinal) || target.Length == root.Length)
                {
                    throw new ProblemException(422, $"{path} is not a file path inside the upload.");
                }

                Directory.CreateDirectory(Path.GetDirectoryName(target)!);
                await using var source = file.OpenReadStream();
                await using var destination = File.Create(target);
                await source.CopyToAsync(destination, cancellationToken);
                written.Add(target);
            }

            // One uploaded .json is an MCP document to wrap on its own.
            var stageInput = written is [var only] && upload.Archive is null && only.EndsWith(".json", StringComparison.OrdinalIgnoreCase) ? only : input;
            var output = Path.Combine(directory, "source.zip");
            await validator.StageAsync(new StagingRequest(stageInput, output, upload.Namespace, upload.PackageId, upload.Name, upload.Description), cancellationToken);
            return await File.ReadAllBytesAsync(output, cancellationToken);
        }
        finally
        {
            DeleteDirectory(directory);
        }
    }

    /// <summary>Packages whose MCP server waits for an admin before the public or a broad share list sees it, oldest first.</summary>
    public async Task<List<PublicReview>> PublicReviewsAsync(CancellationToken cancellationToken)
    {
        var packages = await db.Packages.AsNoTracking()
            .Include(package => package.Versions.Where(version => !version.Yanked))
            .Where(package => package.RevokedAt == null && package.McpApprovedBy == null && package.McpDeclineNote == null)
            .ToListAsync(cancellationToken);
        var rules = await access.RulesAsync(cancellationToken);
        var counts = await events.CountsAsync(cancellationToken);
        return packages
            .Where(package => Gated(package) && AccessService.NeedsReview(rules, package.Namespace, package.PackageId))
            .Select(package => (package, live: LatestVersion(package)!))
            .Select(pair => new PublicReview(
                pair.package.CanonicalId,
                pair.package.Namespace,
                pair.package.PackageId,
                pair.package.Name,
                pair.live.Version,
                pair.live.PublishedBy,
                pair.live.PublishedAt,
                pair.live.Changelog,
                pair.live.ComponentKinds,
                McpServerSummary.FromJson(pair.live.McpServersJson),
                AccessService.Audience(rules, pair.package.Namespace, pair.package.PackageId),
                counts.GetValueOrDefault(pair.package.CanonicalId).InstalledBase))
            .OrderBy(review => review.PublishedAt)
            .ToList();
    }

    /// <summary>
    /// Lets the public see a package's MCP server, or keeps it from them with a note the owners see. An approval
    /// names the <paramref name="version"/> the admin reviewed, so it never covers one published after they looked.
    /// </summary>
    public async Task DecidePublicAsync(MarketplaceIdentity identity, string ns, string packageId, bool approve, string? note, string? version, CancellationToken cancellationToken)
    {
        note = string.IsNullOrWhiteSpace(note) ? null : note.Trim();
        if (!approve && note is null)
        {
            throw new ProblemException(422, "Say why everyone may not see it yet; its owners see the note.");
        }

        if (note is { Length: > MaxReviewNote })
        {
            throw new ProblemException(422, $"A review note is at most {MaxReviewNote:N0} characters; this one has {note.Length:N0}.");
        }

        if (approve && string.IsNullOrWhiteSpace(version))
        {
            throw new ProblemException(422, "Say which version you reviewed, so the approval covers only what you saw.");
        }

        // A publish takes the same lock, so the version checked here is the one approved.
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);
        var package = await db.Packages.Include(candidate => candidate.Versions)
            .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken);
        var live = package is null ? null : LatestVersion(package);
        if (package is null || live?.ComponentKinds.Contains(McpServerKind) != true)
        {
            throw ProblemException.NotFound($"An MCP server in {ns}/{packageId}");
        }

        if (approve && live.Version != version)
        {
            throw new ProblemException(409, $"{package.Name} {live.Version} is live now, not the {version} you reviewed. Look at {live.Version} before approving it.");
        }

        var now = timeProvider.GetUtcNow().UtcDateTime;
        package.McpApprovedBy = approve ? identity.Account : null;
        package.McpApprovedAt = approve ? now : null;
        package.McpApprovedSpec = approve ? McpServerSummary.LaunchSpec(live.McpServersJson) : null;
        package.McpDeclineNote = approve ? null : note;
        db.Audit(identity.Account, approve ? "review.approve" : "review.decline", package.CanonicalId, note, now);
        notifications.Notify(
            await notifications.OwnersAsync(ns, cancellationToken),
            identity.Account,
            "review.decided",
            approve ? $"An admin approved the MCP server in {package.Name}. Everyone who can see the package can install it now." : $"An admin kept {package.Name} from the public: {note}",
            $"/p/{ns}/{packageId}");
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        logger.LogInformation("{Account} {Decision} the MCP server in {Package} for everyone.", identity.Account, approve ? "approved" : "declined", package.CanonicalId);
    }

    /// <summary>
    /// Pulls a package from every PC, or puts it back. A revoked package leaves the catalog, index, and
    /// archive, and clients uninstall it; restoring does not reinstall it anywhere.
    /// </summary>
    public async Task SetRevokedAsync(MarketplaceIdentity identity, string ns, string packageId, bool revoked, CancellationToken cancellationToken)
    {
        if (!identity.Owns(ns))
        {
            throw ProblemException.NotOwner(identity.Account, ns);
        }

        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);
        var package = await db.Packages.SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken)
            ?? throw ProblemException.NotFound($"The package {ns}/{packageId}");
        if (!revoked && package.RevokedByAdmin && !identity.IsAdmin)
        {
            throw new ProblemException(403, "An admin removed this package from every PC, so only an admin can restore it.");
        }

        // An admin revoking what its owners already revoked takes the revoke over, so they can't restore it.
        var adminTakesOver = revoked && identity.IsAdmin && package.RevokedAt is not null && !package.RevokedByAdmin;
        if (package.RevokedAt is not null != revoked || adminTakesOver)
        {
            var now = timeProvider.GetUtcNow().UtcDateTime;
            package.RevokedAt = revoked ? now : null;
            package.RevokedBy = revoked ? identity.Account : null;
            package.RevokedByAdmin = revoked && identity.IsAdmin;
            db.Audit(identity.Account, revoked ? "revoke" : "restore", package.CanonicalId, null, now);
            if (revoked && identity.IsAdmin)
            {
                notifications.Notify(await notifications.OwnersAsync(ns, cancellationToken), identity.Account, "package.revoked", $"An admin removed {package.Name} from every PC.", $"/p/{ns}/{packageId}");
            }

            await db.SaveChangesAsync(cancellationToken);
            await RegenerateNamespaceAsync(ns, new Dictionary<string, ReadOnlyMemory<byte>>(), cancellationToken);
            logger.LogInformation("{Account} {Action} {Package}.", identity.Account, revoked ? "revoked" : "restored", package.CanonicalId);
        }

        await transaction.CommitAsync(cancellationToken);
    }

    /// <summary>Rebuilds every namespace archive, after a migration changed which versions are live.</summary>
    public async Task RegenerateAllAsync(CancellationToken cancellationToken)
    {
        var namespaces = await db.Packages.Select(package => package.Namespace)
            .Union(db.NamespaceArchives.Select(archive => archive.Namespace))
            .ToListAsync(cancellationToken);
        foreach (var ns in namespaces)
        {
            try
            {
                await RegenerateNamespaceAsync(ns, new Dictionary<string, ReadOnlyMemory<byte>>(), cancellationToken);
            }
            catch (ProblemException problem)
            {
                logger.LogWarning("Kept the stored archive of {Namespace}: {Problem}", ns, problem.Title);
            }
        }
    }

    /// <summary>
    /// Yanks or restores a version. A yanked version leaves the namespace archive and index; existing
    /// installations keep it. The listing follows whichever version is live afterwards.
    /// </summary>
    public async Task SetYankedAsync(MarketplaceIdentity identity, string ns, string packageId, string versionText, bool yanked, CancellationToken cancellationToken)
    {
        if (!identity.Owns(ns))
        {
            throw ProblemException.NotOwner(identity.Account, ns);
        }

        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);
        var (package, version) = await FindVersionAsync(ns, packageId, versionText, cancellationToken);
        if (!yanked && version.PurgedAt is not null)
        {
            throw new ProblemException(409, $"{ns}/{packageId} {version.Version} was purged, so it can't come back. Publish a new version.");
        }

        if (version.Yanked != yanked)
        {
            var now = timeProvider.GetUtcNow().UtcDateTime;
            version.Yanked = yanked;
            if (LatestVersion(package) is { } live)
            {
                ApplyListing(package, live, now);
            }

            ReviewAgainIfLiveChanged(package);

            db.Audit(identity.Account, yanked ? "yank" : "unyank", package.CanonicalId, version.Version, now);
            await db.SaveChangesAsync(cancellationToken);
            await RegenerateNamespaceAsync(ns, new Dictionary<string, ReadOnlyMemory<byte>>(), cancellationToken);
            logger.LogInformation("{Account} {Action} {Package} {Version}.", identity.Account, yanked ? "withdrew" : "restored", package.CanonicalId, version.Version);
        }

        await transaction.CommitAsync(cancellationToken);
    }

    /// <summary>Rebuilds the namespace archive from the live version of each package.</summary>
    public async Task RegenerateNamespaceAsync(string ns, IReadOnlyDictionary<string, ReadOnlyMemory<byte>> knownArchives, CancellationToken cancellationToken)
    {
        var packages = await db.Packages
            .Include(package => package.Versions)
            .Where(package => package.Namespace == ns)
            .ToListAsync(cancellationToken);
        var publisher = await db.Publishers.FindAsync([ns], cancellationToken);
        var latest = packages
            .Select(package => (package, version: LatestVersion(package)))
            .Where(pair => pair.version is not null)
            .ToArray();
        var existing = await db.NamespaceArchives.FindAsync([ns], cancellationToken);
        if (latest.Length == 0)
        {
            if (existing is not null)
            {
                db.NamespaceArchives.Remove(existing);
                await db.SaveChangesAsync(cancellationToken);
            }

            return;
        }

        var sources = new List<NamespacePackage>(latest.Length);
        foreach (var (package, version) in latest)
        {
            var bytes = knownArchives.TryGetValue(version!.StoragePath, out var known)
                ? known
                : await store.GetAsync(version.StoragePath, cancellationToken);
            var manifest = JsonNode.Parse(version.ManifestJson) as JsonObject
                ?? throw new InvalidOperationException($"Stored manifest for {package.CanonicalId} {version.Version} is not an object.");
            var prefix = ArchiveInspector.Inspect(bytes).RootPrefix;
            sources.Add(new NamespacePackage(package.PackageId, manifest, bytes, prefix));
        }

        var displayName = publisher?.DisplayName ?? ns;
        var built = NamespaceArchiveBuilder.Build(
            new NamespaceSource(ns, displayName, $"Packages published by {displayName}."),
            sources);
        var tooBig = built.Bytes.Length > ArchiveInspector.MaxArchiveBytes || built.Files > ArchiveInspector.MaxFiles || built.UncompressedBytes > ArchiveInspector.MaxUncompressedBytes;
        // Only a change that makes the namespace bigger is refused: a namespace built under older, larger
        // limits must still let people withdraw, revoke, delete, and purge what is in it.
        if (tooBig && (existing is null || built.Bytes.Length > existing.Bytes.Length))
        {
            throw new ProblemException(422, $"Everything in {displayName} together would be too big for the desktop app to download: {built.Files:N0} files, {Megabytes(built.UncompressedBytes)} unzipped, {Megabytes(built.Bytes.Length)} zipped. "
                + $"The limits are {ArchiveInspector.MaxFiles:N0} files and {Megabytes(ArchiveInspector.MaxUncompressedBytes)} unzipped or zipped. Remove files, or publish this package in another space.");
        }
        var now = timeProvider.GetUtcNow().UtcDateTime;
        if (existing is null)
        {
            db.NamespaceArchives.Add(new NamespaceArchive
            {
                Namespace = ns,
                Bytes = built.Bytes,
                Digest = built.Digest,
                PackageCount = built.PackageCount,
                GeneratedAt = now,
            });
        }
        else if (existing.Digest != built.Digest)
        {
            existing.Bytes = built.Bytes;
            existing.Digest = built.Digest;
            existing.PackageCount = built.PackageCount;
            existing.GeneratedAt = now;
        }
        else
        {
            existing.PackageCount = built.PackageCount;
        }

        await db.SaveChangesAsync(cancellationToken);
    }

    /// <summary>
    /// Deletes a package and frees its id. Its owners may while nobody ever installed it; admins always.
    /// Stored archives go last and best effort: a leftover is harmless, and a retry with the same bytes adopts it.
    /// </summary>
    public async Task DeleteAsync(MarketplaceIdentity identity, string ns, string packageId, CancellationToken cancellationToken)
    {
        if (!identity.Owns(ns))
        {
            throw ProblemException.NotOwner(identity.Account, ns);
        }

        var target = $"{ns}/{packageId}";
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);
        var package = await db.Packages.Include(candidate => candidate.Versions)
            .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken)
            ?? throw ProblemException.NotFound($"The package {ns}/{packageId}");
        if (package.RevokedByAdmin && !identity.IsAdmin)
        {
            // Deleting would free the id for a fresh, unreviewed republish.
            throw new ProblemException(403, $"An admin removed {package.Name} from every PC. Ask the marketplace admins to delete it.");
        }

        if (!identity.IsAdmin && await events.EverInstalledAsync(target, cancellationToken))
        {
            throw new ProblemException(409, $"People have installed {package.Name}, so it can't be deleted. Use Remove from every PC instead.");
        }

        // Deleting takes its reports along, and a problem report is the admins' to close.
        if (!identity.IsAdmin && await db.Reports.AnyAsync(report => report.PackageId == target && report.Kind == PackageReport.Problem && report.ResolvedAt == null, cancellationToken))
        {
            throw new ProblemException(409, $"Someone reported a problem with {package.Name} that the marketplace admins haven't closed yet, so it can't be deleted. Ask the admins to look at the report.");
        }

        var now = timeProvider.GetUtcNow().UtcDateTime;
        var paths = package.Versions.Select(version => version.StoragePath)
            .Concat(await db.Suggestions.Where(suggestion => suggestion.PackageId == package.Id && suggestion.StoragePath != "").Select(suggestion => suggestion.StoragePath).ToListAsync(cancellationToken))
            .ToArray();
        db.Packages.Remove(package);
        await db.AccessRules.Where(rule => rule.Target == target).ExecuteDeleteAsync(cancellationToken);
        await db.Links.Where(link => link.Target == target).ExecuteDeleteAsync(cancellationToken);
        await db.Reports.Where(report => report.PackageId == target).ExecuteDeleteAsync(cancellationToken);
        foreach (var bundle in await db.Bundles.Where(bundle => bundle.Members.Contains(target)).ToListAsync(cancellationToken))
        {
            bundle.Members = bundle.Members.Where(member => member != target).ToArray();
            if (bundle.Members.Length == 0)
            {
                // A bundle holds at least one package, so its last one takes it along.
                var bundleId = bundle.CanonicalId;
                db.Bundles.Remove(bundle);
                await db.AccessRules.Where(rule => rule.Target == bundleId).ExecuteDeleteAsync(cancellationToken);
                await db.Links.Where(link => link.Target == bundleId).ExecuteDeleteAsync(cancellationToken);
                db.Audit(identity.Account, "bundle.delete", bundleId, $"its last package, {target}, was deleted", now);
            }
        }

        db.Audit(identity.Account, "delete", target, $"{package.Versions.Count} versions", now);
        await db.SaveChangesAsync(cancellationToken);
        await RegenerateNamespaceAsync(ns, new Dictionary<string, ReadOnlyMemory<byte>>(), cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        // Committed: a client that hangs up now must not leave the archives behind.
        foreach (var path in paths)
        {
            try
            {
                await store.DeleteAsync(path, CancellationToken.None);
            }
            catch (ProblemException problem)
            {
                logger.LogWarning("Deleted {Package} but left {Path} in the package store: {Problem}", target, path, problem.Title);
            }
        }
    }

    /// <summary>
    /// An admin deletes a version's stored archive for good, for a leaked secret say: the version is withdrawn and
    /// marked purged in the database first, so a failed store delete can simply be retried.
    /// </summary>
    public async Task PurgeAsync(MarketplaceIdentity identity, string ns, string packageId, string versionText, CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);
        var (package, version) = await FindVersionAsync(ns, packageId, versionText, cancellationToken);
        if (version.PurgedAt is null)
        {
            var now = timeProvider.GetUtcNow().UtcDateTime;
            version.Yanked = true;
            version.PurgedAt = now;
            version.PurgedBy = identity.Account;
            if (LatestVersion(package) is { } live)
            {
                ApplyListing(package, live, now);
            }

            ReviewAgainIfLiveChanged(package);

            db.Audit(identity.Account, "purge", package.CanonicalId, version.Version, now);
            await db.SaveChangesAsync(cancellationToken);
            await RegenerateNamespaceAsync(ns, new Dictionary<string, ReadOnlyMemory<byte>>(), cancellationToken);
        }

        await transaction.CommitAsync(cancellationToken);
        // Committed: a client that hangs up now must not leave the archives behind.
        var suggestions = await db.Suggestions.AsNoTracking()
            .Where(suggestion => suggestion.PackageId == package.Id && suggestion.AcceptedVersion == version.Version && suggestion.StoragePath != "")
            .Select(suggestion => suggestion.StoragePath)
            .ToListAsync(CancellationToken.None);
        foreach (var path in suggestions.Prepend(version.StoragePath))
        {
            await store.DeleteAsync(path, CancellationToken.None);
        }

        logger.LogWarning("{Account} purged {Package} {Version}.", identity.Account, package.CanonicalId, version.Version);
    }

    /// <summary>The live version: the highest one not yanked, unless the package was revoked.</summary>
    public static PackageVersion? LatestVersion(Package package) =>
        package.RevokedAt is not null
            ? null
            : package.Versions.Where(version => !version.Yanked).MaxBy(version => SemVer.Parse(version.Version));

    /// <summary>
    /// A withdraw, restore, or purge can make another version live. An approval covers what its servers launch,
    /// as on publish, so a live version that launches something else (one declined, or never reviewed) waits again.
    /// </summary>
    private static void ReviewAgainIfLiveChanged(Package package)
    {
        if (package.McpApprovedBy is not null
            && LatestVersion(package) is { } live
            && live.ComponentKinds.Contains(McpServerKind)
            && package.McpApprovedSpec != McpServerSummary.LaunchSpec(live.McpServersJson))
        {
            package.McpApprovedBy = null;
            package.McpApprovedAt = null;
            package.McpApprovedSpec = null;
        }
    }

    /// <summary>The live version has an MCP server no admin has let the public see.</summary>
    public static bool Gated(Package package) =>
        package.McpApprovedBy is null && LatestVersion(package)?.ComponentKinds.Contains(McpServerKind) == true;

    /// <summary>The patch after the highest version ever used, the suggested next version.</summary>
    public static SemVer NextPatch(Package? package) =>
        package?.Versions.Select(version => SemVer.Parse(version.Version)).Max() is { } highest
            ? highest with { Patch = highest.Patch + 1 }
            : new SemVer(1, 0, 0);

    public static SemVer ParseVersion(string text) =>
        SemVer.TryParse(text, out var semver)
            ? semver
            : throw new ProblemException(422, text.Contains('-')
                ? $"{text} is a pre-release. The marketplace takes release versions only (major.minor.patch, for example 1.2.0)."
                : $"{text} is not a version number. Use major.minor.patch, for example 1.2.0.");

    /// <summary>Lowercases and deduplicates tags; an invalid tag or too many is a 422, never silently dropped.</summary>
    public static string[] NormalizeTags(IEnumerable<string> tags)
    {
        var normalized = tags.Select(tag => tag.Trim().ToLowerInvariant()).Where(tag => tag.Length > 0).Distinct(StringComparer.Ordinal).ToArray();
        if (normalized.FirstOrDefault(tag => tag.Length > MaxTagLength || !TagPattern().IsMatch(tag)) is { } invalid)
        {
            throw new ProblemException(422, $"The tag \"{invalid}\" is not valid. A tag is up to {MaxTagLength} lowercase letters, digits, and single hyphens.");
        }

        if (normalized.Length > MaxTags)
        {
            throw new ProblemException(422, $"A version has at most {MaxTags} tags; this one has {normalized.Length}.");
        }

        return normalized;
    }

    private async Task<ValidationOutcome> ValidateAsync(ReadOnlyMemory<byte> archive, string rootPrefix, CancellationToken cancellationToken)
    {
        var directory = Path.Combine(Path.GetTempPath(), "marketplace-validate-" + Guid.NewGuid().ToString("N"));
        try
        {
            ArchiveInspector.ExtractTo(archive, rootPrefix, directory);
            return await validator.ValidateAsync(directory, cancellationToken);
        }
        finally
        {
            DeleteDirectory(directory);
        }
    }

    private static void DeleteDirectory(string directory)
    {
        try
        {
            Directory.Delete(directory, recursive: true);
        }
        catch (DirectoryNotFoundException)
        {
        }
        catch (IOException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }
    }

    public static void CheckTarget(MarketplaceIdentity identity, string ns, string packageId)
    {
        if (!identity.Owns(ns))
        {
            throw ProblemException.NotOwner(identity.Account, ns);
        }

        CheckIds(ns, packageId);
    }

    private static void CheckIds(string ns, string packageId)
    {
        if (!IdentityResolver.SourceIdPattern().IsMatch(ns))
        {
            throw new ProblemException(422, $"{ns} is not a valid namespace.");
        }

        if (!PackageIdPattern().IsMatch(packageId))
        {
            throw new ProblemException(422, $"{packageId} is not a valid package id.");
        }
    }

    private async Task<(Package Package, PackageVersion Version)> FindVersionAsync(string ns, string packageId, string versionText, CancellationToken cancellationToken)
    {
        var package = await db.Packages
            .Include(candidate => candidate.Versions)
            .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken);
        var version = package?.Versions.SingleOrDefault(candidate => candidate.Version == versionText);
        return package is not null && version is not null ? (package, version) : throw ProblemException.NotFound($"{ns}/{packageId} {versionText}");
    }

    /// <summary>
    /// The namespace's publisher row, created on its first publish (a team's exists from its creation).
    /// <c>official</c> is credited to the namespace itself. A personal namespace is claimed by the account
    /// that first publishes to it, so only its owner may create it, not an admin publishing on their behalf.
    /// Call it inside a transaction holding the namespace lock.
    /// </summary>
    public async Task<Publisher> ClaimPublisherAsync(MarketplaceIdentity identity, string ns, DateTime now, CancellationToken cancellationToken)
    {
        var publisher = await db.Publishers.FindAsync([ns], cancellationToken);
        var personal = publisher?.Kind == PublisherKind.Personal || (publisher is null && ns != MarketplaceIdentity.OfficialNamespace);
        if (publisher is null)
        {
            if (personal && ns != identity.Namespace)
            {
                throw new ProblemException(403, $"{ns} is a personal namespace its owner has not published to yet; only they can create it.");
            }

            publisher = db.Publishers.Add(new Publisher
            {
                Namespace = ns,
                Account = personal ? identity.Account : ns,
                DisplayName = string.Empty,
                Kind = personal ? PublisherKind.Personal : PublisherKind.Official,
                FirstSeenAt = now,
            }).Entity;
        }
        else if (personal && !identity.IsAdmin && !IdentityResolver.IsClaimant(publisher.Account, identity.Account))
        {
            // Two accounts that derive one name can both see it free before either publishes; the lock settles who got it.
            throw new ProblemException(409, $"{ns} was just claimed by another account. Reload to get your own namespace, then publish there.");
        }

        // Keep a person's display name current, but an admin publishing into someone's namespace is not its owner. A team keeps its own name.
        if (publisher.Kind == PublisherKind.Official)
        {
            publisher.DisplayName = "Official";
        }
        else if (personal && ns == identity.Namespace)
        {
            publisher.DisplayName = identity.DisplayName.Length <= 120 ? identity.DisplayName : identity.DisplayName[..120];
        }

        return publisher;
    }

    /// <summary>
    /// Stores a version's archive. The store is immutable and outside the database transaction, so a
    /// publish that failed after its upload leaves the blob behind; a retry with the same bytes adopts it.
    /// </summary>
    private async Task<StoredArtifact> StoreAsync(string storagePath, ReadOnlyMemory<byte> archive, string label, CancellationToken cancellationToken)
    {
        try
        {
            return await store.PutAsync(storagePath, archive, cancellationToken);
        }
        catch (ArtifactConflictException)
        {
            var existing = await store.GetAsync(storagePath, cancellationToken);
            if (!existing.AsSpan().SequenceEqual(archive.Span))
            {
                throw new ProblemException(409, $"{label} is already in the package store with different content, left by an earlier publish that did not finish. Publish a new version number.");
            }

            logger.LogWarning("Adopted the stored archive at {Path} left by an unfinished publish.", storagePath);
            return new StoredArtifact(storagePath, Convert.ToHexStringLower(System.Security.Cryptography.SHA256.HashData(archive.Span)), archive.Length);
        }
    }

    private static string Megabytes(long bytes) => $"{bytes / 1024.0 / 1024.0:0.#} MB";

    /// <summary>Copies a version's name, description, and tags onto the package listing.</summary>
    private static void ApplyListing(Package package, PackageVersion version, DateTime now)
    {
        var manifest = JsonNode.Parse(version.ManifestJson) as JsonObject;
        package.Name = manifest?["name"]?.GetValue<string>() ?? package.PackageId;
        package.Description = manifest?["description"]?.GetValue<string>()
            ?? (version.ComponentKinds.Length == 0 ? null : $"{string.Join(", ", version.ComponentKinds)} package.")
            ?? package.Name;
        package.Tags = version.Tags.Length > 0 ? version.Tags : package.Tags;
        package.UpdatedAt = now;
    }

    [GeneratedRegex("^[a-z0-9](?:[a-z0-9]|-(?=[a-z0-9])){0,63}$")]
    public static partial Regex PackageIdPattern();

    [GeneratedRegex("^[a-z0-9](?:[a-z0-9]|-(?=[a-z0-9]))*$")]
    private static partial Regex TagPattern();
}
