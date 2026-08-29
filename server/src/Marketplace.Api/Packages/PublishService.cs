using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Marketplace.Api.Auth;
using Marketplace.Api.Data;
using Marketplace.Api.Storage;
using Microsoft.EntityFrameworkCore;

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
    string[] Tags);

public sealed partial class PublishService(
    MarketplaceDbContext db,
    IArtifactStore store,
    IPackageValidator validator,
    TimeProvider timeProvider,
    ILogger<PublishService> logger)
{
    public const int MaxTags = 10;

    public async Task<PublishedVersion> PublishAsync(MarketplaceIdentity identity, PublishRequest request, CancellationToken cancellationToken)
    {
        if (!identity.Owns(request.Namespace))
        {
            throw new PublishRejectedException(403, $"{identity.Account} does not own the namespace {request.Namespace}.");
        }

        if (!IdentityResolver.SourceIdPattern().IsMatch(request.Namespace))
        {
            throw new PublishRejectedException(422, $"{request.Namespace} is not a valid namespace.");
        }

        if (!PackageIdPattern().IsMatch(request.PackageId))
        {
            throw new PublishRejectedException(422, $"{request.PackageId} is not a valid package id.");
        }

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

        var name = inspected.PackageManifest["name"]?.GetValue<string>() ?? request.PackageId;
        var description = inspected.PackageManifest["description"]?.GetValue<string>() ?? DescriptionFromComponents(inspected) ?? name;
        if (package is null)
        {
            package = new Package
            {
                Namespace = request.Namespace,
                PackageId = request.PackageId,
                Name = name,
                Description = description,
                Tags = tags,
                CreatedAt = now,
                UpdatedAt = now,
            };
            db.Packages.Add(package);
        }
        else
        {
            package.Name = name;
            package.Description = description;
            package.Tags = tags.Length > 0 ? tags : package.Tags;
            package.UpdatedAt = now;
        }

        var version = new PackageVersion
        {
            Package = package,
            Version = semver.ToString(),
            StoragePath = storagePath,
            ArchiveDigest = stored.Sha256,
            SizeBytes = stored.SizeBytes,
            ManifestJson = inspected.PackageManifest.ToJsonString(),
            ComponentKinds = inspected.ComponentKinds.ToArray(),
            PublishedBy = identity.Account,
            PublishedAt = now,
            Changelog = request.Changelog is { Length: > 0 } changelog ? changelog[..Math.Min(changelog.Length, 4096)] : null,
        };
        package.Versions.Add(version);

        var publisher = await db.Publishers.FindAsync([request.Namespace], cancellationToken);
        if (publisher is null)
        {
            db.Publishers.Add(new Publisher
            {
                Namespace = request.Namespace,
                Account = request.Namespace == MarketplaceIdentity.OfficialNamespace ? MarketplaceIdentity.OfficialNamespace : identity.Account,
                DisplayName = request.Namespace == MarketplaceIdentity.OfficialNamespace ? "Official" : identity.DisplayName,
                FirstSeenAt = now,
                LastPublishedAt = now,
            });
        }
        else
        {
            publisher.LastPublishedAt = now;
            if (request.Namespace != MarketplaceIdentity.OfficialNamespace)
            {
                publisher.DisplayName = identity.DisplayName;
            }
        }

        await db.SaveChangesAsync(cancellationToken);
        await RegenerateNamespaceAsync(request.Namespace, new Dictionary<string, ReadOnlyMemory<byte>> { [storagePath] = request.Archive }, cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        logger.LogInformation("{Account} published {Namespace}/{Package} {Version}.", identity.Account, request.Namespace, request.PackageId, semver);
        return ToPublished(package, version);
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

    public static PackageVersion? LatestVersion(Package package) =>
        package.Versions
            .Where(version => !version.Yanked)
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
        package.Tags);

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
            try
            {
                Directory.Delete(directory, recursive: true);
            }
            catch (IOException)
            {
            }
            catch (UnauthorizedAccessException)
            {
            }
        }
    }

    private async Task LockNamespaceAsync(string ns, CancellationToken cancellationToken)
    {
        if (db.Database.IsNpgsql())
        {
            await db.Database.ExecuteSqlAsync($"SELECT pg_advisory_xact_lock(hashtext({ns}))", cancellationToken);
        }
    }

    private static string? DescriptionFromComponents(InspectedArchive inspected) =>
        inspected.ComponentKinds.Count == 0 ? null : $"{string.Join(", ", inspected.ComponentKinds)} package.";

    [GeneratedRegex("^[a-z0-9](?:[a-z0-9]|-(?=[a-z0-9])){0,63}$")]
    private static partial Regex PackageIdPattern();

    [GeneratedRegex("^[a-z0-9](?:[a-z0-9]|-(?=[a-z0-9]))*$")]
    private static partial Regex TagPattern();
}
