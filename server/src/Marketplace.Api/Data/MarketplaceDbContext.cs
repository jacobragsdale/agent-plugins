using Microsoft.EntityFrameworkCore;

namespace Marketplace.Api.Data;

public sealed class MarketplaceDbContext(DbContextOptions<MarketplaceDbContext> options) : DbContext(options)
{
    public DbSet<Publisher> Publishers => Set<Publisher>();

    public DbSet<Package> Packages => Set<Package>();

    public DbSet<PackageVersion> PackageVersions => Set<PackageVersion>();

    public DbSet<NamespaceArchive> NamespaceArchives => Set<NamespaceArchive>();

    public DbSet<ClientEvent> Events => Set<ClientEvent>();

    public DbSet<Heartbeat> Heartbeats => Set<Heartbeat>();

    public DbSet<PackageReport> Reports => Set<PackageReport>();

    public DbSet<AccessRule> AccessRules => Set<AccessRule>();

    public DbSet<TeamMember> TeamMembers => Set<TeamMember>();

    public DbSet<Person> People => Set<Person>();

    public DbSet<Link> Links => Set<Link>();

    public DbSet<Suggestion> Suggestions => Set<Suggestion>();

    public DbSet<Bundle> Bundles => Set<Bundle>();

    public DbSet<Notification> Notifications => Set<Notification>();

    public DbSet<AuditEvent> AuditEvents => Set<AuditEvent>();

    public DbSet<Block> Blocks => Set<Block>();

    /// <summary>Records who changed what; saved with the caller's next <c>SaveChanges</c>.</summary>
    public void Audit(string actor, string action, string target, string? detail, DateTime at) =>
        AuditEvents.Add(new AuditEvent
        {
            At = at,
            Actor = actor,
            Action = action,
            Target = target.Length <= 256 ? target : target[..256],
            Detail = detail is null || detail.Length <= 1024 ? detail : detail[..1024],
        });

    /// <summary>Serializes writes to one namespace until the surrounding transaction ends.</summary>
    public async Task LockNamespaceAsync(string ns, CancellationToken cancellationToken) =>
        await Database.ExecuteSqlAsync($"SELECT pg_advisory_xact_lock(hashtext({ns}))", cancellationToken);

