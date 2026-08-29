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

    protected override void OnModelCreating(ModelBuilder modelBuilder)
    {
        modelBuilder.Entity<Publisher>(entity =>
        {
            entity.HasKey(publisher => publisher.Namespace);
            entity.Property(publisher => publisher.Namespace).HasMaxLength(16);
            entity.Property(publisher => publisher.Account).HasMaxLength(256);
            entity.Property(publisher => publisher.DisplayName).HasMaxLength(120);
            entity.HasIndex(publisher => publisher.Account);
        });

        modelBuilder.Entity<Package>(entity =>
        {
            entity.Property(package => package.Namespace).HasMaxLength(16);
            entity.Property(package => package.PackageId).HasMaxLength(64);
            entity.Property(package => package.Name).HasMaxLength(120);
            entity.Property(package => package.Description).HasMaxLength(1024);
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
            entity.HasKey(heartbeat => heartbeat.Account);
            entity.Property(heartbeat => heartbeat.Account).HasMaxLength(256);
            entity.Property(heartbeat => heartbeat.ClientVersion).HasMaxLength(64);
            entity.Property(heartbeat => heartbeat.OsBuild).HasMaxLength(120);
            entity.Property(heartbeat => heartbeat.ChecksJson).HasColumnType("jsonb");
        });

        modelBuilder.Entity<PackageReport>(entity =>
        {
            entity.Property(report => report.Account).HasMaxLength(256);
            entity.Property(report => report.PackageId).HasMaxLength(81);
            entity.Property(report => report.Reason).HasMaxLength(2048);
        });
    }
}
