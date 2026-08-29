using System.Net;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using System.Text.Json;
using Marketplace.Api.Configuration;
using Microsoft.Extensions.Options;

namespace Marketplace.Api.Storage;

/// <summary>
/// Artifact Keeper's generic repository. Uploads are <c>PUT /api/v1/repositories/{repo}/artifacts/{path}</c>
/// and are immutable (a repeat returns 409); downloads are <c>GET …/download/{path}</c>. The store logs in
/// with the service credential and refreshes the bearer token before it expires.
/// </summary>
public sealed class ArtifactKeeperStore(
    HttpClient httpClient,
    IOptions<ArtifactKeeperOptions> options,
    TimeProvider timeProvider,
    ILogger<ArtifactKeeperStore> logger) : IArtifactStore
{
    private static readonly TimeSpan TokenMargin = TimeSpan.FromSeconds(60);

    private readonly SemaphoreSlim _loginLock = new(1, 1);
    private string? _token;
    private DateTimeOffset _tokenExpiresAt = DateTimeOffset.MinValue;

    public async Task<StoredArtifact> PutAsync(string path, ReadOnlyMemory<byte> bytes, CancellationToken cancellationToken)
    {
        var full = FullPath(path);
        using var response = await SendAsync(
            () =>
            {
                var request = new HttpRequestMessage(HttpMethod.Put, $"api/v1/repositories/{options.Value.Repository}/artifacts/{full}");
                request.Content = new ReadOnlyMemoryContent(bytes);
                request.Content.Headers.ContentType = new MediaTypeHeaderValue("application/zip");
                return request;
            },
            cancellationToken);
        if (response.StatusCode == HttpStatusCode.Conflict)
        {
            throw new ArtifactConflictException(full);
        }

        if (!response.IsSuccessStatusCode)
        {
            throw new InvalidOperationException($"Artifact Keeper upload of {full} failed with HTTP {(int)response.StatusCode}.");
        }

        var expected = Convert.ToHexStringLower(SHA256.HashData(bytes.Span));
        await using var body = await response.Content.ReadAsStreamAsync(cancellationToken);
        using var document = await JsonDocument.ParseAsync(body, cancellationToken: cancellationToken);
        var reported = document.RootElement.TryGetProperty("checksum_sha256", out var checksum) ? checksum.GetString() : null;
        if (reported is not null && !string.Equals(reported, expected, StringComparison.OrdinalIgnoreCase))
        {
            throw new InvalidOperationException($"Artifact Keeper stored {full} with digest {reported}, expected {expected}.");
        }

        return new StoredArtifact(full, expected, bytes.Length);
    }

    public async Task<byte[]> GetAsync(string path, CancellationToken cancellationToken)
    {
        var full = FullPath(path);
        using var response = await SendAsync(
            () => new HttpRequestMessage(HttpMethod.Get, $"api/v1/repositories/{options.Value.Repository}/download/{full}"),
            cancellationToken);
        if (!response.IsSuccessStatusCode)
        {
            throw new InvalidOperationException($"Artifact Keeper download of {full} failed with HTTP {(int)response.StatusCode}.");
        }

        return await response.Content.ReadAsByteArrayAsync(cancellationToken);
    }

    private string FullPath(string path)
    {
        if (path.Length == 0 || path.StartsWith('/') || path.Contains("..", StringComparison.Ordinal) || path.Contains('\\'))
        {
            throw new ArgumentException($"Unsafe artifact path: {path}", nameof(path));
        }

        var prefix = options.Value.Prefix.Trim('/');
        return prefix.Length == 0 ? path : $"{prefix}/{path}";
    }

    private async Task<HttpResponseMessage> SendAsync(Func<HttpRequestMessage> build, CancellationToken cancellationToken)
    {
        for (var attempt = 0; ; attempt++)
        {
            var token = await TokenAsync(forceRefresh: attempt > 0, cancellationToken);
            var request = build();
            request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token);
            var response = await httpClient.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, cancellationToken);
            if (response.StatusCode == HttpStatusCode.Unauthorized && attempt == 0)
            {
                logger.LogWarning("Artifact Keeper rejected the bearer token; logging in again.");
                response.Dispose();
                continue;
            }

            return response;
        }
    }

    private async Task<string> TokenAsync(bool forceRefresh, CancellationToken cancellationToken)
    {
        var now = timeProvider.GetUtcNow();
        if (!forceRefresh && _token is not null && now < _tokenExpiresAt)
        {
            return _token;
        }

        await _loginLock.WaitAsync(cancellationToken);
        try
        {
            now = timeProvider.GetUtcNow();
            if (!forceRefresh && _token is not null && now < _tokenExpiresAt)
            {
                return _token;
            }

            using var response = await httpClient.PostAsJsonAsync(
                "api/v1/auth/login",
                new { username = options.Value.Username, password = options.Value.Password },
                cancellationToken);
            if (!response.IsSuccessStatusCode)
            {
                throw new InvalidOperationException($"Artifact Keeper login failed with HTTP {(int)response.StatusCode}.");
            }

            await using var body = await response.Content.ReadAsStreamAsync(cancellationToken);
            using var document = await JsonDocument.ParseAsync(body, cancellationToken: cancellationToken);
            var token = document.RootElement.GetProperty("access_token").GetString()
                ?? throw new InvalidOperationException("Artifact Keeper login returned no access token.");
            _token = token;
            _tokenExpiresAt = ExpiryOf(token, now) - TokenMargin;
            return token;
        }
        finally
        {
            _loginLock.Release();
        }
    }

    /// <summary>Reads <c>exp</c> from the JWT payload; falls back to 20 minutes when absent.</summary>
    internal static DateTimeOffset ExpiryOf(string token, DateTimeOffset now)
    {
        var parts = token.Split('.');
        if (parts.Length < 2)
        {
            return now.AddMinutes(20);
        }

        try
        {
            var payload = parts[1].Replace('-', '+').Replace('_', '/');
            payload = payload.PadRight(payload.Length + (4 - payload.Length % 4) % 4, '=');
            using var document = JsonDocument.Parse(Convert.FromBase64String(payload));
            return document.RootElement.TryGetProperty("exp", out var exp) && exp.TryGetInt64(out var seconds)
                ? DateTimeOffset.FromUnixTimeSeconds(seconds)
                : now.AddMinutes(20);
        }
        catch (FormatException)
        {
            return now.AddMinutes(20);
        }
        catch (JsonException)
        {
            return now.AddMinutes(20);
        }
    }
}
