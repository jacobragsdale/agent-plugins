using System.Security.Cryptography;
using Marketplace.Api.Packages;
using Marketplace.Api.Storage;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Npgsql;
using Testcontainers.PostgreSql;
using Xunit;

namespace Marketplace.Api.Tests;

/// <summary>
/// Hosts the API in Development mode against a throwaway PostgreSQL container, with an in-memory
/// artifact store. The validator is the real Rust binary when <c>MARKETPLACE_VALIDATOR</c> names it
/// (or the debug build exists); otherwise a permissive fake so the suite runs without cargo.
/// </summary>
public sealed class MarketplaceApiFactory : WebApplicationFactory<Program>, IAsyncLifetime
{
    private readonly PostgreSqlContainer _postgres = new PostgreSqlBuilder("postgres:16-alpine")
        .WithDatabase("marketplace")
        .WithUsername("marketplace")
        .WithPassword("marketplace")
        .Build();

    public InMemoryArtifactStore Store { get; } = new();

    public string? ValidatorPath { get; } = LocateValidator();

    /// <summary>Stands in for the Angular build and the installer folder.</summary>
    public string SiteRoot { get; } = Directory.CreateTempSubdirectory("marketplace-site-").FullName;

    public async ValueTask InitializeAsync() => await _postgres.StartAsync();

    /// <summary>A second, empty database on the same server, for tests that drive migrations themselves.</summary>
    public async Task<string> CreateDatabaseAsync(string name)
    {
        await using var connection = new NpgsqlConnection(_postgres.GetConnectionString());
        await connection.OpenAsync();
        await using var command = new NpgsqlCommand($"CREATE DATABASE \"{name}\"", connection);
        await command.ExecuteNonQueryAsync();
        return new NpgsqlConnectionStringBuilder(_postgres.GetConnectionString()) { Database = name }.ConnectionString;
    }

    public override async ValueTask DisposeAsync()
    {
        await base.DisposeAsync();
        await _postgres.DisposeAsync();
        Directory.Delete(SiteRoot, recursive: true);
    }

    protected override void ConfigureWebHost(IWebHostBuilder builder)
    {
        builder.UseEnvironment("Development");
        var webRoot = Directory.CreateDirectory(Path.Combine(SiteRoot, "wwwroot")).FullName;
        File.WriteAllText(Path.Combine(webRoot, "index.html"), "<!doctype html><title>Agent Plugins</title>");
        var downloads = Directory.CreateDirectory(Path.Combine(SiteRoot, "downloads", "releases", "0.1.0")).Parent!.Parent!.FullName;
        File.WriteAllText(Path.Combine(downloads, "manifest.json"), "{}");
        File.WriteAllText(Path.Combine(downloads, "releases", "0.1.0", "Agent-Plugins.AppImage"), "binary");
        builder.UseWebRoot(webRoot);
        builder.UseSetting("Server:DownloadsPath", downloads);
        builder.UseSetting("ConnectionStrings:Marketplace", _postgres.GetConnectionString());
        builder.UseSetting("Server:PublicBaseUrl", "https://marketplace.test");
        builder.UseSetting("Auth:EnableNegotiate", "false");
        builder.UseSetting("Auth:AdminAccounts:0", "TEST\\admin");
        builder.UseSetting("Auth:OfficialPublishers:0", "TEST\\curator");
        builder.UseSetting("Client:MinimumVersion", "0.1.0");
        builder.UseSetting("Client:LatestVersion", "0.2.0");
        builder.UseSetting("RateLimits:WritesPerMinute", "100000");
        builder.UseSetting("RateLimits:UploadsPerHour", "100000");
        if (ValidatorPath is not null)
        {
            builder.UseSetting("Validator:Path", ValidatorPath);
        }

        builder.ConfigureServices(services =>
        {
            services.RemoveAll<IArtifactStore>();
            services.AddSingleton<IArtifactStore>(Store);
            if (ValidatorPath is null)
            {
                services.RemoveAll<IPackageValidator>();
                services.AddSingleton<IPackageValidator, PermissiveValidator>();
            }
        });
    }

