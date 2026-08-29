using System.Diagnostics;
using System.Text.Json;
using System.Text.Json.Serialization;
using Marketplace.Api.Configuration;
using Microsoft.Extensions.Options;

namespace Marketplace.Api.Packages;

public sealed record ValidationError(string Path, string Message);

public sealed record ValidationOutcome(bool Accepted, string? SourceId, int ValidInstalls, IReadOnlyList<ValidationError> Errors)
{
    public static ValidationOutcome Fatal(string message) => new(false, null, 0, [new ValidationError("", message)]);
}

public interface IPackageValidator
{
    Task<ValidationOutcome> ValidateAsync(string sourceDirectory, CancellationToken cancellationToken);
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
        var start = new ProcessStartInfo(options.Value.Path)
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        };
        start.ArgumentList.Add("--json");
        start.ArgumentList.Add(sourceDirectory);

        using var process = new Process { StartInfo = start };
        try
        {
            process.Start();
        }
        catch (Exception error) when (error is System.ComponentModel.Win32Exception or InvalidOperationException)
        {
            logger.LogError(error, "Could not start the validator at {Path}.", options.Value.Path);
            throw new InvalidOperationException($"The validator at {options.Value.Path} could not be started.", error);
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

            throw new InvalidOperationException($"The validator did not finish within {options.Value.TimeoutSeconds} seconds.");
        }

        var stdout = await stdoutTask;
        var stderr = await stderrTask;
        if (process.ExitCode == 2)
        {
            throw new InvalidOperationException($"The validator rejected its arguments: {stderr.Trim()}");
        }

        var line = stdout.Trim();
        if (line.Length == 0)
        {
            return ValidationOutcome.Fatal(stderr.Trim().Length > 0 ? stderr.Trim() : "The validator produced no output.");
        }

        ValidatorReport report;
        try
        {
            report = JsonSerializer.Deserialize<ValidatorReport>(line, JsonOptions)
                ?? throw new JsonException("empty document");
        }
        catch (JsonException error)
        {
            logger.LogError(error, "Validator output was not JSON: {Output}", line);
            throw new InvalidOperationException("The validator produced malformed output.", error);
        }

        if (report.Fatal is not null)
        {
            return ValidationOutcome.Fatal(report.Fatal);
        }

        var errors = report.Errors.Select(error => new ValidationError(error.Path, error.Message)).ToArray();
        return new ValidationOutcome(errors.Length == 0 && report.ValidInstalls > 0, report.SourceId, report.ValidInstalls, errors);
    }

    private sealed class ValidatorReport
    {
        [JsonPropertyName("sourceId")]
        public string? SourceId { get; init; }

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
