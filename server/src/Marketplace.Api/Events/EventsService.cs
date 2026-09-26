using System.Text.Json;
using Marketplace.Api.Data;
using Microsoft.EntityFrameworkCore;

namespace Marketplace.Api.Events;

/// <summary>One client event. Fields beyond <c>kind</c>, <c>occurredAt</c>, and <c>clientVersion</c> depend on the kind.</summary>
public sealed record ClientEventDto(
    string Kind,
    DateTimeOffset OccurredAt,
    string ClientVersion,
    string? OsBuild,
    string[]? Agents,
    string[]? Installed,
    Dictionary<string, string>? Checks,
    string? PackageId,
    string? Version,
    string? FromVersion,
    string? ToVersion);

public sealed record EventsBatch(ClientEventDto[] Events);

public sealed record EventsAccepted(int Accepted, int Duplicates, int Rejected, string[] Problems);

public sealed record DailyCount(DateOnly Day, int Count);

public sealed record PackageStats(
    string Id,
    int Installs,
    int InstalledBase,
    IReadOnlyList<DailyCount> InstallsByDay,
    IReadOnlyDictionary<string, int> AgentMix);

public sealed record ActiveUsers(int Day, int Week, int Month);

/// <summary>The caller's desktop app, from its latest heartbeat and the installs and uninstalls reported since.</summary>
public sealed record AppView(string Version, string Os, DateTime LastSeenAt, string[] Installed);

public sealed record TopPackage(string Id, int InstalledBase);

public sealed record AdminSummary(
    ActiveUsers ActiveUsers,
    int Publishers,
    int Packages,
    IReadOnlyDictionary<string, int> ClientVersions,
    IReadOnlyDictionary<string, int> AgentMix,
    IReadOnlyList<TopPackage> TopPackages,
    IReadOnlyDictionary<string, int> PreflightFailures,
    int OpenReports);

public sealed class EventsService(MarketplaceDbContext db, TimeProvider timeProvider)
{
    public const int MaxBatch = 500;
    private static readonly HashSet<string> Kinds = ["heartbeat", "install", "update", "uninstall"];
    private static readonly TimeSpan FutureTolerance = TimeSpan.FromMinutes(10);
    private static readonly TimeSpan MaxAge = TimeSpan.FromDays(30);

    /// <summary>An app not heard from for this long is treated as gone.</summary>
    private static readonly TimeSpan AppWindow = TimeSpan.FromDays(30);

