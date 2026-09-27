using System.Security.Cryptography;
using Marketplace.Api.Access;
using Marketplace.Api.Auth;
using Marketplace.Api.Data;
using Marketplace.Api.Notifications;
using Marketplace.Api.Storage;
using Microsoft.EntityFrameworkCore;

namespace Marketplace.Api.Packages;

public sealed record SuggestionView(
    long Id,
    string Target,
    string Namespace,
    string PackageId,
    string Name,
    string? BaseVersion,
    string? LiveVersion,
    string Message,
    string SuggestedBy,
    string SuggestedByName,
    DateTime SuggestedAt,
    SuggestionState State,
    string? DecidedBy,
    DateTime? DecidedAt,
    string? Note,
    string? AcceptedVersion,
    long SizeBytes,
    string[] ComponentKinds);

/// <summary>A file in a suggestion, compared with the live version: <c>added</c>, <c>changed</c>, <c>removed</c>, or <c>unchanged</c>.</summary>
public sealed record SuggestionFile(string Path, long Size, string Status);

public sealed record DecisionRequest(string? Decision, string? Version, string? Note);

public sealed record SuggestionsView(SuggestionView[] Waiting, SuggestionView[] Yours);

/// <summary>
/// Changes someone who does not own a package proposes. The upload is staged, validated, and scanned like
/// a publish, then waits for the package's owners, who accept it as their next version (credited to the
/// suggester) or decline it with a note.
/// </summary>
public sealed class SuggestionService(
    MarketplaceDbContext db,
    IArtifactStore store,
    AccessService access,
    PublishService publish,
    NotificationService notifications,
    TimeProvider timeProvider,
    ILogger<SuggestionService> logger)
{
    public const int MaxMessage = 4096;
    private const int MaxYours = 50;

    public async Task<SuggestionView> CreateAsync(MarketplaceIdentity identity, UploadRequest upload, string? message, CancellationToken cancellationToken)
    {
        var package = await access.VisiblePackageAsync(identity, upload.Namespace, upload.PackageId, cancellationToken);
        if (identity.Owns(package.Namespace))
        {
            throw new ProblemException(409, "You own this package; publish a new version instead.");
        }

        message = message?.Trim() ?? string.Empty;
        if (message.Length is 0 or > MaxMessage)
        {
            throw new ProblemException(422, $"Say what you changed, in up to {MaxMessage:N0} characters; the owners see your message.");
        }

        var archive = await publish.PrepareUploadAsync(upload, cancellationToken);
        var inspected = await publish.CheckArchiveAsync(package.Namespace, package.PackageId, archive, cancellationToken);
        var suggestion = new Suggestion
        {
            PackageId = package.Id,
            ArchiveDigest = Convert.ToHexStringLower(SHA256.HashData(archive)),
            SizeBytes = archive.Length,
            ManifestJson = inspected.PackageManifest.ToJsonString(),
            ComponentKinds = inspected.ComponentKinds.ToArray(),
            BaseVersion = PublishService.LatestVersion(package)?.Version,
            Message = message,
            SuggestedBy = identity.Account,
            SuggestedAt = timeProvider.GetUtcNow().UtcDateTime,
        };
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        db.Suggestions.Add(suggestion);
        await db.SaveChangesAsync(cancellationToken);
        suggestion.StoragePath = $"{package.Namespace}/{package.PackageId}/suggestions/{suggestion.Id}.zip";
        await store.PutAsync(suggestion.StoragePath, archive, cancellationToken);
        notifications.Notify(await notifications.OwnersAsync(package.Namespace, cancellationToken), identity.Account, "suggestion.created", $"{identity.DisplayName} suggested a change to {package.Name}.", $"/suggestions/{suggestion.Id}");
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        logger.LogInformation("{Account} suggested a change to {Package}.", identity.Account, package.CanonicalId);
        return await GetAsync(identity, suggestion.Id, cancellationToken);
    }

    /// <summary>A package's suggestions: every one for its owners, the caller's own for anyone else.</summary>
    public async Task<SuggestionView[]> ForPackageAsync(MarketplaceIdentity identity, string ns, string packageId, CancellationToken cancellationToken)
    {
        var package = await access.VisiblePackageAsync(identity, ns, packageId, cancellationToken);
        var owner = identity.Owns(ns);
        var account = identity.Account.ToLower();
        var suggestions = await db.Suggestions.AsNoTracking().Include(suggestion => suggestion.Package).ThenInclude(candidate => candidate.Versions)
            .Where(suggestion => suggestion.PackageId == package.Id && (owner || suggestion.SuggestedBy.ToLower() == account))
            .OrderByDescending(suggestion => suggestion.SuggestedAt)
            .ToListAsync(cancellationToken);
        return await ViewsAsync(suggestions, cancellationToken);
    }

    public async Task<SuggestionView> GetAsync(MarketplaceIdentity identity, long id, CancellationToken cancellationToken) =>
        (await ViewsAsync([await ReadableAsync(identity, id, cancellationToken)], cancellationToken))[0];

    /// <summary>The suggestion's stored zip, for the file list and file views; gone once the version it became was purged.</summary>
    public async Task<byte[]> ArchiveAsync(MarketplaceIdentity identity, long id, CancellationToken cancellationToken)
    {
        var suggestion = await ReadableAsync(identity, id, cancellationToken);
        if (suggestion.Package.Versions.Any(version => version.Version == suggestion.AcceptedVersion && version.PurgedAt is not null))
        {
            throw ProblemException.NotFound($"The files of suggestion {id}");
        }

        return await store.GetAsync(suggestion.StoragePath, cancellationToken);
    }

    /// <summary>Each file of the suggestion and of the live version, and how they differ.</summary>
    public async Task<SuggestionFile[]> FilesAsync(MarketplaceIdentity identity, long id, CancellationToken cancellationToken)
    {
        var suggestion = await ReadableAsync(identity, id, cancellationToken);
        var proposed = ArchiveInspector.ReadFiles(await ArchiveAsync(identity, id, cancellationToken));
        var live = PublishService.LatestVersion(suggestion.Package) is { } version
            ? ArchiveInspector.ReadFiles(await store.GetAsync(version.StoragePath, cancellationToken))
            : [];
        return proposed.Select(file => new SuggestionFile(
                file.Key,
                file.Value.Length,
                !live.TryGetValue(file.Key, out var current) ? "added" : current.AsSpan().SequenceEqual(file.Value) ? "unchanged" : "changed"))
            .Concat(live.Where(file => !proposed.ContainsKey(file.Key)).Select(file => new SuggestionFile(file.Key, file.Value.Length, "removed")))
            .OrderBy(file => file.Path, StringComparer.Ordinal)
            .ToArray();
    }

    /// <summary>Accepts (publishing the stored archive, credited to the suggester) or declines with a note.</summary>
    public async Task<SuggestionView> DecideAsync(MarketplaceIdentity identity, long id, DecisionRequest request, CancellationToken cancellationToken)
    {
        var suggestion = await ReadableAsync(identity, id, cancellationToken, tracked: true);
        var package = suggestion.Package;
        if (!identity.Owns(package.Namespace))
        {
            throw new ProblemException(403, "Only the package's owners can accept or decline a suggestion.");
        }

        RequirePending(suggestion);
        var now = timeProvider.GetUtcNow().UtcDateTime;
        suggestion.DecidedBy = identity.Account;
        suggestion.DecidedAt = now;
        if (request.Decision == "accept")
        {
            var semver = request.Version is { Length: > 0 } text ? PublishService.ParseVersion(text) : PublishService.NextPatch(package);
            var archive = await store.GetAsync(suggestion.StoragePath, cancellationToken);
            await ClaimAsync(suggestion, SuggestionState.Accepted, identity.Account, now, cancellationToken);
            suggestion.State = SuggestionState.Accepted;
            suggestion.AcceptedVersion = semver.ToString();
            Decided(identity, suggestion, $"{identity.DisplayName} accepted your suggestion to {package.Name}; it's version {semver} now.", now);

            // The commit saves this tracked suggestion in its own transaction, so accepting is all or nothing.
            try
            {
                await publish.CommitAsync(identity, suggestion.SuggestedBy, package.Namespace, package.PackageId, semver, package.Tags, suggestion.Message, archive, ArchiveInspector.Inspect(archive), cancellationToken);
            }
            catch
            {
                await db.Suggestions.Where(candidate => candidate.Id == suggestion.Id && candidate.State == SuggestionState.Accepted)
                    .ExecuteUpdateAsync(set => set.SetProperty(candidate => candidate.State, SuggestionState.Pending).SetProperty(candidate => candidate.DecidedBy, (string?)null).SetProperty(candidate => candidate.DecidedAt, (DateTime?)null), CancellationToken.None);
                throw;
            }
        }
        else if (request.Decision == "decline")
        {
            var note = request.Note?.Trim() ?? string.Empty;
            if (note.Length is 0 or > PublishService.MaxReviewNote)
            {
                throw new ProblemException(422, $"Say why you're declining, in up to {PublishService.MaxReviewNote:N0} characters; the person who suggested it sees your note.");
            }

            await ClaimAsync(suggestion, SuggestionState.Declined, identity.Account, now, cancellationToken);
            suggestion.State = SuggestionState.Declined;
            suggestion.DecisionNote = note;
            Decided(identity, suggestion, $"{identity.DisplayName} declined your suggestion to {package.Name}: {note}", now);
            await db.SaveChangesAsync(cancellationToken);
        }
        else
        {
            throw new ProblemException(422, "The decision is accept or decline.");
        }

        logger.LogInformation("{Account} {Decision} suggestion {Id} to {Package}.", identity.Account, request.Decision, id, package.CanonicalId);
        return await GetAsync(identity, id, cancellationToken);
    }

    /// <summary>Tells the suggester and records the decision; saved with the decision.</summary>
    private void Decided(MarketplaceIdentity identity, Suggestion suggestion, string text, DateTime now)
    {
        notifications.Notify([suggestion.SuggestedBy], identity.Account, "suggestion.decided", text, $"/suggestions/{suggestion.Id}");
        db.Audit(identity.Account, "suggestion.decide", suggestion.Package.CanonicalId, $"{suggestion.State.ToString().ToLowerInvariant()} suggestion {suggestion.Id}", now);
    }

    public async Task WithdrawAsync(MarketplaceIdentity identity, long id, CancellationToken cancellationToken)
    {
        var suggestion = await ReadableAsync(identity, id, cancellationToken, tracked: true);
        if (!IdentityResolver.IsClaimant(suggestion.SuggestedBy, identity.Account))
        {
            throw new ProblemException(403, "Only the person who suggested it can withdraw it.");
        }

        RequirePending(suggestion);
        await ClaimAsync(suggestion, SuggestionState.Withdrawn, identity.Account, timeProvider.GetUtcNow().UtcDateTime, cancellationToken);
    }

    /// <summary>
    /// Moves a pending suggestion to its decided state in one statement, so a decision and a withdrawal sent at
    /// once (or two decisions) can't both win: the second finds it no longer pending and gets 409.
    /// </summary>
    private async Task ClaimAsync(Suggestion suggestion, SuggestionState state, string account, DateTime now, CancellationToken cancellationToken)
    {
        var claimed = await db.Suggestions
            .Where(candidate => candidate.Id == suggestion.Id && candidate.State == SuggestionState.Pending)
            .ExecuteUpdateAsync(set => set.SetProperty(candidate => candidate.State, state).SetProperty(candidate => candidate.DecidedBy, account).SetProperty(candidate => candidate.DecidedAt, now), cancellationToken);
        if (claimed == 0)
        {
            throw new ProblemException(409, "This suggestion is not waiting any more: someone decided or withdrew it just now.");
        }
    }

    /// <summary>Suggestions waiting on the caller's namespaces, and the caller's own.</summary>
    public async Task<SuggestionsView> MineAsync(MarketplaceIdentity identity, CancellationToken cancellationToken)
    {
        var namespaces = identity.Namespaces.ToArray();
        var account = identity.Account.ToLower();
        var waiting = await db.Suggestions.AsNoTracking().Include(suggestion => suggestion.Package).ThenInclude(package => package.Versions)
            .Where(suggestion => suggestion.State == SuggestionState.Pending && namespaces.Contains(suggestion.Package.Namespace))
            .OrderBy(suggestion => suggestion.SuggestedAt)
            .ToListAsync(cancellationToken);
        var yours = await db.Suggestions.AsNoTracking().Include(suggestion => suggestion.Package).ThenInclude(package => package.Versions)
            .Where(suggestion => suggestion.SuggestedBy.ToLower() == account)
            .OrderByDescending(suggestion => suggestion.SuggestedAt)
            .Take(MaxYours)
            .ToListAsync(cancellationToken);
        return new SuggestionsView(await ViewsAsync(waiting, cancellationToken), await ViewsAsync(yours, cancellationToken));
    }

    public async Task<int> WaitingCountAsync(MarketplaceIdentity identity, CancellationToken cancellationToken)
    {
        var namespaces = identity.Namespaces.ToArray();
        return await db.Suggestions.CountAsync(suggestion => suggestion.State == SuggestionState.Pending && namespaces.Contains(suggestion.Package.Namespace), cancellationToken);
    }

    /// <summary>The suggestion if the caller may read it: the target's owners, the suggester, and admins.</summary>
    private async Task<Suggestion> ReadableAsync(MarketplaceIdentity identity, long id, CancellationToken cancellationToken, bool tracked = false)
    {
        var query = db.Suggestions.Include(suggestion => suggestion.Package).ThenInclude(package => package.Versions).AsQueryable();
        var suggestion = await (tracked ? query : query.AsNoTracking()).SingleOrDefaultAsync(candidate => candidate.Id == id, cancellationToken);
        return suggestion is not null && (identity.Owns(suggestion.Package.Namespace) || IdentityResolver.IsClaimant(suggestion.SuggestedBy, identity.Account))
            ? suggestion
            : throw ProblemException.NotFound($"Suggestion {id}");
    }

    private static void RequirePending(Suggestion suggestion)
    {
        if (suggestion.State != SuggestionState.Pending)
        {
            throw new ProblemException(409, $"This suggestion is not waiting any more: it was {suggestion.State.ToString().ToLowerInvariant()}.");
        }
    }

    private async Task<SuggestionView[]> ViewsAsync(IReadOnlyList<Suggestion> suggestions, CancellationToken cancellationToken)
    {
        var names = await access.DisplayNamesAsync(suggestions.Select(suggestion => suggestion.SuggestedBy).Distinct().ToArray(), cancellationToken);
        return suggestions.Select(suggestion => new SuggestionView(
                suggestion.Id,
                suggestion.Package.CanonicalId,
                suggestion.Package.Namespace,
                suggestion.Package.PackageId,
                suggestion.Package.Name,
                suggestion.BaseVersion,
                PublishService.LatestVersion(suggestion.Package)?.Version,
                suggestion.Message,
                suggestion.SuggestedBy,
                names.GetValueOrDefault(suggestion.SuggestedBy) ?? IdentityResolver.Username(suggestion.SuggestedBy),
                suggestion.SuggestedAt,
                suggestion.State,
                suggestion.DecidedBy,
                suggestion.DecidedAt,
                suggestion.DecisionNote,
                suggestion.AcceptedVersion,
                suggestion.SizeBytes,
                suggestion.ComponentKinds))
            .ToArray();
    }
}
