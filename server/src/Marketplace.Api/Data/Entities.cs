namespace Marketplace.Api.Data;

/// <summary>
/// A claimed namespace: a person's, once they publish; <c>official</c>, once someone publishes there; or a
/// team, from the moment it is created. A team's <see cref="Account"/> is the namespace itself.
/// </summary>
public sealed class Publisher
{
    public required string Namespace { get; set; }

    public required string Account { get; set; }

    public required string DisplayName { get; set; }

    public PublisherKind Kind { get; set; }

    public DateTime FirstSeenAt { get; set; }

    public DateTime LastPublishedAt { get; set; }
}

public enum PublisherKind
{
    Personal,
    Team,
    Official,
}

/// <summary>
/// A team member. <see cref="Account"/> is stored as given: <c>CORP\jane</c> matches that account only,
/// a bare <c>jane</c> matches any account whose username is jane.
/// </summary>
public sealed class TeamMember
{
    public required string Namespace { get; set; }

    public required string Account { get; set; }

    public bool IsOwner { get; set; }

    public DateTime JoinedAt { get; set; }
}

/// <summary>Everyone who has signed in, for display names and the people search. Touched at most daily.</summary>
public sealed class Person
{
    public required string Account { get; set; }

    public required string DisplayName { get; set; }

    public DateTime LastSeenAt { get; set; }
}

/// <summary>A team invite (<c>ns</c>) or a share link (<c>ns</c>, <c>ns/package</c>, or <c>ns/bundle</c>).</summary>
public sealed class Link
{
    public const string Invite = "invite";
    public const string Share = "share";

    public required string Code { get; set; }

    public required string Kind { get; set; }

    public required string Target { get; set; }

    public required string CreatedBy { get; set; }

    public DateTime CreatedAt { get; set; }
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

    /// <summary>Pulled from every PC: it leaves the catalog, and clients uninstall it at their next sync.</summary>
    public DateTime? RevokedAt { get; set; }

    public string? RevokedBy { get; set; }

    /// <summary>An admin let the public see this package's MCP server. Later versions keep it.</summary>
    public string? McpApprovedBy { get; set; }

    public DateTime? McpApprovedAt { get; set; }

    /// <summary>Why an admin kept the MCP server from the public; the next publish clears it.</summary>
    public string? McpDeclineNote { get; set; }

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
/// Who may see and install a namespace (<c>ns</c>), a package, or a bundle (<c>ns/id</c>). No row means
/// public for a namespace and <see cref="Visibility.Inherit"/> for anything in it. The share list is kept
/// whatever the visibility, so switching back to private restores it.
/// </summary>
public sealed class AccessRule
{
    public required string Target { get; set; }

    public Visibility Visibility { get; set; }

    public string[] Users { get; set; } = [];

    public string[] Teams { get; set; } = [];

    public string[] Groups { get; set; } = [];

    public required string UpdatedBy { get; set; }

    public DateTime UpdatedAt { get; set; }
}

/// <summary>A namespace is public or private; a package or bundle can also follow its namespace.</summary>
public enum Visibility
{
    Inherit,
    Public,
    Private,
}

/// <summary>Files someone who does not own a package proposes as its next version; its owners accept or decline.</summary>
public sealed class Suggestion
{
    public long Id { get; set; }

    public int PackageId { get; set; }

    public Package Package { get; set; } = null!;

    /// <summary>Path inside the artifact store, relative to the configured prefix.</summary>
    public string StoragePath { get; set; } = string.Empty;

    public required string ArchiveDigest { get; set; }

    public long SizeBytes { get; set; }

    public required string ManifestJson { get; set; }

    public string[] ComponentKinds { get; set; } = [];

    /// <summary>The live version the suggestion was made against.</summary>
    public string? BaseVersion { get; set; }

    public required string Message { get; set; }

    public required string SuggestedBy { get; set; }

    public DateTime SuggestedAt { get; set; }

    public SuggestionState State { get; set; }

    public string? DecidedBy { get; set; }

    public DateTime? DecidedAt { get; set; }

    public string? DecisionNote { get; set; }

    public string? AcceptedVersion { get; set; }
}

public enum SuggestionState
{
    Pending,
    Accepted,
    Declined,
    Withdrawn,
}

/// <summary>A named list of live packages, from any namespace, installed together. It has no archive or versions.</summary>
public sealed class Bundle
{
    public required string Namespace { get; set; }

    public required string BundleId { get; set; }

    public required string Name { get; set; }

    public string Description { get; set; } = string.Empty;

    public string[] Members { get; set; } = [];

    public required string UpdatedBy { get; set; }

    public DateTime CreatedAt { get; set; }

    public DateTime UpdatedAt { get; set; }

    public string CanonicalId => $"{Namespace}/{BundleId}";
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
