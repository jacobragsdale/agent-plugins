using Marketplace.Api.Packages;
using Microsoft.AspNetCore.Diagnostics;
using Npgsql;

namespace Marketplace.Api;

/// <summary>
/// An error the caller should see: its status, a sentence saying what went wrong, and for validation
/// failures the per-path errors. <see cref="ProblemExceptionHandler"/> turns it into an RFC 9457 document.
/// </summary>
public sealed class ProblemException(int status, string title, IReadOnlyList<ValidationError>? errors = null) : Exception(title)
{
    public int Status { get; } = status;

    public string Title { get; } = title;

    public IReadOnlyList<ValidationError> Errors { get; } = errors ?? [];

    /// <summary>The same sentence whether the target is missing or hidden, so a 404 never reveals which.</summary>
    public static ProblemException NotFound(string what) => new(404, $"{what} was not found, or you do not have access to it.");

    public static ProblemException NotOwner(string account, string ns) => new(403, $"{account} does not own the namespace {ns}.");

    public static ProblemException DatabaseUnavailable() =>
        new(503, "The marketplace database is unavailable. Try again in a few minutes; if it keeps failing, tell the marketplace admins.");
}

/// <summary>
/// Writes <see cref="ProblemException"/>, malformed-request errors, and database outages as problem
/// documents. Anything else falls through to the default 500, which logs it.
/// </summary>
public sealed class ProblemExceptionHandler(IProblemDetailsService problems, ILogger<ProblemExceptionHandler> logger) : IExceptionHandler
{
    public async ValueTask<bool> TryHandleAsync(HttpContext context, Exception exception, CancellationToken cancellationToken)
    {
        var (status, title, detail, errors) = exception switch
        {
            ProblemException problem => (problem.Status, problem.Title, null, problem.Errors),
            BadHttpRequestException { StatusCode: StatusCodes.Status413PayloadTooLarge } =>
                (413, $"The request is larger than the {ArchiveInspector.MaxArchiveBytes / 1024 / 1024} MB limit.", null, []),
            BadHttpRequestException bad => (bad.StatusCode, bad.Message, bad.InnerException?.Message, (IReadOnlyList<ValidationError>)[]),
            _ when IsDatabaseOutage(exception) => (503, ProblemException.DatabaseUnavailable().Title, null, []),
            _ => (0, string.Empty, null, []),
        };
        if (status == 0)
        {
            return false;
        }

        context.Response.StatusCode = status;
        var problemDetails = new Microsoft.AspNetCore.Mvc.ProblemDetails { Status = status, Title = title, Detail = detail };
        if (errors.Count > 0)
        {
            problemDetails.Extensions["errors"] = errors.Select(error => new { path = error.Path, message = error.Message }).ToArray();
        }

        if (status == 503 && exception is not ProblemException)
        {
            logger.LogError(exception, "Answered 503: {Title}", title);
        }

        return await problems.TryWriteAsync(new ProblemDetailsContext { HttpContext = context, ProblemDetails = problemDetails, Exception = exception });
    }

    /// <summary>A transient Npgsql failure, bare or wrapped by EF Core, means PostgreSQL is unreachable.</summary>
    private static bool IsDatabaseOutage(Exception? exception)
    {
        for (; exception is not null; exception = exception.InnerException)
        {
            if (exception is NpgsqlException { IsTransient: true })
            {
                return true;
            }
        }

        return false;
    }
}