    public async Task<EventsAccepted> RecordAsync(string account, EventsBatch batch, CancellationToken cancellationToken)
    {
        if (batch.Events.Length > MaxBatch)
        {
            throw new ProblemException(422, $"A batch holds at most {MaxBatch} events; this one has {batch.Events.Length}.");
        }

        var now = timeProvider.GetUtcNow();
        var received = now.UtcDateTime;
        var accepted = 0;
        var duplicates = 0;
        var problems = new List<string>();
        Heartbeat? heartbeat = null;
        var pending = new List<ClientEvent>();
        var seen = new HashSet<(string, DateTime, string?)>();

        foreach (var dto in batch.Events)
        {
            if (dto is null)
            {
                problems.Add("An event is null.");
                continue;
            }

            if (dto.Kind is null || !Kinds.Contains(dto.Kind))
            {
                problems.Add($"Unknown event kind {dto.Kind ?? "(none)"}.");
                continue;
            }

            if (dto.OccurredAt > now + FutureTolerance || dto.OccurredAt < now - MaxAge)
            {
                problems.Add($"{dto.Kind} at {dto.OccurredAt:O} is outside the accepted window.");
                continue;
            }

            if (string.IsNullOrWhiteSpace(dto.ClientVersion) || dto.ClientVersion.Length > 64)
            {
                problems.Add($"{dto.Kind} has no client version.");
                continue;
            }

            var occurredAt = dto.OccurredAt.UtcDateTime;
            var agents = Clean(dto.Agents, 16, 32);
            if (dto.Kind == "heartbeat")
            {
                var candidate = new Heartbeat
                {
                    Account = account,
                    OccurredAt = occurredAt,
                    ClientVersion = dto.ClientVersion,
                    OsBuild = (dto.OsBuild ?? string.Empty).Length <= 120 ? dto.OsBuild ?? string.Empty : dto.OsBuild![..120],
                    Agents = agents,
                    Installed = Clean(dto.Installed, 2000, 81),
                    ChecksJson = JsonSerializer.Serialize(CleanChecks(dto.Checks)),
                    ReceivedAt = received,
                };
                if (heartbeat is null || candidate.OccurredAt > heartbeat.OccurredAt)
                {
                    heartbeat = candidate;
                }

                accepted++;
                continue;
            }

            if (string.IsNullOrWhiteSpace(dto.PackageId) || dto.PackageId.Length > 81 || !dto.PackageId.Contains('/'))
            {
                problems.Add($"{dto.Kind} has no canonical package id.");
                continue;
            }

            if (!seen.Add((dto.Kind, occurredAt, dto.PackageId)))
            {
                duplicates++;
                continue;
            }

            pending.Add(new ClientEvent
            {
                Account = account,
                Kind = dto.Kind,
                OccurredAt = occurredAt,
                ClientVersion = dto.ClientVersion,
                PackageId = dto.PackageId,
                Version = Clip(dto.Kind == "update" ? dto.ToVersion ?? dto.Version : dto.Version, 64),
                FromVersion = Clip(dto.FromVersion, 64),
                Agents = agents,
                ReceivedAt = received,
            });
        }

        if (pending.Count > 0)
        {
            var kinds = pending.Select(item => item.Kind).Distinct().ToArray();
            var earliest = pending.Min(item => item.OccurredAt);
            var existing = await db.Events
                .Where(item => item.Account == account && kinds.Contains(item.Kind) && item.OccurredAt >= earliest)
                .Select(item => new { item.Kind, item.OccurredAt, item.PackageId })
                .ToListAsync(cancellationToken);
            var existingKeys = existing.Select(item => (item.Kind, item.OccurredAt, item.PackageId)).ToHashSet();
            foreach (var item in pending)
            {
                if (existingKeys.Contains((item.Kind, item.OccurredAt, item.PackageId)))
                {
                    duplicates++;
                    continue;
                }

                db.Events.Add(item);
                accepted++;
            }
        }

        if (heartbeat is not null)
        {
            var current = await db.Heartbeats.FindAsync([account], cancellationToken);
            if (current is null)
            {
                db.Heartbeats.Add(heartbeat);
            }
            else if (heartbeat.OccurredAt >= current.OccurredAt)
            {
                current.OccurredAt = heartbeat.OccurredAt;
                current.ClientVersion = heartbeat.ClientVersion;
                current.OsBuild = heartbeat.OsBuild;
                current.Agents = heartbeat.Agents;
                current.Installed = heartbeat.Installed;
                current.ChecksJson = heartbeat.ChecksJson;
                current.ReceivedAt = heartbeat.ReceivedAt;
            }
        }

        await db.SaveChangesAsync(cancellationToken);
        return new EventsAccepted(accepted, duplicates, problems.Count, problems.ToArray());
    }

    public async Task<AppView?> AppAsync(string account, CancellationToken cancellationToken)
    {
        var heartbeat = await db.Heartbeats.AsNoTracking().SingleOrDefaultAsync(candidate => candidate.Account == account, cancellationToken);
        if (heartbeat is null || heartbeat.OccurredAt < timeProvider.GetUtcNow().UtcDateTime - AppWindow)
        {
            return null;
        }

        var changes = await db.Events.AsNoTracking()
            .Where(item => item.Account == account && item.OccurredAt > heartbeat.OccurredAt && (item.Kind == "install" || item.Kind == "uninstall"))
            .OrderBy(item => item.OccurredAt)
            .Select(item => new { item.Kind, item.PackageId })
            .ToListAsync(cancellationToken);
        var installed = heartbeat.Installed.ToList();
        foreach (var change in changes)
        {
            installed.Remove(change.PackageId!);
            if (change.Kind == "install")
            {
                installed.Add(change.PackageId!);
            }
        }

        return new AppView(heartbeat.ClientVersion, heartbeat.OsBuild, heartbeat.OccurredAt, installed.ToArray());
    }

