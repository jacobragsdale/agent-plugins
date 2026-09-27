using Marketplace.Api.Access;
using Marketplace.Api.Auth;
using Marketplace.Api.Data;
using Marketplace.Api.Notifications;
using Marketplace.Api.Packages;
using Microsoft.EntityFrameworkCore;

namespace Marketplace.Api.Teams;

public sealed record TeamMemberView(string Account, string DisplayName, bool Owner, DateTime JoinedAt);

/// <summary><see cref="Role"/> is the caller's: <c>owner</c>, <c>member</c>, or <c>admin</c> for an admin who is not a member.</summary>
public sealed record TeamView(string Namespace, string DisplayName, Visibility Visibility, string Role, TeamMemberView[] Members, string? Invite);

public sealed record TeamSummary(string Namespace, string DisplayName, Visibility Visibility, string Role, int MemberCount);

public sealed record CreateTeamRequest(string? Namespace, string? DisplayName, Visibility? Visibility);

public sealed record RenameTeamRequest(string? DisplayName);

public sealed record MemberRequest(string? Account, bool? Owner);

public sealed record InviteRequest(bool? Reset);

public sealed record InviteView(string Invite);

public sealed record TransferRequest(string? Account);

public sealed record DirectoryView(PersonView[] People, TeamRef[] Teams);

/// <summary>What a link leads to. <see cref="TargetKind"/> is <c>team</c>, <c>space</c>, <c>package</c>, or <c>bundle</c>.</summary>
public sealed record LinkView(string Kind, string TargetKind, string Target, string Name, string By);

public sealed record RedeemedLink(string Kind, string TargetKind, string Target, string Name, string By, bool Changed);

/// <summary>
/// Teams anyone can create. Members publish, withdraw, share, and edit bundles in the team's namespace;
/// owners also manage members, the invite link, the name, and the team's visibility. A team is a
/// publisher row of kind team, created with the team, whose account is the namespace itself.
/// </summary>
public sealed class TeamService(MarketplaceDbContext db, AccessService access, PublishService publish, NotificationService notifications, TimeProvider timeProvider)
{
    public const int MaxDirectoryResults = 10;
    private const string GoneLink = "This link no longer works. Ask the person who sent it for a new one.";

    public async Task<TeamView> CreateAsync(MarketplaceIdentity identity, CreateTeamRequest request, CancellationToken cancellationToken)
    {
        var ns = request.Namespace?.Trim() ?? string.Empty;
        if (!IdentityResolver.SourceIdPattern().IsMatch(ns) || ns == MarketplaceIdentity.OfficialNamespace)
        {
            throw new ProblemException(422, $"\"{ns}\" can't be a team name. Use 2 to 16 lowercase letters, digits, and single hyphens, starting with a letter.");
        }

        var displayName = DisplayName(request.DisplayName);
        var visibility = request.Visibility ?? Visibility.Private;
        if (visibility == Visibility.Inherit)
        {
            throw new ProblemException(422, "A team is public or private.");
        }

        var now = timeProvider.GetUtcNow().UtcDateTime;
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);
        if (await db.Publishers.AnyAsync(publisher => publisher.Namespace == ns, cancellationToken)
            || await db.Packages.AnyAsync(package => package.Namespace == ns, cancellationToken))
        {
            throw new ProblemException(409, $"The name {ns} is taken. Pick another.");
        }

        db.Publishers.Add(new Publisher { Namespace = ns, Account = ns, DisplayName = displayName, Kind = PublisherKind.Team, FirstSeenAt = now });
        db.TeamMembers.Add(new TeamMember { Namespace = ns, Account = identity.Account, IsOwner = true, JoinedAt = now });
        if (visibility == Visibility.Private)
        {
            db.AccessRules.Add(new AccessRule { Target = ns, Visibility = Visibility.Private, UpdatedBy = identity.Account, UpdatedAt = now });
        }