    protected override void OnModelCreating(ModelBuilder modelBuilder)
    {
        modelBuilder.Entity<Publisher>(entity =>
        {
            entity.HasKey(publisher => publisher.Namespace);
            entity.Property(publisher => publisher.Namespace).HasMaxLength(16);
            entity.Property(publisher => publisher.Account).HasMaxLength(256);
            entity.Property(publisher => publisher.DisplayName).HasMaxLength(120);
            entity.Property(publisher => publisher.Kind).HasConversion<string>().HasMaxLength(16);
            entity.HasIndex(publisher => publisher.Account);
        });

        modelBuilder.Entity<TeamMember>(entity =>
        {
            entity.HasKey(member => new { member.Namespace, member.Account });
            entity.Property(member => member.Namespace).HasMaxLength(16);
            entity.Property(member => member.Account).HasMaxLength(256);
            entity.HasIndex(member => member.Account);
        });

        modelBuilder.Entity<Person>(entity =>
        {
            entity.HasKey(person => person.Account);
            entity.Property(person => person.Account).HasMaxLength(256);
            entity.Property(person => person.DisplayName).HasMaxLength(120);
        });

        modelBuilder.Entity<Link>(entity =>
        {
            entity.HasKey(link => link.Code);
            entity.Property(link => link.Code).HasMaxLength(32);
            entity.Property(link => link.Kind).HasMaxLength(16);
            entity.Property(link => link.Target).HasMaxLength(81);
            entity.Property(link => link.CreatedBy).HasMaxLength(256);
            entity.HasIndex(link => new { link.Kind, link.Target }).IsUnique();
        });

        modelBuilder.Entity<Suggestion>(entity =>
        {
            entity.Property(suggestion => suggestion.StoragePath).HasMaxLength(512);
            entity.Property(suggestion => suggestion.ArchiveDigest).HasMaxLength(64);
            entity.Property(suggestion => suggestion.ManifestJson).HasColumnType("jsonb");
            entity.Property(suggestion => suggestion.BaseVersion).HasMaxLength(64);
            entity.Property(suggestion => suggestion.Message).HasMaxLength(4096);
            entity.Property(suggestion => suggestion.SuggestedBy).HasMaxLength(256);
            entity.Property(suggestion => suggestion.State).HasConversion<string>().HasMaxLength(16);
            entity.Property(suggestion => suggestion.DecidedBy).HasMaxLength(256);
            entity.Property(suggestion => suggestion.DecisionNote).HasMaxLength(2048);
            entity.Property(suggestion => suggestion.AcceptedVersion).HasMaxLength(64);
            entity.HasIndex(suggestion => suggestion.State);
            entity.HasOne(suggestion => suggestion.Package)
                .WithMany()
                .HasForeignKey(suggestion => suggestion.PackageId)
                .OnDelete(DeleteBehavior.Cascade);
        });

        modelBuilder.Entity<Bundle>(entity =>
        {
            entity.HasKey(bundle => new { bundle.Namespace, bundle.BundleId });
            entity.Property(bundle => bundle.Namespace).HasMaxLength(16);
            entity.Property(bundle => bundle.BundleId).HasMaxLength(64);
            entity.Property(bundle => bundle.Name).HasMaxLength(120);
            entity.Property(bundle => bundle.Description).HasMaxLength(1024);
            entity.Property(bundle => bundle.UpdatedBy).HasMaxLength(256);
            entity.Ignore(bundle => bundle.CanonicalId);
        });

        modelBuilder.Entity<Package>(entity =>
        {
            entity.Property(package => package.Namespace).HasMaxLength(16);
            entity.Property(package => package.PackageId).HasMaxLength(64);
            entity.Property(package => package.Name).HasMaxLength(120);
            entity.Property(package => package.Description).HasMaxLength(1024);
            entity.Property(package => package.RevokedBy).HasMaxLength(256);
            entity.Property(package => package.McpApprovedBy).HasMaxLength(256);
            entity.Property(package => package.McpApprovedSpec).HasMaxLength(8192);
            entity.Property(package => package.McpDeclineNote).HasMaxLength(2048);
            entity.HasIndex(package => new { package.Namespace, package.PackageId }).IsUnique();
            entity.Ignore(package => package.CanonicalId);
        });

        modelBuilder.Entity<PackageVersion>(entity =>
        {
            entity.Property(version => version.Version).HasMaxLength(64);
            entity.Property(version => version.StoragePath).HasMaxLength(512);
            entity.Property(version => version.ArchiveDigest).HasMaxLength(64);
            entity.Property(version => version.ManifestJson).HasColumnType("jsonb");
            entity.Property(version => version.PublishedBy).HasMaxLength(256);
            entity.Property(version => version.Changelog).HasMaxLength(4096);
            entity.Property(version => version.McpServersJson).HasColumnType("jsonb");
            entity.Property(version => version.PurgedBy).HasMaxLength(256);
            entity.HasIndex(version => new { version.PackageId, version.Version }).IsUnique();
            entity.HasOne(version => version.Package)
                .WithMany(package => package.Versions)
                .HasForeignKey(version => version.PackageId)
                .OnDelete(DeleteBehavior.Cascade);
        });

        modelBuilder.Entity<NamespaceArchive>(entity =>
        {
            entity.HasKey(archive => archive.Namespace);
            entity.Property(archive => archive.Namespace).HasMaxLength(16);
            entity.Property(archive => archive.Digest).HasMaxLength(64);
        });

        modelBuilder.Entity<ClientEvent>(entity =>
        {
            entity.Property(clientEvent => clientEvent.Account).HasMaxLength(256);
            entity.Property(clientEvent => clientEvent.Kind).HasMaxLength(16);
            entity.Property(clientEvent => clientEvent.ClientVersion).HasMaxLength(64);
            entity.Property(clientEvent => clientEvent.PackageId).HasMaxLength(81);
            entity.Property(clientEvent => clientEvent.Version).HasMaxLength(64);
            entity.Property(clientEvent => clientEvent.FromVersion).HasMaxLength(64);
            entity.HasIndex(clientEvent => new { clientEvent.Account, clientEvent.Kind, clientEvent.OccurredAt, clientEvent.PackageId }).IsUnique();
            entity.HasIndex(clientEvent => new { clientEvent.Kind, clientEvent.PackageId, clientEvent.OccurredAt });
        });

        modelBuilder.Entity<Heartbeat>(entity =>
        {
            entity.HasKey(heartbeat => new { heartbeat.Account, heartbeat.Device });
            entity.Property(heartbeat => heartbeat.Account).HasMaxLength(256);
            entity.Property(heartbeat => heartbeat.Device).HasMaxLength(120);
            entity.Property(heartbeat => heartbeat.InstalledVersionsJson).HasColumnType("jsonb");
            entity.Property(heartbeat => heartbeat.ClientVersion).HasMaxLength(64);
            entity.Property(heartbeat => heartbeat.OsBuild).HasMaxLength(120);
            entity.Property(heartbeat => heartbeat.ChecksJson).HasColumnType("jsonb");
        });

        modelBuilder.Entity<AccessRule>(entity =>
        {
            entity.HasKey(rule => rule.Target);
            entity.Property(rule => rule.Target).HasMaxLength(81);
            entity.Property(rule => rule.Visibility).HasConversion<string>().HasMaxLength(16);
            entity.Property(rule => rule.UpdatedBy).HasMaxLength(256);
            entity.Ignore(rule => rule.HasLink);
        });

        modelBuilder.Entity<PackageReport>(entity =>
        {
            entity.Property(report => report.Account).HasMaxLength(256);
            entity.Property(report => report.PackageId).HasMaxLength(81);
            entity.Property(report => report.Kind).HasMaxLength(16);
            entity.Property(report => report.Reason).HasMaxLength(2048);
            entity.Property(report => report.ResolvedBy).HasMaxLength(256);
            entity.Property(report => report.Note).HasMaxLength(2048);
            entity.HasIndex(report => report.PackageId);
        });

        modelBuilder.Entity<Notification>(entity =>
        {
            entity.Property(notification => notification.Account).HasMaxLength(256);
            entity.Property(notification => notification.Kind).HasMaxLength(32);
            entity.Property(notification => notification.Text).HasMaxLength(512);
            entity.Property(notification => notification.Link).HasMaxLength(256);
            entity.HasIndex(notification => new { notification.Account, notification.Id });
        });

        modelBuilder.Entity<AuditEvent>(entity =>
        {
            entity.Property(audit => audit.Actor).HasMaxLength(256);
            entity.Property(audit => audit.Action).HasMaxLength(32);
            entity.Property(audit => audit.Target).HasMaxLength(256);
            entity.Property(audit => audit.Detail).HasMaxLength(1024);
        });

        modelBuilder.Entity<Block>(entity =>
        {
            entity.HasKey(block => block.Account);
            entity.Property(block => block.Account).HasMaxLength(256);
            entity.Property(block => block.BlockedBy).HasMaxLength(256);
            entity.Property(block => block.Reason).HasMaxLength(1024);
        });
    }
}