    public async Task<PackageStats> PackageStatsAsync(string canonicalId, CancellationToken cancellationToken)
    {
        var now = timeProvider.GetUtcNow().UtcDateTime;
        var since = now.AddDays(-90);
        var installs = await db.Events
            .Where(item => item.Kind == "install" && item.PackageId == canonicalId)
            .Select(item => new { item.OccurredAt, item.Agents })
            .ToListAsync(cancellationToken);
        var installedBase = await db.Heartbeats
            .CountAsync(heartbeat => heartbeat.OccurredAt >= now.AddDays(-30) && heartbeat.Installed.Contains(canonicalId), cancellationToken);
        var byDay = installs
            .Where(item => item.OccurredAt >= since)
            .GroupBy(item => DateOnly.FromDateTime(item.OccurredAt))
            .Select(group => new DailyCount(group.Key, group.Count()))
            .OrderBy(day => day.Day)
            .ToArray();
        var agentMix = installs
            .Where(item => item.OccurredAt >= since)
            .SelectMany(item => item.Agents)
            .GroupBy(agent => agent, StringComparer.Ordinal)
            .ToDictionary(group => group.Key, group => group.Count(), StringComparer.Ordinal);
        return new PackageStats(canonicalId, installs.Count, installedBase, byDay, agentMix);
    }

    public async Task<AdminSummary> AdminSummaryAsync(CancellationToken cancellationToken)
    {
        var now = timeProvider.GetUtcNow().UtcDateTime;
        var heartbeats = await db.Heartbeats.ToListAsync(cancellationToken);
        var eventAccounts = await db.Events
            .Where(item => item.OccurredAt >= now.AddDays(-30))
            .Select(item => new { item.Account, item.OccurredAt })
            .ToListAsync(cancellationToken);
        int Active(int days)
        {
            var since = now.AddDays(-days);
            return heartbeats.Where(heartbeat => heartbeat.OccurredAt >= since).Select(heartbeat => heartbeat.Account)
                .Concat(eventAccounts.Where(item => item.OccurredAt >= since).Select(item => item.Account))
                .Distinct(StringComparer.OrdinalIgnoreCase)
                .Count();
        }

        var recent = heartbeats.Where(heartbeat => heartbeat.OccurredAt >= now.AddDays(-30)).ToArray();
        var preflight = new Dictionary<string, int>(StringComparer.Ordinal);
        foreach (var heartbeat in heartbeats.Where(heartbeat => heartbeat.OccurredAt >= now.AddDays(-7)))
        {
            Dictionary<string, string>? checks;
            try
            {
                checks = JsonSerializer.Deserialize<Dictionary<string, string>>(heartbeat.ChecksJson);
            }
            catch (JsonException)
            {
                continue;
            }

            foreach (var (id, status) in checks ?? [])
            {
                if (status == "fail")
                {
                    preflight[id] = preflight.GetValueOrDefault(id) + 1;
                }
            }
        }

        var installedBase = recent
            .SelectMany(heartbeat => heartbeat.Installed.Distinct(StringComparer.Ordinal))
            .GroupBy(id => id, StringComparer.Ordinal)
            .Select(group => new TopPackage(group.Key, group.Count()))
            .OrderByDescending(top => top.InstalledBase)
            .ThenBy(top => top.Id, StringComparer.Ordinal)
            .Take(20)
            .ToArray();
        return new AdminSummary(
            new ActiveUsers(Active(1), Active(7), Active(30)),
            await db.Publishers.CountAsync(cancellationToken),
            await db.Packages.CountAsync(package => package.RevokedAt == null && package.Versions.Any(version => !version.Yanked), cancellationToken),
            recent.GroupBy(heartbeat => heartbeat.ClientVersion, StringComparer.Ordinal).ToDictionary(group => group.Key, group => group.Count(), StringComparer.Ordinal),
            recent.SelectMany(heartbeat => heartbeat.Agents.Distinct(StringComparer.Ordinal)).GroupBy(agent => agent, StringComparer.Ordinal).ToDictionary(group => group.Key, group => group.Count(), StringComparer.Ordinal),
            installedBase,
            preflight,
            await db.Reports.CountAsync(report => report.ResolvedAt == null, cancellationToken));
    }

    private static string[] Clean(string[]? values, int maxCount, int maxLength) =>
        (values ?? [])
            .Select(value => value?.Trim() ?? string.Empty)
            .Where(value => value.Length is > 0 and var length && length <= maxLength)
            .Distinct(StringComparer.Ordinal)
            .Take(maxCount)
            .ToArray();

    private static Dictionary<string, string> CleanChecks(Dictionary<string, string>? checks) =>
        (checks ?? [])
            .Where(pair => pair.Key.Length is > 0 and <= 64 && pair.Value is "ok" or "warn" or "fail" or "skipped")
            .Take(200)
            .ToDictionary(pair => pair.Key, pair => pair.Value, StringComparer.Ordinal);

    private static string? Clip(string? value, int max) =>
        value is null ? null : value.Length <= max ? value : value[..max];
}
