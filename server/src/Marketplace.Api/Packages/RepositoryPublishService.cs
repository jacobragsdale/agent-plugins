using System.Text;
using Marketplace.Api.Auth;
using Marketplace.Api.Configuration;
using Marketplace.Api.Data;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Options;

namespace Marketplace.Api.Packages;

/// <summary>
/// One package of a repository publish. <see cref="Status"/> is <c>new</c>, <c>changed</c>, <c>unchanged</c>, or
/// <c>failed</c>; <see cref="Version"/> is the version it takes (or has, when unchanged) and <see cref="LiveVersion"/>
/// the one live before. <see cref="PackageId"/> is null for a skill folder that could not be read.
/// </summary>
public sealed record RepositoryPackage(
    string? PackageId,
    string Path,
    string Status,
    string? Version,
    string? LiveVersion,
    bool Published,
    DryRunFile[] Files,
    ValidationError[] Errors,
    string[] Warnings);

/// <summary>
/// What publishing a repository did, or with <see cref="DryRun"/> would do. <see cref="Elsewhere"/> lists the
/// namespace's packages the repository does not hold: they are left as they are.
/// </summary>
public sealed record RepositoryPublish(string Namespace, bool DryRun, RepositoryPackage[] Packages, string[] Elsewhere)
{
    public bool Failed => Packages.Any(package => package.Status == RepositoryPublishService.FailedStatus);
}

