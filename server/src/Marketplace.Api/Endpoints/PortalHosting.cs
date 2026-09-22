using Marketplace.Api.Configuration;
using Microsoft.Extensions.FileProviders;

namespace Marketplace.Api.Endpoints;

/// <summary>
/// Serves the web portal (the Angular build in <c>wwwroot</c>) and the installer downloads from the same
/// origin as the API, so the browser's Windows sign-in covers both (ADR 0006).
/// </summary>
public static class PortalHosting
{
    /// <summary>Angular injects component styles at runtime, hence <c>'unsafe-inline'</c> for styles only.</summary>
    private const string ContentSecurityPolicy =
        "default-src 'self'; connect-src 'self'; font-src 'self'; img-src 'self' data:; object-src 'none'; script-src 'self'; " +
        "style-src 'self' 'unsafe-inline'; base-uri 'self'; form-action 'none'; frame-ancestors 'none'";

    public static WebApplication UsePortalFiles(this WebApplication app, ServerOptions server)
    {
        app.Use((context, next) =>
        {
            var headers = context.Response.Headers;
            headers.ContentSecurityPolicy = ContentSecurityPolicy;
            headers["Permissions-Policy"] = "camera=(), geolocation=(), microphone=(), payment=(), usb=()";
            headers["Referrer-Policy"] = "strict-origin-when-cross-origin";
            headers.XContentTypeOptions = "nosniff";
            headers.XFrameOptions = "DENY";
            return next(context);
        });

        app.UseStaticFiles();
        if (server.DownloadsPath is { Length: > 0 } downloads && Directory.Exists(downloads))
        {
            app.UseStaticFiles(new StaticFileOptions
            {
                FileProvider = new PhysicalFileProvider(Path.GetFullPath(downloads)),
                RequestPath = "/downloads",
                // Installers are .exe and .AppImage, which have no registered content type.
                ServeUnknownFileTypes = true,
                DefaultContentType = "application/octet-stream",
                OnPrepareResponse = file =>
                {
                    var path = file.Context.Request.Path.Value ?? string.Empty;
                    if (path.Equals("/downloads/manifest.json", StringComparison.Ordinal))
                    {
                        file.Context.Response.Headers.CacheControl = "no-store";
                    }
                    else if (path.StartsWith("/downloads/releases/", StringComparison.Ordinal))
                    {
                        file.Context.Response.Headers.CacheControl = "public, max-age=31536000, immutable";
                        file.Context.Response.Headers.ContentDisposition = "attachment";
                    }
                },
            });
        }

        return app;
    }

    public static WebApplication MapPortalFallback(this WebApplication app)
    {
        // An unknown API route is a 404, not the portal's index page.
        app.MapFallback("/api/{**rest}", () => Results.NotFound());
        app.MapFallbackToFile("index.html", new StaticFileOptions
        {
            // Bundles are content-hashed; the page that names them must always be revalidated.
            OnPrepareResponse = file => file.Context.Response.Headers.CacheControl = "no-cache",
        });
        return app;
    }
}
