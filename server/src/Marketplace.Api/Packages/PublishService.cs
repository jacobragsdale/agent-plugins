using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Marketplace.Api.Access;
using Marketplace.Api.Auth;
using Marketplace.Api.Data;
using Marketplace.Api.Storage;
using Microsoft.EntityFrameworkCore;

namespace Marketplace.Api.Packages;

public sealed record PublishRequest(
    string Namespace,
    string PackageId,
    string Version,
    string[] Tags,
    string? Changelog,
    ReadOnlyMemory<byte> Archive);

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

/// <summary><see cref="WaitingForPublicReview"/>: everyone but the namespace's owners sees it once an admin approves its MCP server.</summary>
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
    bool WaitingForPublicReview);

/// <summary>A public package whose MCP server waits for an admin before anyone outside its namespace sees it.</summary>
public sealed record PublicReview(
    string Id,
    string Namespace,
    string PackageId,
    string Name,
    string Version,
    string PublishedBy,
    DateTime PublishedAt,
    string? Changelog,
    string[] ComponentKinds);

/// <summary>
/// Publishing, withdrawing, and revoking. Every publish goes live at once; the one gate left is an admin
/// letting the public see a package's MCP server, which is visibility, not a version state (ADR 0007).
/// </summary>
public sealed partial class PublishService(
    MarketplaceDbContext db,
    IArtifactStore store,
    IPackageValidator validator,
    AccessService access,
    TimeProvider timeProvider,
    ILogger<PublishService> logger)
{
    public const int MaxTags = 10;
    public const int MaxReviewNote = 2048;
    public const int MaxUploadFiles = 2000;
    public const int MaxChangelog = 4096;
    public const int MaxTagLength = 32;
    public const string McpServerKind = "mcpServer";

    public async Task<PublishedVersion> PublishAsync(MarketplaceIdentity identity, PublishRequest request, CancellationToken cancellationToken)
    {
        CheckTarget(identity, request.Namespace, request.PackageId);
        var semver = ParseVersion(request.Version);
        var tags = NormalizeTags(request.Tags);
        if (request.Changelog is { Length: > MaxChangelog } changelog)
        {
            throw new ProblemException(422, $"The changelog is at most {MaxChangelog:N0} characters; this one has {changelog.Length:N0}.");
        }

        var inspected = await CheckArchiveAsync(request.Namespace, request.PackageId, request.Archive, cancellationToken);
        return await CommitAsync(identity, identity.Account, request.Namespace, request.PackageId, semver, tags, request.Changelog, request.Archive, inspected, cancellationToken);
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
    public async Task<PublishedVersion> CommitAsync(
        MarketplaceIdentity identity,
        string publishedBy,
        string ns,
        string packageId,
        SemVer semver,
        string[] tags,
        string? changelog,
        ReadOnlyMemory<byte> archive,
        InspectedArchive inspected,
        CancellationToken cancellationToken)
    {
        var now = timeProvider.GetUtcNow().UtcDateTime;
        var storagePath = $"{ns}/{packageId}/{semver}.zip";
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);

        var package = await db.Packages
            .Include(candidate => candidate.Versions)
            .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken);
        if (package?.Versions.Any(existing => existing.Version == semver.ToString()) == true)
        {
            throw new ProblemException(409, $"{ns}/{packageId} {semver} was already published, and version numbers are never reused (even after a withdrawal). Publish {NextPatch(package)} or later.");
        }

        if (package is null && await db.Bundles.AnyAsync(bundle => bundle.Namespace == ns && bundle.BundleId == packageId, cancellationToken))
        {
            throw new ProblemException(409, $"{ns} already has a bundle called {packageId}. Pick another name.");
        }

        var publisher = await ClaimPublisherAsync(identity, ns, now, cancellationToken);
        var stored = await StoreAsync(storagePath, archive, $"{ns}/{packageId} {semver}", cancellationToken);
        var isNew = package is null;
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
        };
        package.Versions.Add(version);

        // A new version is a new request to the public: a decline does not carry over. An admin's own MCP server needs nobody.
        package.McpDeclineNote = null;
        if (identity.IsAdmin && package.McpApprovedBy is null && version.ComponentKinds.Contains(McpServerKind))
        {
            package.McpApprovedBy = identity.Account;
            package.McpApprovedAt = now;
        }

        if (isNew || LatestVersion(package) == version)
        {
            ApplyListing(package, version, now);
        }

        publisher.LastPublishedAt = now;
        await db.SaveChangesAsync(cancellationToken);
        await RegenerateNamespaceAsync(ns, new Dictionary<string, ReadOnlyMemory<byte>> { [storagePath] = archive }, cancellationToken);
        await transaction.CommitAsync(cancellationToken);

        var waiting = Gated(package) && !AccessService.Effective(await access.RulesAsync(cancellationToken), ns, packageId).Private;
        logger.LogInformation("{Account} published {Namespace}/{Package} {Version} for {PublishedBy}.", identity.Account, ns, packageId, semver, publishedBy);
        return new PublishedVersion(package.CanonicalId, ns, packageId, version.Version, version.ArchiveDigest, version.SizeBytes, version.PublishedBy, version.PublishedAt, version.ComponentKinds, package.Tags, waiting);
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

    /// <summary>Public packages whose MCP server waits for an admin, oldest first.</summary>
    public async Task<List<PublicReview>> PublicReviewsAsync(CancellationToken cancellationToken)
    {
        var packages = await db.Packages.AsNoTracking()
            .Include(package => package.Versions.Where(version => !version.Yanked))
            .Where(package => package.RevokedAt == null && package.McpApprovedBy == null && package.McpDeclineNote == null)
            .ToListAsync(cancellationToken);
        var rules = await access.RulesAsync(cancellationToken);
        return packages
            .Where(package => Gated(package) && !AccessService.Effective(rules, package.Namespace, package.PackageId).Private)
            .Select(package => (package, live: LatestVersion(package)!))
            .Select(pair => new PublicReview(pair.package.CanonicalId, pair.package.Namespace, pair.package.PackageId, pair.package.Name, pair.live.Version, pair.live.PublishedBy, pair.live.PublishedAt, pair.live.Changelog, pair.live.ComponentKinds))
            .OrderBy(review => review.PublishedAt)
            .ToList();
    }

    /// <summary>Lets the public see a package's MCP server, or keeps it from them with a note the owners see.</summary>
    public async Task DecidePublicAsync(MarketplaceIdentity identity, string ns, string packageId, bool approve, string? note, CancellationToken cancellationToken)
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

        var package = await db.Packages.Include(candidate => candidate.Versions)
            .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken);
        if (package is null || LatestVersion(package)?.ComponentKinds.Contains(McpServerKind) != true)
        {
            throw ProblemException.NotFound($"An MCP server in {ns}/{packageId}");
        }

        package.McpApprovedBy = approve ? identity.Account : null;
        package.McpApprovedAt = approve ? timeProvider.GetUtcNow().UtcDateTime : null;
        package.McpDeclineNote = approve ? null : note;
        await db.SaveChangesAsync(cancellationToken);
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
        if (package.RevokedAt is not null != revoked)
        {
            package.RevokedAt = revoked ? timeProvider.GetUtcNow().UtcDateTime : null;
            package.RevokedBy = revoked ? identity.Account : null;
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
            await RegenerateNamespaceAsync(ns, new Dictionary<string, ReadOnlyMemory<byte>>(), cancellationToken);
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
        if (version.Yanked != yanked)
        {
            var now = timeProvider.GetUtcNow().UtcDateTime;
            version.Yanked = yanked;
            if (LatestVersion(package) is { } live)
            {
                ApplyListing(package, live, now);
            }

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

    /// <summary>The live version: the highest one not yanked, unless the package was revoked.</summary>
    public static PackageVersion? LatestVersion(Package package) =>
        package.RevokedAt is not null
            ? null
            : package.Versions.Where(version => !version.Yanked).MaxBy(version => SemVer.Parse(version.Version));

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
