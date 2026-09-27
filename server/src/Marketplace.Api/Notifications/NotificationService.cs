using Marketplace.Api.Auth;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Options;

namespace Marketplace.Api.Notifications;

public sealed record NotificationView(long Id, string Kind, string Text, string? Link, DateTime CreatedAt, bool Read);

public sealed record NotificationsView(NotificationView[] Items);

public sealed record ReadRequest(long? UpTo);

/// <summary>
/// What people hear about: suggestions, review decisions, reports, being added to a team or a share list,
/// and an admin pulling their package. The portal lists them and the desktop app shows new ones.
/// </summary>
public sealed class NotificationService(
    MarketplaceDbContext db,
    IOptions<AuthOptions> auth,
    IOptions<ServerOptions> server,
    IOptions<NotificationOptions> options,
    IHttpClientFactory httpClients,
    TimeProvider timeProvider,
    ILogger<NotificationService> logger)
{
    public const int MaxLimit = 200;
    public const string WebhookClient = "webhook";

    /// <summary>Queues one notification per recipient, never to <paramref name="actor"/>; saved with the caller's next <c>SaveChanges</c>.</summary>
    public void Notify(IEnumerable<string> recipients, string actor, string kind, string text, string? link)
    {
        var now = timeProvider.GetUtcNow().UtcDateTime;
        foreach (var account in recipients.Where(account => account.Length > 0).Distinct(StringComparer.OrdinalIgnoreCase))
        {
            if (IdentityResolver.EntryMatches(account, actor))
            {
                continue;
            }

            db.Notifications.Add(new Notification
            {
                Account = account,
                Kind = kind,
                Text = text.Length <= 512 ? text : text[..509] + "...",
                Link = link,
                CreatedAt = now,
            });
        }
    }

    /// <summary>Who owns a namespace: a person's account, a team's members, or <c>official</c>'s configured admins and publishers.</summary>
    public async Task<string[]> OwnersAsync(string ns, CancellationToken cancellationToken)
    {
        if (ns == MarketplaceIdentity.OfficialNamespace)
        {
            return [.. auth.Value.AdminAccounts, .. auth.Value.OfficialPublishers];
        }

        var publisher = await db.Publishers.AsNoTracking().SingleOrDefaultAsync(candidate => candidate.Namespace == ns, cancellationToken);
        return publisher?.Kind == PublisherKind.Team
            ? await db.TeamMembers.AsNoTracking().Where(member => member.Namespace == ns).Select(member => member.Account).ToArrayAsync(cancellationToken)
            : publisher is null ? [] : [publisher.Account];
    }

    public async Task<NotificationsView> ListAsync(MarketplaceIdentity identity, long? after, int? limit, CancellationToken cancellationToken)
    {
        var take = Math.Clamp(limit ?? 50, 1, MaxLimit);
        var query = Mine(identity);
        var items = after is { } since
            ? query.Where(notification => notification.Id > since).OrderBy(notification => notification.Id)
            : query.OrderByDescending(notification => notification.Id);
        return new NotificationsView(await items.Take(take)
            .Select(notification => new NotificationView(notification.Id, notification.Kind, notification.Text, notification.Link, notification.CreatedAt, notification.ReadAt != null))
            .ToArrayAsync(cancellationToken));
    }

    public async Task ReadAsync(MarketplaceIdentity identity, long upTo, CancellationToken cancellationToken)
    {
        var now = timeProvider.GetUtcNow().UtcDateTime;
        await Mine(identity)
            .Where(notification => notification.Id <= upTo && notification.ReadAt == null)
            .ExecuteUpdateAsync(setters => setters.SetProperty(notification => notification.ReadAt, now), cancellationToken);
    }

    public Task<int> UnreadAsync(MarketplaceIdentity identity, CancellationToken cancellationToken) =>
        Mine(identity).CountAsync(notification => notification.ReadAt == null, cancellationToken);

    /// <summary>
    /// Posts <c>{ text, link }</c> to <c>Notifications:WebhookUrl</c>, when set, without holding up the request.
    /// A failure is logged; the admin portal still has everything.
    /// </summary>
    public void Webhook(string text, string? link)
    {
        if (options.Value.WebhookUrl is not { Length: > 0 } url)
        {
            return;
        }

        var body = new { text, link = link is null ? null : server.Value.PublicBaseUrl.TrimEnd('/') + link };
        var client = httpClients.CreateClient(WebhookClient);
        _ = Task.Run(async () =>
        {
            try
            {
                using var response = await client.PostAsJsonAsync(url, body);
                response.EnsureSuccessStatusCode();
            }
            catch (Exception error) when (error is HttpRequestException or TaskCanceledException)
            {
                logger.LogWarning(error, "The notification webhook failed.");
            }
        });
    }

    /// <summary>The caller's notifications. A bare username entry (a team member added by name) matches any domain.</summary>
    private IQueryable<Notification> Mine(MarketplaceIdentity identity)
    {
        var account = identity.Account.ToLower();
        var username = IdentityResolver.Username(identity.Account).ToLower();
        return db.Notifications.Where(notification => notification.Account.ToLower() == account || notification.Account.ToLower() == username);
    }
}