    public HttpClient ClientFor(string account, params string[] groups)
    {
        var client = CreateClient();
        client.DefaultRequestHeaders.Add("X-Dev-User", account);
        if (groups.Length > 0)
        {
            client.DefaultRequestHeaders.Add("X-Dev-Groups", string.Join(',', groups));
        }

        return client;
    }

    private static string? LocateValidator()
    {
        var configured = Environment.GetEnvironmentVariable("MARKETPLACE_VALIDATOR");
        if (!string.IsNullOrEmpty(configured) && File.Exists(configured))
        {
            return configured;
        }

        var directory = AppContext.BaseDirectory;
        for (var depth = 0; depth < 8 && directory is not null; depth++)
        {
            var candidate = Path.Combine(directory, "src-tauri", "target", "debug", OperatingSystem.IsWindows() ? "validate-source.exe" : "validate-source");
            if (File.Exists(candidate))
            {
                return candidate;
            }

            directory = Path.GetDirectoryName(directory);
        }

        return null;
    }
}

public sealed class InMemoryArtifactStore : IArtifactStore
{
    private readonly Dictionary<string, byte[]> _blobs = new(StringComparer.Ordinal);
    private readonly HashSet<string> _cancelableDeletes = new(StringComparer.Ordinal);
    private readonly HashSet<string> _deleted = new(StringComparer.Ordinal);
    private readonly Lock _lock = new();

    public IReadOnlyCollection<string> Paths
    {
        get
        {
            lock (_lock)
            {
                return _blobs.Keys.ToArray();
            }
        }
    }

    public Task<StoredArtifact> PutAsync(string path, ReadOnlyMemory<byte> bytes, CancellationToken cancellationToken)
    {
        lock (_lock)
        {
            // Like Artifact Keeper, a deleted artifact keeps its path reserved.
            if (_deleted.Contains(path) || !_blobs.TryAdd(path, bytes.ToArray()))
            {
                throw new ArtifactConflictException(path);
            }
        }

        // Artifact Keeper answers with its full path, prefix included, which callers must not store as theirs.
        return Task.FromResult(new StoredArtifact($"prefix/{path}", Convert.ToHexStringLower(SHA256.HashData(bytes.Span)), bytes.Length));
    }

    public Task DeleteAsync(string path, CancellationToken cancellationToken)
    {
        lock (_lock)
        {
            if (_blobs.Remove(path))
            {
                _deleted.Add(path);
            }

            if (cancellationToken.CanBeCanceled)
            {
                _cancelableDeletes.Add(path);
            }
        }

        return Task.CompletedTask;
    }

    /// <summary>Whether <paramref name="path"/> was deleted with a token a hung-up request could cancel.</summary>
    public bool DeletedCancelably(string path)
    {
        lock (_lock)
        {
            return _cancelableDeletes.Contains(path);
        }
    }

    public Task<bool> CheckAsync(CancellationToken cancellationToken) => Task.FromResult(true);

    public Task<byte[]> GetAsync(string path, CancellationToken cancellationToken)
    {
        lock (_lock)
        {
            return _blobs.TryGetValue(path, out var bytes)
                ? Task.FromResult(bytes)
                : throw new InvalidOperationException($"No artifact at {path}.");
        }
    }
}

public sealed class PermissiveValidator : IPackageValidator
{
    public Task<ValidationOutcome> ValidateAsync(string sourceDirectory, CancellationToken cancellationToken)
    {
        var manifest = Path.Combine(sourceDirectory, "agent-plugins.json");
        return Task.FromResult(File.Exists(manifest)
            ? new ValidationOutcome(true, [])
            : ValidationOutcome.Fatal("agent-plugins.json is missing."));
    }

    public Task StageAsync(StagingRequest request, CancellationToken cancellationToken) =>
        throw new NotSupportedException("Wrapping uploads needs the Rust validator; build validate-source.");
}
