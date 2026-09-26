using System.Diagnostics;
using System.Text.Json;
using System.Text.Json.Serialization;
using Marketplace.Api.Configuration;
using Microsoft.Extensions.Options;

namespace Marketplace.Api.Packages;

public sealed record ValidationError(string Path, string Message);

public sealed record ValidationOutcome(bool Accepted, IReadOnlyList<ValidationError> Errors)
{
    public static ValidationOutcome Fatal(string message) => new(false, [new ValidationError("", message)]);
}

/// <summary>What <c>validate-source stage</c> wraps into a one-package source zip at <see cref="OutputZip"/>.</summary>
public sealed record StagingRequest(string InputDirectory, string OutputZip, string Namespace, string PackageId, string? Name, string? Description);

public interface IPackageValidator
{
    /// <summary>Validates a source tree and scans it for files that look like credentials.</summary>
    Task<ValidationOutcome> ValidateAsync(string sourceDirectory, CancellationToken cancellationToken);

    /// <summary>Wraps a skill, a folder of skills, an MCP document, or a source tree; throws a 422 <see cref="ProblemException"/> when it cannot.</summary>
    Task StageAsync(StagingRequest request, CancellationToken cancellationToken);
}

/// <summary>
/// Runs the Rust <c>validate-source --json</c> binary so the server and the desktop app agree on
/// every manifest rule. There is one validator.
/// </summary>
public sealed class ProcessPackageValidator(IOptions<ValidatorOptions> options, ILogger<ProcessPackageValidator> logger) : IPackageValidator
{
    private static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web);

    public async Task<ValidationOutcome> ValidateAsync(string sourceDirectory, CancellationToken cancellationToken)
    {
        var (stdout, stderr) = await RunAsync(["--json", "--secrets", sourceDirectory], cancellationToken);
        var report = Parse(stdout, stderr);
        if (report is null || report.Fatal is not null)
        {
            return ValidationOutcome.Fatal(report?.Fatal ?? (stderr.Length > 0 ? stderr : "The validator produced no output."));
        }

        var errors = report.Errors.Select(error => new ValidationError(error.Path, error.Message)).ToArray();
        return new ValidationOutcome(errors.Length == 0 && report.ValidInstalls > 0, errors);
    }

    public async Task StageAsync(StagingRequest request, CancellationToken cancellationToken)
    {
        List<string> arguments = ["stage", "--namespace", request.Namespace, "--package-id", request.PackageId];
        if (request.Name is { Length: > 0 } name)
        {
            arguments.AddRange(["--name", name]);
        }

        if (request.Description is { Length: > 0 } description)
        {
            arguments.AddRange(["--description", description]);
        }

        arguments.AddRange([request.InputDirectory, request.OutputZip]);
        var (stdout, stderr) = await RunAsync(arguments, cancellationToken);
        var report = Parse(stdout, stderr);
        if (report?.Fatal is { } fatal)
        {
            throw new ProblemException(422, fatal);
        }

        if (!File.Exists(request.OutputZip))
        {
            throw new InvalidOperationException($"The validator did not stage the upload: {stderr}");
        }
    }

    private async Task<(string Stdout, string Stderr)> RunAsync(IEnumerable<string> arguments, CancellationToken cancellationToken)
    {
        var start = new ProcessStartInfo(options.Value.Path)
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        };
        foreach (var argument in arguments)
        {
            start.ArgumentList.Add(argument);
        }

        using var process = new Process { StartInfo = start };
        try
        {
            process.Start();
        }
        catch (Exception error) when (error is System.ComponentModel.Win32Exception or InvalidOperationException)
        {
            logger.LogError(error, "Could not start the validator at {Path}.", options.Value.Path);
            throw new ProblemException(503, "The package validator is unavailable, so nothing can be published right now. Tell the marketplace admins.");
        }

        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromSeconds(options.Value.TimeoutSeconds));
        var stdoutTask = process.StandardOutput.ReadToEndAsync(timeout.Token);
        var stderrTask = process.StandardError.ReadToEndAsync(timeout.Token);
        try
        {
            await process.WaitForExitAsync(timeout.Token);
        }
        catch (OperationCanceledException)
        {
            try
            {
                process.Kill(entireProcessTree: true);
            }
            catch (InvalidOperationException)
            {
            }

            cancellationToken.ThrowIfCancellationRequested();
            logger.LogWarning("The validator did not finish within {Seconds} seconds.", options.Value.TimeoutSeconds);
            throw new ProblemException(503, $"Checking the package took longer than {options.Value.TimeoutSeconds} seconds. Try again; a very large package may need to be split.");
        }

        var stdout = await stdoutTask;
        var stderr = await stderrTask;
        if (process.ExitCode == 2)
        {
            throw new InvalidOperationException($"The validator rejected its arguments: {stderr.Trim()}");
        }

        return (stdout.Trim(), stderr.Trim());
    }

    /// <summary>The single JSON line the validator prints, or null when it printed nothing.</summary>
    private ValidatorReport? Parse(string stdout, string stderr)
    {
        if (stdout.Length == 0)
        {
            return null;
        }

        try
        {
            return JsonSerializer.Deserialize<ValidatorReport>(stdout, JsonOptions)
                ?? throw new JsonException("empty document");
        }
        catch (JsonException error)
        {
            logger.LogError(error, "Validator output was not JSON: {Output} {Error}", stdout, stderr);
            throw new InvalidOperationException("The validator produced malformed output.", error);
        }
    }

    private sealed class ValidatorReport
    {
        [JsonPropertyName("validInstalls")]
        public int ValidInstalls { get; init; }

        [JsonPropertyName("errors")]
        public ValidatorError[] Errors { get; init; } = [];

        [JsonPropertyName("fatal")]
        public string? Fatal { get; init; }
    }

    private sealed class ValidatorError
    {
        [JsonPropertyName("path")]
        public string Path { get; init; } = string.Empty;

        [JsonPropertyName("message")]
        public string Message { get; init; } = string.Empty;
    }
}
