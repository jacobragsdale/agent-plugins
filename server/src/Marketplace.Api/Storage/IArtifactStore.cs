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
}
