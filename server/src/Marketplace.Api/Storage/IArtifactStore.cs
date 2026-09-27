namespace Marketplace.Api.Storage;

public sealed record StoredArtifact(string Path, string Sha256, long SizeBytes);

public sealed class ArtifactConflictException(string path) : Exception($"An artifact already exists at {path}.")
{
    public string Path { get; } = path;
}

/// <summary>Immutable blob storage. Paths are relative to the store's configured prefix.</summary>
public interface IArtifactStore
{
    Task<StoredArtifact> PutAsync(string path, ReadOnlyMemory<byte> bytes, CancellationToken cancellationToken);

    Task<byte[]> GetAsync(string path, CancellationToken cancellationToken);

    /// <summary>Removes a stored archive for good (a purge). A path that is already gone is not an error.</summary>
    Task DeleteAsync(string path, CancellationToken cancellationToken);

    /// <summary>Whether the store answers and accepts the service credential, for the health check.</summary>
    Task<bool> CheckAsync(CancellationToken cancellationToken);
}