/// <summary>
/// Publishes every package in a repository to one namespace, the way CI does on each merge: the server finds the
/// packages, and each whose files differ from its live version goes out as the next patch. Nothing publishes
/// unless every package passes, so a repository is never half released by a bad skill.
/// </summary>
public sealed class RepositoryPublishService(
    MarketplaceDbContext db,
    PublishService publish,
    IPackageValidator validator,
    IOptions<ServerOptions> server)
{
    public const string FailedStatus = "failed";

    public async Task<RepositoryPublish> PublishAsync(MarketplaceIdentity identity, string ns, byte[] archive, string? changelog, bool dryRun, CancellationToken cancellationToken)
    {
        PublishService.CheckOwner(identity, ns);
        if (changelog is { Length: > PublishService.MaxChangelog })
        {
            throw new ProblemException(422, $"The changelog is at most {PublishService.MaxChangelog:N0} characters; this one has {changelog.Length:N0}.");
        }

        var directory = Path.Combine(Path.GetTempPath(), "marketplace-repository-" + Guid.NewGuid().ToString("N"));
        try
        {
            var input = Path.Combine(directory, "repository");
            var (prefix, _) = ArchiveInspector.Survey(archive);
            ArchiveInspector.ExtractTo(archive, prefix, input);
            var discovery = await validator.DiscoverAsync(input, ns, cancellationToken);
            var results = discovery.Errors
                .Select(error => Failed(null, error.Path, [new ValidationError("", error.Message)]))
                .ToList();

            // Plan every package first: its staged archive, and the version it would take.
            var planned = new List<(int Index, PublishRequest Request)>();
            foreach (var (found, index) in discovery.Packages.Select((found, index) => (found, index)))
            {
                try
                {
                    var staged = Path.Combine(directory, $"{index}.zip");
                    await validator.StageAsync(new StagingRequest(Path.Combine(input, found.Path), staged, ns, found.PackageId, null, null), cancellationToken);
                    var bytes = await File.ReadAllBytesAsync(staged, cancellationToken);
                    if (await publish.UnchangedVersionAsync(ns, found.PackageId, bytes, cancellationToken) is { } live)
                    {
                        results.Add(new RepositoryPackage(found.PackageId, found.Path, "unchanged", live, live, false, [], [], []));
                        continue;
                    }

                    var request = new PublishRequest(ns, found.PackageId, await publish.NextVersionAsync(ns, found.PackageId, cancellationToken), [], changelog, bytes);
                    var dry = await publish.DryRunAsync(identity, request, cancellationToken);
                    var previous = await LiveVersionAsync(ns, found.PackageId, cancellationToken);
                    planned.Add((results.Count, request));
                    results.Add(new RepositoryPackage(found.PackageId, found.Path, previous is null ? "new" : "changed", dry.Version, previous, false, dry.Files, [], dry.Warnings));
                }
                catch (ProblemException problem)
                {
                    results.Add(Failed(found.PackageId, found.Path, Errors(problem)));
                }
            }

            if (!dryRun && !results.Any(result => result.Status == FailedStatus))
            {
                foreach (var (index, request) in planned)
                {
                    try
                    {
                        var (published, _) = await publish.PublishAsync(identity, request, cancellationToken);
                        results[index] = results[index] with { Version = published.Version, Published = true, Warnings = published.Warnings };
                    }
                    catch (ProblemException problem)
                    {
                        // Another publish got in between, or the store failed: the rest still go, and a rerun finishes this one.
                        results[index] = results[index] with { Status = FailedStatus, Errors = Errors(problem) };
                    }
                }
            }

            var ids = results.Select(result => result.PackageId).OfType<string>().ToHashSet(StringComparer.Ordinal);
            var elsewhere = (await db.Packages.AsNoTracking()
                    .Where(package => package.Namespace == ns && package.RevokedAt == null)
                    .Select(package => package.PackageId)
                    .ToListAsync(cancellationToken))
                .Where(id => !ids.Contains(id))
                .Order(StringComparer.Ordinal)
                .ToArray();
            return new RepositoryPublish(ns, dryRun, [.. results.OrderBy(result => result.PackageId ?? result.Path, StringComparer.Ordinal)], elsewhere);
        }
        finally
        {
            try
            {
                Directory.Delete(directory, recursive: true);
            }
            catch (IOException)
            {
            }
            catch (UnauthorizedAccessException)
            {
            }
        }
    }

    /// <summary>The report as Markdown, for a pipeline's job summary and log.</summary>
    public string Markdown(RepositoryPublish report)
    {
        var baseUrl = server.Value.PublicBaseUrl.TrimEnd('/');
        var failed = report.Packages.Count(package => package.Status == FailedStatus);
        var going = report.Packages.Count(package => package.Status is "new" or "changed");
        var unchanged = report.Packages.Count(package => package.Status == "unchanged");
        var published = report.Packages.Count(package => package.Published);
        var headline = failed > 0 && published > 0 ? $"published {published}, {failed} failed"
            : failed > 0 ? $"{failed} of {report.Packages.Length} failed, so nothing was published"
            : report.DryRun ? $"{going} to publish, {unchanged} unchanged (dry run)"
            : $"published {published}, {unchanged} unchanged";
        var text = new StringBuilder($"### {report.Namespace}: {headline}\n\n| Package | Version | Change |\n| --- | --- | --- |\n");
        foreach (var package in report.Packages)
        {
            var name = package.PackageId is { } id ? $"[{id}]({baseUrl}/p/{report.Namespace}/{id})" : $"`{package.Path}`";
            var version = package.Status switch
            {
                "new" => $"{package.Version} (new)",
                "changed" => $"{package.LiveVersion} → {package.Version}",
                "unchanged" => package.Version,
                _ => package.LiveVersion ?? "",
            };
            var change = package.Status switch
            {
                "new" => $"{package.Files.Length} file(s)",
                "changed" => string.Join(", ", package.Files.Where(file => file.Status != "same").Select(file => $"{file.Status} `{file.Path}`")),
                "unchanged" => "unchanged",
                _ => "**failed**, see below",
            };
            text.Append($"| {name} | {version} | {change} |\n");
        }

        foreach (var package in report.Packages.Where(package => package.Errors.Length > 0 || package.Warnings.Length > 0))
        {
            text.Append($"\n**{package.PackageId ?? package.Path}**{(package.Path.Length > 0 && package.PackageId is not null ? $" (`{package.Path}`)" : "")}\n\n");
            foreach (var error in package.Errors)
            {
                text.Append($"- {(error.Path.Length > 0 ? $"`{error.Path}`: " : "")}{error.Message}\n");
            }

            foreach (var warning in package.Warnings)
            {
                text.Append($"- Warning: {warning}\n");
            }
        }

        if (report.Elsewhere.Length > 0)
        {
            text.Append($"\nAlso in {report.Namespace}, not from this repository (left as they are): {string.Join(", ", report.Elsewhere)}.\n");
        }

        return text.ToString();
    }

    private async Task<string?> LiveVersionAsync(string ns, string packageId, CancellationToken cancellationToken)
    {
        var package = await db.Packages.AsNoTracking()
            .Include(candidate => candidate.Versions)
            .SingleOrDefaultAsync(candidate => candidate.Namespace == ns && candidate.PackageId == packageId, cancellationToken);
        return package is null ? null : PublishService.LatestVersion(package)?.Version;
    }

    private static RepositoryPackage Failed(string? packageId, string path, ValidationError[] errors) =>
        new(packageId, path, FailedStatus, null, null, false, [], errors, []);

    private static ValidationError[] Errors(ProblemException problem) =>
        problem.Errors.Count > 0 ? [.. problem.Errors] : [new ValidationError("", problem.Title)];
}