        db.Audit(identity.Account, "team.create", ns, displayName, now);
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        return await ViewAsync(identity, ns, cancellationToken);
    }

    /// <summary>The caller's teams.</summary>
    public async Task<List<TeamSummary>> ListAsync(MarketplaceIdentity identity, CancellationToken cancellationToken)
    {
        var namespaces = identity.Teams.Select(team => team.Namespace).ToArray();
        var counts = await db.TeamMembers.AsNoTracking()
            .Where(member => namespaces.Contains(member.Namespace))
            .GroupBy(member => member.Namespace)
            .Select(group => new { Namespace = group.Key, Count = group.Count() })
            .ToDictionaryAsync(group => group.Namespace, group => group.Count, StringComparer.Ordinal, cancellationToken);
        var rules = await access.RulesAsync(cancellationToken);
        return identity.Teams
            .Select(team => new TeamSummary(team.Namespace, team.DisplayName, VisibilityOf(rules, team.Namespace), team.Owner ? "owner" : "member", counts.GetValueOrDefault(team.Namespace)))
            .ToList();
    }

    public async Task<TeamView> GetAsync(MarketplaceIdentity identity, string ns, CancellationToken cancellationToken)
    {
        if (!identity.InTeam(ns) && !identity.IsAdmin)
        {
            throw ProblemException.NotFound($"The team {ns}");
        }

        return await ViewAsync(identity, ns, cancellationToken);
    }

    public async Task<TeamView> RenameAsync(MarketplaceIdentity identity, string ns, RenameTeamRequest request, CancellationToken cancellationToken)
    {
        var team = await OwnedTeamAsync(identity, ns, cancellationToken);
        team.DisplayName = DisplayName(request.DisplayName);
        db.Audit(identity.Account, "team.rename", ns, team.DisplayName, timeProvider.GetUtcNow().UtcDateTime);
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);
        await db.SaveChangesAsync(cancellationToken);

        // The namespace archive names its source after the team.
        await publish.RegenerateNamespaceAsync(ns, new Dictionary<string, ReadOnlyMemory<byte>>(), cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        return await ViewAsync(identity, ns, cancellationToken);
    }

    /// <summary>Adds a member, or changes whether an existing one is an owner.</summary>
    public async Task<TeamView> SetMemberAsync(MarketplaceIdentity identity, string ns, MemberRequest request, CancellationToken cancellationToken)
    {
        var team = await OwnedTeamAsync(identity, ns, cancellationToken);
        var account = request.Account?.Trim() ?? string.Empty;
        if (account.Length is 0 or > AccessService.MaxEntryLength)
        {
            throw new ProblemException(422, $"Name the person by their Windows account, up to {AccessService.MaxEntryLength} characters.");
        }

        var now = timeProvider.GetUtcNow().UtcDateTime;
        var members = await db.TeamMembers.Where(member => member.Namespace == ns).ToListAsync(cancellationToken);
        var existing = members.FirstOrDefault(member => string.Equals(member.Account, account, StringComparison.OrdinalIgnoreCase));
        if (existing is null)
        {
            db.TeamMembers.Add(new TeamMember { Namespace = ns, Account = account, IsOwner = request.Owner ?? false, JoinedAt = now });
            notifications.Notify([account], identity.Account, "team.added", $"{identity.DisplayName} added you to {team.DisplayName}. You can publish there now.", $"/teams/{ns}");
        }
        else
        {
            existing.IsOwner = request.Owner ?? existing.IsOwner;
            RequireAnOwner(members);
        }

        db.Audit(identity.Account, "team.member.set", ns, $"{account}{(request.Owner == true ? " (owner)" : string.Empty)}", now);
        await db.SaveChangesAsync(cancellationToken);
        return await ViewAsync(identity, ns, cancellationToken);
    }

    /// <summary>An owner removes anyone; any member removes themselves (leaves). The last owner can't go.</summary>
    public async Task RemoveMemberAsync(MarketplaceIdentity identity, string ns, string? account, CancellationToken cancellationToken)
    {
        account = account?.Trim() ?? string.Empty;
        var leaving = account.Length > 0 && identity.Is(account);
        if (!leaving)
        {
            await OwnedTeamAsync(identity, ns, cancellationToken);
        }
        else if (!identity.InTeam(ns))
        {
            throw ProblemException.NotFound($"The team {ns}");
        }

        var members = await db.TeamMembers.Where(member => member.Namespace == ns).ToListAsync(cancellationToken);
        var removed = members
            .Where(member => leaving ? identity.Is(member.Account) : string.Equals(member.Account, account, StringComparison.OrdinalIgnoreCase))
            .ToList();
        if (removed.Count == 0)
        {
            throw ProblemException.NotFound($"{account} in the team {ns}");
        }

        RequireAnOwner(members.Except(removed).ToList());
        db.TeamMembers.RemoveRange(removed);
        db.Audit(identity.Account, "team.member.remove", ns, string.Join(", ", removed.Select(member => member.Account)), timeProvider.GetUtcNow().UtcDateTime);
        await db.SaveChangesAsync(cancellationToken);
    }

    /// <summary>
    /// The invite link, which makes whoever opens it a publisher: only owners create or reset it. Members see a
    /// link that exists on the team, so they can pass it on.
    /// </summary>
    public async Task<InviteView> InviteAsync(MarketplaceIdentity identity, string ns, bool reset, CancellationToken cancellationToken)
    {
        if (!identity.InTeam(ns) && !identity.IsAdmin)
        {
            throw ProblemException.NotFound($"The team {ns}");
        }

        if (!identity.IsTeamOwner(ns))
        {
            throw new ProblemException(403, "Only the team's owners can share its invite link, because joining lets people publish.");
        }

        await TeamAsync(ns, cancellationToken);
        return new InviteView(await access.LinkAsync(Link.Invite, ns, identity.Account, reset, cancellationToken));
    }

    /// <summary>
    /// Deletes a team that never published a package or bundle, with its members, links, and rules. An admin may
    /// also delete one that did: its packages are revoked so every PC removes them, and the namespace stays
    /// reserved with no members, because installed skill names are built from it.
    /// </summary>
    public async Task DeleteAsync(MarketplaceIdentity identity, string ns, CancellationToken cancellationToken)
    {
        var team = await OwnedTeamAsync(identity, ns, cancellationToken);
        var now = timeProvider.GetUtcNow().UtcDateTime;
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);
        var packages = await db.Packages.Where(package => package.Namespace == ns).ToListAsync(cancellationToken);
        var used = packages.Count > 0 || await db.Bundles.AnyAsync(bundle => bundle.Namespace == ns, cancellationToken);
        if (used && !identity.IsAdmin)
        {
            throw new ProblemException(409, $"{team.DisplayName} has packages or bundles, so it can't be deleted.");
        }

        var prefix = ns + "/";
        foreach (var package in packages.Where(package => package.RevokedAt is null))
        {
            package.RevokedAt = now;
            package.RevokedBy = identity.Account;
            package.RevokedByAdmin = true;
        }

        await db.Bundles.Where(bundle => bundle.Namespace == ns).ExecuteDeleteAsync(cancellationToken);
        await db.TeamMembers.Where(member => member.Namespace == ns).ExecuteDeleteAsync(cancellationToken);
        await db.Links.Where(link => link.Target == ns || link.Target.StartsWith(prefix)).ExecuteDeleteAsync(cancellationToken);
        await db.AccessRules.Where(rule => rule.Target == ns || rule.Target.StartsWith(prefix)).ExecuteDeleteAsync(cancellationToken);
        await db.NamespaceArchives.Where(archive => archive.Namespace == ns).ExecuteDeleteAsync(cancellationToken);
        if (!used)
        {
            db.Publishers.Remove(team);
        }
        else
        {
            // The name stays reserved, and private: its revoked ids go only to people who had them installed.
            db.AccessRules.Add(new AccessRule { Target = ns, Visibility = Visibility.Private, UpdatedBy = identity.Account, UpdatedAt = now });
        }

        db.Audit(identity.Account, "team.delete", ns, used ? $"revoked {packages.Count} packages" : null, now);
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
    }

    /// <summary>
    /// An admin hands a personal namespace to someone else, for a departed owner: it becomes a team the new
    /// account owns, so the namespace, and every installed skill name built from it, stays the same.
    /// </summary>
    public async Task TransferAsync(MarketplaceIdentity identity, string ns, TransferRequest request, CancellationToken cancellationToken)
    {
        var account = request.Account?.Trim() ?? string.Empty;
        if (account.Length is 0 or > AccessService.MaxEntryLength)
        {
            throw new ProblemException(422, $"Name the new owner by their Windows account, up to {AccessService.MaxEntryLength} characters.");
        }

        var now = timeProvider.GetUtcNow().UtcDateTime;
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await db.LockNamespaceAsync(ns, cancellationToken);
        var publisher = await db.Publishers.FindAsync([ns], cancellationToken) ?? throw ProblemException.NotFound($"The namespace {ns}");
        if (publisher.Kind != PublisherKind.Personal)
        {
            throw new ProblemException(409, $"{ns} is not a personal namespace. Change a team's owners on its team page.");
        }

        var previous = publisher.Account;
        publisher.Kind = PublisherKind.Team;
        publisher.Account = ns;
        db.TeamMembers.Add(new TeamMember { Namespace = ns, Account = account, IsOwner = true, JoinedAt = now });
        db.Audit(identity.Account, "namespace.transfer", ns, $"from {previous} to {account}", now);
        notifications.Notify([account], identity.Account, "team.added", $"An admin gave you {publisher.DisplayName} ({ns}). You own it as a team now.", $"/teams/{ns}");
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
    }

    /// <summary>People who have signed in, and teams the caller may see, whose name or account contains <paramref name="query"/>.</summary>
    public async Task<DirectoryView> DirectoryAsync(MarketplaceIdentity identity, string? query, CancellationToken cancellationToken)
    {
        var text = query?.Trim() ?? string.Empty;
        if (text.Length == 0)
        {
            return new DirectoryView([], []);
        }

        const string escape = "\\";
        var pattern = "%" + text.Replace(escape, escape + escape).Replace("%", escape + "%").Replace("_", escape + "_") + "%";
        var people = await db.People.AsNoTracking()
            .Where(person => EF.Functions.ILike(person.Account, pattern, escape) || EF.Functions.ILike(person.DisplayName, pattern, escape))
            .OrderBy(person => person.DisplayName)
            .Take(MaxDirectoryResults)
            .Select(person => new PersonView(person.Account, person.DisplayName))
            .ToArrayAsync(cancellationToken);
        var teams = await db.Publishers.AsNoTracking()
            .Where(publisher => publisher.Kind == PublisherKind.Team && db.TeamMembers.Any(member => member.Namespace == publisher.Namespace)
                && (EF.Functions.ILike(publisher.Namespace, pattern, escape) || EF.Functions.ILike(publisher.DisplayName, pattern, escape)))
            .OrderBy(publisher => publisher.Namespace)
            .ToListAsync(cancellationToken);
        var rules = await access.RulesAsync(cancellationToken);
        return new DirectoryView(
            people,
            teams.Where(team => AccessService.IsVisible(rules, identity, team.Namespace, null))
                .Take(MaxDirectoryResults)
                .Select(team => new TeamRef(team.Namespace, team.DisplayName))
                .ToArray());
    }

    public async Task<LinkView> PreviewLinkAsync(string code, CancellationToken cancellationToken)
    {
        var link = await db.Links.AsNoTracking().SingleOrDefaultAsync(candidate => candidate.Code == code, cancellationToken)
            ?? throw new ProblemException(404, GoneLink);
        var (targetKind, name) = await DescribeAsync(link, cancellationToken);
        var by = (await access.DisplayNamesAsync([link.CreatedBy], cancellationToken)).GetValueOrDefault(link.CreatedBy) ?? IdentityResolver.Username(link.CreatedBy);
        return new LinkView(link.Kind, targetKind, link.Target, name, by);
    }

    /// <summary>Joins the team or adds the caller to the share list. Redeeming twice changes nothing.</summary>
    public async Task<RedeemedLink> RedeemLinkAsync(MarketplaceIdentity identity, string code, CancellationToken cancellationToken)
    {
        var preview = await PreviewLinkAsync(code, cancellationToken);
        bool changed;
        if (preview.Kind == Link.Invite)
        {
            changed = !identity.InTeam(preview.Target);
            if (changed)
            {
                var now = timeProvider.GetUtcNow().UtcDateTime;
                db.TeamMembers.Add(new TeamMember { Namespace = preview.Target, Account = identity.Account, JoinedAt = now });
                db.Audit(identity.Account, "link.redeem", preview.Target, "invite", now);
                await db.SaveChangesAsync(cancellationToken);
            }
        }
        else
        {
            changed = await access.AddToShareListAsync(identity, preview.Target, cancellationToken);
        }

        return new RedeemedLink(preview.Kind, preview.TargetKind, preview.Target, preview.Name, preview.By, changed);
    }

    private async Task<(string TargetKind, string Name)> DescribeAsync(Link link, CancellationToken cancellationToken)
    {
        var (ns, id) = AccessService.Split(link.Target);
        var publisher = await db.Publishers.AsNoTracking().SingleOrDefaultAsync(candidate => candidate.Namespace == ns, cancellationToken);
        if (link.Kind == Link.Invite)
        {
            return publisher?.Kind == PublisherKind.Team ? ("team", publisher.DisplayName) : throw new ProblemException(404, GoneLink);
        }

        if (id is null)
        {
            return ("space", publisher?.DisplayName is { Length: > 0 } spaceName ? spaceName : ns);
        }

        var package = await db.Packages.AsNoTracking().Where(candidate => candidate.Namespace == ns && candidate.PackageId == id && candidate.RevokedAt == null)
            .Select(candidate => candidate.Name)
            .SingleOrDefaultAsync(cancellationToken);
        if (package is not null)
        {
            return ("package", package);
        }

        var bundle = await db.Bundles.AsNoTracking().Where(candidate => candidate.Namespace == ns && candidate.BundleId == id)
            .Select(candidate => candidate.Name)
            .SingleOrDefaultAsync(cancellationToken);
        return bundle is not null ? ("bundle", bundle) : throw new ProblemException(404, GoneLink);
    }

    private async Task<TeamView> ViewAsync(MarketplaceIdentity identity, string ns, CancellationToken cancellationToken)
    {
        var team = await TeamAsync(ns, cancellationToken);
        var members = await db.TeamMembers.AsNoTracking().Where(member => member.Namespace == ns).OrderBy(member => member.JoinedAt).ToListAsync(cancellationToken);
        var names = await access.DisplayNamesAsync(members.Select(member => member.Account).ToArray(), cancellationToken);
        var invite = await db.Links.AsNoTracking()
            .Where(link => link.Kind == Link.Invite && link.Target == ns)
            .Select(link => link.Code)
            .SingleOrDefaultAsync(cancellationToken);
        var mine = members.Where(member => identity.Is(member.Account)).ToList();
        var role = mine.Count > 0 ? (mine.Any(member => member.IsOwner) ? "owner" : "member") : "admin";
        return new TeamView(
            ns,
            team.DisplayName,
            VisibilityOf(await access.RulesAsync(cancellationToken), ns),
            role,
            members.Select(member => new TeamMemberView(member.Account, names.GetValueOrDefault(member.Account) ?? IdentityResolver.Username(member.Account), member.IsOwner, member.JoinedAt)).ToArray(),
            invite is null ? null : access.LinkUrl(invite));
    }

    private async Task<Publisher> TeamAsync(string ns, CancellationToken cancellationToken) =>
        await db.Publishers.SingleOrDefaultAsync(publisher => publisher.Namespace == ns && publisher.Kind == PublisherKind.Team, cancellationToken)
        ?? throw ProblemException.NotFound($"The team {ns}");

    private async Task<Publisher> OwnedTeamAsync(MarketplaceIdentity identity, string ns, CancellationToken cancellationToken)
    {
        if (!identity.InTeam(ns) && !identity.IsAdmin)
        {
            throw ProblemException.NotFound($"The team {ns}");
        }

        var team = await TeamAsync(ns, cancellationToken);
        return identity.IsTeamOwner(ns) ? team : throw new ProblemException(403, $"Only the owners of {team.DisplayName} can do that.");
    }

    private static void RequireAnOwner(IReadOnlyCollection<TeamMember> members)
    {
        if (!members.Any(member => member.IsOwner))
        {
            throw new ProblemException(409, "A team needs an owner. Make someone else an owner first.");
        }
    }

    private static Visibility VisibilityOf(IReadOnlyDictionary<string, AccessRule> rules, string ns) =>
        AccessService.Effective(rules, ns, null).Private ? Visibility.Private : Visibility.Public;

    private static string DisplayName(string? requested)
    {
        var name = requested?.Trim() ?? string.Empty;
        return name.Length is > 0 and <= 120 ? name : throw new ProblemException(422, "A team's name is 1 to 120 characters.");
    }
}
