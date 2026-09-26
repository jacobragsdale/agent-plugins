using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Marketplace.Api.Auth;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Marketplace.Api.Storage;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Options;

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
    ReviewState ReviewState);

/// <summary>A pending version waiting for an admin, with what the reviewer needs to judge it.</summary>
public sealed record PendingReview(
    string Id,
    string Namespace,
    string PackageId,
    string Name,
    string Version,
    string PublishedBy,
    DateTime PublishedAt,
    string? Changelog,
    string[] ComponentKinds,
    bool FirstVersion,
    string? LiveVersion);

public sealed partial class PublishService(
    MarketplaceDbContext db,
    IArtifactStore store,
    IPackageValidator validator,
    IOptions<AuthOptions> auth,
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

        if (!SemVer.TryParse(request.Version, out var semver))
        {
            throw new ProblemException(422, request.Version.Contains('-')
                ? $"{request.Version} is a pre-release. The marketplace takes release versions only (major.minor.patch, for example 1.2.0)."
                : $"{request.Version} is not a version number. Use major.minor.patch, for example 1.2.0.");
        }

        var tags = NormalizeTags(request.Tags);
        if (request.Changelog is { Length: > MaxChangelog } changelog)
        {
            throw new ProblemException(422, $"The changelog is at most {MaxChangelog:N0} characters; this one has {changelog.Length:N0}.");
        }

        var inspected = ArchiveInspector.Inspect(request.Archive);

        if (inspected.SourceId != request.Namespace)
        {
            throw new ProblemException(422, $"agent-plugins.json declares source.id {inspected.SourceId}; the namespace is {request.Namespace}.");
        }

        if (inspected.PackageId != request.PackageId)
        {
            throw new ProblemException(422, $"agent-plugins.json declares package {inspected.PackageId}; the request names {request.PackageId}.");
        }

        var outcome = await ValidateAsync(request.Archive, inspected.RootPrefix, cancellationToken);
        if (!outcome.Accepted)
        {
            var errors = outcome.Errors.Count > 0 ? outcome.Errors : [new ValidationError("", "The package has no valid install.")];
            throw new ProblemException(422, "The package failed validation.", errors);
        }

        var now = timeProvider.GetUtcNow().UtcDateTime;
        var storagePath = $"{request.Namespace}/{request.PackageId}/{semver}.zip";
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await LockNamespaceAsync(request.Namespace, cancellationToken);

        var package = await db.Packages
            .Include(candidate => candidate.Versions)
            .SingleOrDefaultAsync(candidate => candidate.Namespace == request.Namespace && candidate.PackageId == request.PackageId, cancellationToken);
        if (package?.Versions.Any(existing => existing.Version == semver.ToString()) == true)
        {
            var highest = package.Versions.Select(existing => SemVer.Parse(existing.Version)).Max()!;
            var next = highest with { Patch = highest.Patch + 1 };
            throw new ProblemException(409, $"{request.Namespace}/{request.PackageId} {semver} was already published, and version numbers are never reused (even after a yank or a rejection). Publish {next} or later.");
        }

        var publisher = await ClaimPublisherAsync(identity, request.Namespace, now, cancellationToken);
        var stored = await StoreAsync(storagePath, request.Archive, $"{request.Namespace}/{request.PackageId} {semver}", cancellationToken);

        // A package's first version and every version with an MCP server wait for an admin (ADR 0006).
        var approved = identity.IsAdmin || (package is not null && LatestVersion(package) is not null && !inspected.ComponentKinds.Contains(McpServerKind));
        var isNew = package is null;
        package ??= db.Packages.Add(new Package
        {
            Namespace = request.Namespace,
            PackageId = request.PackageId,
            Name = request.PackageId,
            Description = request.PackageId,
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
            PublishedBy = identity.Account,
            PublishedAt = now,
            Changelog = request.Changelog is { Length: > 0 } text ? text : null,
            ReviewState = approved ? ReviewState.Approved : ReviewState.Pending,
            ReviewedBy = approved ? identity.Account : null,
            ReviewedAt = approved ? now : null,
        };
        package.Versions.Add(version);

        // The listing follows the live version; a brand-new package takes its pending version's so the owner sees a name.
        if (isNew || LatestVersion(package) == version)
        {
            ApplyListing(package, version, now);
        }

        publisher.LastPublishedAt = now;
        await db.SaveChangesAsync(cancellationToken);
        if (approved)
        {
            await RegenerateNamespaceAsync(request.Namespace, new Dictionary<string, ReadOnlyMemory<byte>> { [storagePath] = request.Archive }, cancellationToken);
        }

        await transaction.CommitAsync(cancellationToken);
        logger.LogInformation("{Account} published {Namespace}/{Package} {Version} ({State}).", identity.Account, request.Namespace, request.PackageId, semver, version.ReviewState);
        return ToPublished(package, version);
    }

    /// <summary>
    /// Turns an upload into a publishable source zip. A zip with a manifest passes through; anything
    /// else goes through <c>validate-source stage</c>, the same wrapping the CLI does.
    /// </summary>
    public async Task<byte[]> PrepareUploadAsync(MarketplaceIdentity identity, UploadRequest upload, CancellationToken cancellationToken)
    {
        CheckTarget(identity, upload.Namespace, upload.PackageId);
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

    /// <summary>The review queue, oldest first.</summary>
    public async Task<List<PendingReview>> PendingAsync(CancellationToken cancellationToken)
    {
        var packages = await db.Packages.AsNoTracking()
            .Include(package => package.Versions)
            .Where(package => package.Versions.Any(version => version.ReviewState == ReviewState.Pending && !version.Yanked))
            .ToListAsync(cancellationToken);
        return packages
            .SelectMany(package => package.Versions
                .Where(version => version.ReviewState == ReviewState.Pending && !version.Yanked)
                .Select(version => new PendingReview(
                    package.CanonicalId,
                    package.Namespace,
                    package.PackageId,
                    package.Name,
                    version.Version,
                    version.PublishedBy,
                    version.PublishedAt,
                    version.Changelog,
                    version.ComponentKinds,
                    !package.Versions.Any(other => other.ReviewState == ReviewState.Approved),
                    LatestVersion(package)?.Version)))
            .OrderBy(review => review.PublishedAt)
            .ToList();
    }

    /// <summary>Approves or rejects a pending version. Approval makes it live and rebuilds the namespace archive.</summary>
    public async Task ReviewAsync(MarketplaceIdentity identity, string ns, string packageId, string versionText, bool approve, string? note, CancellationToken cancellationToken)
    {
        note = string.IsNullOrWhiteSpace(note) ? null : note.Trim();
        if (!approve && note is null)
        {
            throw new ProblemException(422, "Say why the version is rejected; the publisher sees the note.");
        }

        if (note is { Length: > MaxReviewNote })
        {
            throw new ProblemException(422, $"A review note is at most {MaxReviewNote:N0} characters; this one has {note.Length:N0}.");
        }

        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await LockNamespaceAsync(ns, cancellationToken);
        var (package, version) = await FindVersionAsync(ns, packageId, versionText, cancellationToken);
        if (version.Yanked || version.ReviewState != ReviewState.Pending)
        {
            var state = version.Yanked ? "yanked" : version.ReviewState == ReviewState.Approved ? "already approved" : "already rejected";
            throw new ProblemException(409, $"{package.CanonicalId} {version.Version} is not waiting for review: it is {state}.");
        }

        var now = timeProvider.GetUtcNow().UtcDateTime;
        version.ReviewState = approve ? ReviewState.Approved : ReviewState.Rejected;
        version.ReviewedBy = identity.Account;
        version.ReviewedAt = now;
        version.ReviewNote = note;
        if (approve && LatestVersion(package) == version)
        {
            ApplyListing(package, version, now);
        }

        await db.SaveChangesAsync(cancellationToken);
        if (approve)
        {
            await RegenerateNamespaceAsync(ns, new Dictionary<string, ReadOnlyMemory<byte>>(), cancellationToken);
        }

        await transaction.CommitAsync(cancellationToken);
        logger.LogInformation("{Account} {Decision} {Package} {Version}.", identity.Account, approve ? "approved" : "rejected", package.CanonicalId, version.Version);
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
        await LockNamespaceAsync(ns, cancellationToken);
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
            logger.LogInformation("{Account} {Action} {Package} {Version}.", identity.Account, yanked ? "yanked" : "restored", package.CanonicalId, version.Version);
        }

        await transaction.CommitAsync(cancellationToken);
    }

    /// <summary>Rebuilds the namespace archive from the latest non-yanked version of each package.</summary>
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

    /// <summary>The live version: the highest one that is approved and not yanked.</summary>
    public static PackageVersion? LatestVersion(Package package) =>
        package.Versions
            .Where(version => !version.Yanked && version.ReviewState == ReviewState.Approved)
            .Select(version => (version, semver: SemVer.Parse(version.Version)))
            .OrderByDescending(pair => pair.semver)
            .Select(pair => pair.version)
            .FirstOrDefault();

    public static PublishedVersion ToPublished(Package package, PackageVersion version) => new(
        package.CanonicalId,
        package.Namespace,
        package.PackageId,
        version.Version,
        version.ArchiveDigest,
        version.SizeBytes,
        version.PublishedBy,
        version.PublishedAt,
        version.ComponentKinds,
        package.Tags,
        version.ReviewState);

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

    private static void CheckTarget(MarketplaceIdentity identity, string ns, string packageId)
    {
        if (!identity.Owns(ns))
        {
            throw ProblemException.NotOwner(identity.Account, ns);
        }

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
    /// The namespace's publisher row, created on its first publish. Official and team namespaces are
    /// credited to the namespace itself. A personal namespace is claimed by the account that first
    /// publishes to it, so only its owner may create it, not an admin publishing on their behalf.
    /// </summary>
    private async Task<Publisher> ClaimPublisherAsync(MarketplaceIdentity identity, string ns, DateTime now, CancellationToken cancellationToken)
    {
        var personal = IdentityResolver.Lane(auth.Value, ns) == "personal";
        var publisher = await db.Publishers.FindAsync([ns], cancellationToken);
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
                FirstSeenAt = now,
            }).Entity;
        }
        else if (personal && !identity.IsAdmin && !IdentityResolver.IsClaimant(publisher.Account, identity.Account))
        {
            // Two accounts that derive one name can both see it free before either publishes; the lock settles who got it.
            throw new ProblemException(409, $"{ns} was just claimed by another account. Reload to get your own namespace, then publish there.");
        }

        // Keep the display name current, but an admin publishing into someone's namespace is not its owner.
        if (!personal || ns == identity.Namespace)
        {
            var displayName = IdentityResolver.NamespaceDisplayName(auth.Value, identity, ns);
            publisher.DisplayName = displayName.Length <= 120 ? displayName : displayName[..120];
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

    private async Task LockNamespaceAsync(string ns, CancellationToken cancellationToken) =>
        await db.Database.ExecuteSqlAsync($"SELECT pg_advisory_xact_lock(hashtext({ns}))", cancellationToken);

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
