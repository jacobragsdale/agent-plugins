namespace Marketplace.Api.Data;

/// <summary>A publisher namespace: one per Windows account that has published.</summary>
public sealed class Publisher
{
    public required string Namespace { get; set; }

    public required string Account { get; set; }

    public required string DisplayName { get; set; }

    public DateTime FirstSeenAt { get; set; }

    public DateTime LastPublishedAt { get; set; }
}

public sealed class Package
{
    public int Id { get; set; }

    public required string Namespace { get; set; }

    public required string PackageId { get; set; }

    public required string Name { get; set; }

    public required string Description { get; set; }

    public string[] Tags { get; set; } = [];

    public DateTime CreatedAt { get; set; }

    public DateTime UpdatedAt { get; set; }

    public List<PackageVersion> Versions { get; set; } = [];

    public string CanonicalId => $"{Namespace}/{PackageId}";
}

public sealed class PackageVersion
{
    public int Id { get; set; }

    public int PackageId { get; set; }

    public Package Package { get; set; } = null!;

    public required string Version { get; set; }

    /// <summary>Path inside the artifact store, relative to the configured prefix.</summary>
    public required string StoragePath { get; set; }

    public required string ArchiveDigest { get; set; }

    public long SizeBytes { get; set; }

    /// <summary>The uploaded package's manifest entry, as JSON, with unprefixed component paths.</summary>
    public required string ManifestJson { get; set; }

    public string[] ComponentKinds { get; set; } = [];

    /// <summary>The tags sent with this version; they reach the package listing when the version is approved.</summary>
    public string[] Tags { get; set; } = [];

    public required string PublishedBy { get; set; }

    public DateTime PublishedAt { get; set; }

    public string? Changelog { get; set; }

    public bool Yanked { get; set; }

    /// <summary>Pending versions are visible only to their owners and admins until an admin approves them.</summary>
    public ReviewState ReviewState { get; set; }

    public string? ReviewedBy { get; set; }

    public DateTime? ReviewedAt { get; set; }

    /// <summary>The reviewer's note to the publisher; required when a version is rejected.</summary>
    public string? ReviewNote { get; set; }
}

public enum ReviewState
{
    Pending,
    Approved,
    Rejected,
}

/// <summary>The generated source archive a client downloads for one namespace.</summary>
public sealed class NamespaceArchive
{
    public required string Namespace { get; set; }

    public required byte[] Bytes { get; set; }

    public required string Digest { get; set; }

    public int PackageCount { get; set; }

    public DateTime GeneratedAt { get; set; }
}

public sealed class ClientEvent
{
    public long Id { get; set; }

    public required string Account { get; set; }

    public required string Kind { get; set; }

    public DateTime OccurredAt { get; set; }

    public required string ClientVersion { get; set; }

    public string? PackageId { get; set; }

    public string? Version { get; set; }

    public string? FromVersion { get; set; }

    public string[] Agents { get; set; } = [];

    public DateTime ReceivedAt { get; set; }
}

/// <summary>The latest heartbeat per account.</summary>
public sealed class Heartbeat
{
    public required string Account { get; set; }

    public DateTime OccurredAt { get; set; }

    public required string ClientVersion { get; set; }

    public required string OsBuild { get; set; }

    public string[] Agents { get; set; } = [];

    public string[] Installed { get; set; } = [];

    /// <summary>Preflight check statuses as a JSON object of id to status.</summary>
    public required string ChecksJson { get; set; }

    public DateTime ReceivedAt { get; set; }
}

/// <summary>
/// Who may see and install a namespace (<c>ns</c>) or one package (<c>ns/package</c>). No row means
/// public; a row is an allowlist of accounts and AD groups.
/// </summary>
public sealed class AccessRule
{
    public required string Target { get; set; }

    public string[] Users { get; set; } = [];

    public string[] Groups { get; set; } = [];

    public required string UpdatedBy { get; set; }

    public DateTime UpdatedAt { get; set; }
}

public sealed class PackageReport
{
    public long Id { get; set; }

    public required string Account { get; set; }

    public required string PackageId { get; set; }

    public required string Reason { get; set; }

    public DateTime CreatedAt { get; set; }

    public DateTime? ResolvedAt { get; set; }

    public string? ResolvedBy { get; set; }
}
