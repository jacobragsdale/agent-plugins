using System.Security.Cryptography;
using Marketplace.Api.Packages;
using Marketplace.Api.Storage;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
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

    public async ValueTask InitializeAsync() => await _postgres.StartAsync();

    public override async ValueTask DisposeAsync()
    {
        await base.DisposeAsync();
        await _postgres.DisposeAsync();
    }

    protected override void ConfigureWebHost(IWebHostBuilder builder)
    {
        builder.UseEnvironment("Development");
        builder.UseSetting("ConnectionStrings:Marketplace", _postgres.GetConnectionString());
        builder.UseSetting("Server:PublicBaseUrl", "https://marketplace.test");
        builder.UseSetting("Auth:EnableNegotiate", "false");
        builder.UseSetting("Auth:AdminAccounts:0", "TEST\\admin");
        builder.UseSetting("Auth:OfficialPublishers:0", "TEST\\curator");
        builder.UseSetting("Client:MinimumVersion", "0.1.0");
        builder.UseSetting("Client:LatestVersion", "0.2.0");
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
            if (!_blobs.TryAdd(path, bytes.ToArray()))
            {
                throw new ArtifactConflictException(path);
            }
        }

        return Task.FromResult(new StoredArtifact(path, Convert.ToHexStringLower(SHA256.HashData(bytes.Span)), bytes.Length));
    }

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
            ? new ValidationOutcome(true, "fake", 1, [])
            : ValidationOutcome.Fatal("agent-plugins.json is missing."));
    }
}
