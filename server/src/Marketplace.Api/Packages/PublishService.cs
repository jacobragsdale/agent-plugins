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

public sealed class PublishRejectedException(int status, string title, IReadOnlyList<ValidationError>? errors = null) : Exception(title)
{
    public int Status { get; } = status;

    public string Title { get; } = title;

    public IReadOnlyList<ValidationError> Errors { get; } = errors ?? [];
}

public sealed record PublishRequest(
    string Namespace,
    string PackageId,
    string Version,
    string[] Tags,
    string? Changelog,
    ReadOnlyMemory<byte> Archive);

/// <summary>
/// A browser upload: files with their relative paths (optionally laid over a published
/// <see cref="BaseVersion"/>), or a zip. Anything without a manifest is wrapped into one package.
/// </summary>
public sealed record UploadRequest(
    string Namespace,
    string PackageId,
    IReadOnlyList<(string Path, IFormFile File)> Files,
    byte[]? Archive,
    string? BaseVersion,
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
    public const string McpServerKind = "mcpServer";

    public async Task<PublishedVersion> PublishAsync(MarketplaceIdentity identity, PublishRequest request, CancellationToken cancellationToken)
    {
        CheckTarget(identity, request.Namespace, request.PackageId);

        if (!SemVer.TryParse(request.Version, out var semver))
        {
            throw new PublishRejectedException(422, $"{request.Version} is not a semantic version (major.minor.patch).");
        }

        var tags = NormalizeTags(request.Tags);
        InspectedArchive inspected;
        try
        {
            inspected = ArchiveInspector.Inspect(request.Archive);
        }
        catch (ArchiveRejectedException error)
        {
            throw new PublishRejectedException(422, error.Message);
        }

        if (inspected.SourceId != request.Namespace)
        {
            throw new PublishRejectedException(422, $"agent-plugins.json declares source.id {inspected.SourceId}; the namespace is {request.Namespace}.");
        }

        if (inspected.PackageId != request.PackageId)
        {
            throw new PublishRejectedException(422, $"agent-plugins.json declares package {inspected.PackageId}; the request names {request.PackageId}.");
        }

        var outcome = await ValidateAsync(request.Archive, inspected.RootPrefix, cancellationToken);
        if (!outcome.Accepted)
        {
            var errors = outcome.Errors.Count > 0 ? outcome.Errors : [new ValidationError("", "The package has no valid install.")];
            throw new PublishRejectedException(422, "The package failed validation.", errors);
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
            throw new PublishRejectedException(409, $"{request.Namespace}/{request.PackageId} {semver} is already published.");
        }

        StoredArtifact stored;
        try
        {
            stored = await store.PutAsync(storagePath, request.Archive, cancellationToken);
        }
        catch (ArtifactConflictException)
        {
            throw new PublishRejectedException(409, $"{request.Namespace}/{request.PackageId} {semver} already exists in the artifact store.");
        }

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
            Changelog = request.Changelog is { Length: > 0 } changelog ? changelog[..Math.Min(changelog.Length, 4096)] : null,
            ReviewState = approved ? ReviewState.Approved : ReviewState.Pending,
            ReviewedBy = approved ? identity.Account : null,
            ReviewedAt = approved ? now : null,
        };
        package.Versions.Add(version);

        // A pending version leaves the listing alone; a brand-new package is hidden until approved anyway.
        if (approved || isNew)
        {
            ApplyListing(package, version, now);
        }

        var displayName = IdentityResolver.NamespaceDisplayName(auth.Value, identity, request.Namespace);
        var publisher = await db.Publishers.FindAsync([request.Namespace], cancellationToken);
        if (publisher is null)
        {
            db.Publishers.Add(new Publisher
            {
                Namespace = request.Namespace,
                Account = request.Namespace == MarketplaceIdentity.OfficialNamespace ? MarketplaceIdentity.OfficialNamespace : identity.Account,
                DisplayName = displayName,
                FirstSeenAt = now,
                LastPublishedAt = now,
            });
        }
        else
        {
            publisher.LastPublishedAt = now;
            publisher.DisplayName = displayName;
        }

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
            throw new PublishRejectedException(422, $"An upload holds at most {MaxUploadFiles} files.");
        }

        var directory = Path.Combine(Path.GetTempPath(), "marketplace-upload-" + Guid.NewGuid().ToString("N"));
        var input = Path.Combine(directory, "input");
        try
        {
            if (upload.Archive is { } archive)
            {
                var (prefix, hasManifest) = ArchiveInspector.Survey(archive);
                if (hasManifest && upload.Files.Count == 0 && upload.BaseVersion is null)
                {
                    return archive;
                }

                ArchiveInspector.ExtractTo(archive, prefix, input);
            }

            if (upload.BaseVersion is { } baseVersion)
            {
                var stored = await db.PackageVersions.AsNoTracking()
                    .SingleOrDefaultAsync(version => version.Package.Namespace == upload.Namespace && version.Package.PackageId == upload.PackageId && version.Version == baseVersion, cancellationToken)
                    ?? throw new PublishRejectedException(422, $"{upload.Namespace}/{upload.PackageId} {baseVersion} is not published, so it cannot be edited.");
                var bytes = await store.GetAsync(stored.StoragePath, cancellationToken);
                ArchiveInspector.ExtractTo(bytes, ArchiveInspector.Survey(bytes).Prefix, input);
            }

            // A chosen folder arrives as folder/..., so strip a shared top directory; edits name paths from the source root.
            var strip = upload.BaseVersion is null ? ArchiveInspector.RootPrefix(upload.Files.Select(file => file.Path).ToArray()) : string.Empty;
            var root = Path.GetFullPath(input) + Path.DirectorySeparatorChar;
            var written = new List<string>(upload.Files.Count);
            foreach (var (path, file) in upload.Files)
            {
                ArchiveInspector.SafePath(path);
                var target = Path.GetFullPath(Path.Combine(root, path[strip.Length..]));
                if (!target.StartsWith(root, StringComparison.Ordinal) || target.Length == root.Length)
                {
                    throw new ArchiveRejectedException($"{path} is not a file path inside the upload.");
                }

                Directory.CreateDirectory(Path.GetDirectoryName(target)!);
                await using var source = file.OpenReadStream();
                await using var destination = File.Create(target);
                await source.CopyToAsync(destination, cancellationToken);
                written.Add(target);
            }

            // One uploaded .json is an MCP document to wrap on its own.
            var stageInput = written is [var only] && upload.BaseVersion is null && upload.Archive is null && only.EndsWith(".json", StringComparison.OrdinalIgnoreCase) ? only : input;
            var output = Path.Combine(directory, "source.zip");
            await validator.StageAsync(new StagingRequest(stageInput, output, upload.Namespace, upload.PackageId, upload.Name, upload.Description), cancellationToken);
            return await File.ReadAllBytesAsync(output, cancellationToken);
        }
        catch (ArchiveRejectedException error)
        {
            throw new PublishRejectedException(422, error.Message);
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
    public async Task<bool> ReviewAsync(MarketplaceIdentity identity, string ns, string packageId, string versionText, bool approve, string? note, CancellationToken cancellationToken)
    {
        note = string.IsNullOrWhiteSpace(note) ? null : note.Trim();
        if (!approve && note is null)
        {
            throw new PublishRejectedException(422, "Say why the version is rejected; the publisher sees the note.");
        }

        if (note is { Length: > MaxReviewNote })
        {
            throw new PublishRejectedException(422, $"A review note is at most {MaxReviewNote} characters.");
        }

        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await LockNamespaceAsync(ns, cancellationToken);
        var package = await db.Packages
            .Include(candidate => candidate.Versions)
            .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken);
        var version = package?.Versions.SingleOrDefault(candidate => candidate.Version == versionText);
        if (package is null || version is null)
        {
            return false;
        }

        if (version.ReviewState != ReviewState.Pending || version.Yanked)
        {
            throw new PublishRejectedException(409, $"{package.CanonicalId} {version.Version} is not waiting for review.");
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
        return true;
    }

    public async Task<bool> YankAsync(MarketplaceIdentity identity, string ns, string packageId, string versionText, CancellationToken cancellationToken)
    {
        if (!identity.Owns(ns))
        {
            throw new PublishRejectedException(403, $"{identity.Account} does not own the namespace {ns}.");
        }

        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await LockNamespaceAsync(ns, cancellationToken);
        var version = await db.PackageVersions
            .Include(candidate => candidate.Package)
            .SingleOrDefaultAsync(candidate => candidate.Package.Namespace == ns && candidate.Package.PackageId == packageId && candidate.Version == versionText, cancellationToken);
        if (version is null)
        {
            return false;
        }

        if (!version.Yanked)
        {
            version.Yanked = true;
            await db.SaveChangesAsync(cancellationToken);
            await RegenerateNamespaceAsync(ns, new Dictionary<string, ReadOnlyMemory<byte>>(), cancellationToken);
        }

        await transaction.CommitAsync(cancellationToken);
        return true;
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

    public static string[] NormalizeTags(IEnumerable<string> tags) =>
        tags.Select(tag => tag.Trim().ToLowerInvariant())
            .Where(tag => tag.Length is > 0 and <= 32 && TagPattern().IsMatch(tag))
            .Distinct(StringComparer.Ordinal)
            .Take(MaxTags)
            .ToArray();

    private async Task<ValidationOutcome> ValidateAsync(ReadOnlyMemory<byte> archive, string rootPrefix, CancellationToken cancellationToken)
    {
        var directory = Path.Combine(Path.GetTempPath(), "marketplace-validate-" + Guid.NewGuid().ToString("N"));
        try
        {
            ArchiveInspector.ExtractTo(archive, rootPrefix, directory);
            return await validator.ValidateAsync(directory, cancellationToken);
        }
        catch (ArchiveRejectedException error)
        {
            throw new PublishRejectedException(422, error.Message);
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
            throw new PublishRejectedException(403, $"{identity.Account} does not own the namespace {ns}.");
        }

        if (!IdentityResolver.SourceIdPattern().IsMatch(ns))
        {
            throw new PublishRejectedException(422, $"{ns} is not a valid namespace.");
        }

        if (!PackageIdPattern().IsMatch(packageId))
        {
            throw new PublishRejectedException(422, $"{packageId} is not a valid package id.");
        }
    }

    private async Task LockNamespaceAsync(string ns, CancellationToken cancellationToken)
    {
        if (db.Database.IsNpgsql())
        {
            await db.Database.ExecuteSqlAsync($"SELECT pg_advisory_xact_lock(hashtext({ns}))", cancellationToken);
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
