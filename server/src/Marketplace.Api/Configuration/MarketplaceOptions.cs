namespace Marketplace.Api.Configuration;

/// <summary>Where clients reach this server. Used to build catalog URLs.</summary>
public sealed class ServerOptions
{
    public const string Section = "Server";

    /// <summary>Public HTTPS base URL, for example <c>https://marketplace.ragsdale.dev</c>.</summary>
    public string PublicBaseUrl { get; set; } = "https://localhost:8080";

    public string CatalogId { get; set; } = "marketplace";

    public string CatalogName { get; set; } = "Marketplace";

    public string CatalogDescription { get; set; } = "Skills and MCP servers published by people at the company.";

    /// <summary>Folder served at <c>/downloads</c>: the installer <c>manifest.json</c> and <c>releases/</c>. Unset serves nothing.</summary>
    public string? DownloadsPath { get; set; }

    /// <summary>
    /// Reverse proxies whose <c>X-Forwarded-For</c> and <c>X-Forwarded-Proto</c> are trusted: IP addresses or CIDR
    /// ranges. Empty trusts any sender, which is only safe when nothing but the proxy can reach the server.
    /// </summary>
    public string[] TrustedProxies { get; set; } = [];
}

public sealed class ArtifactKeeperOptions
{
    public const string Section = "ArtifactKeeper";

    public string BaseUrl { get; set; } = "http://artifact-keeper:8080";

    public string Repository { get; set; } = "files";

    public string Prefix { get; set; } = "marketplace";

    public string Username { get; set; } = string.Empty;

    public string Password { get; set; } = string.Empty;
}

public sealed class AuthOptions
{
    public const string Section = "Auth";

    /// <summary>
    /// Registers the Negotiate (Kerberos) scheme. Always on in a real deployment; the integration
    /// tests turn it off because the handler requires Kestrel's connection features.
    /// </summary>
    public bool EnableNegotiate { get; set; } = true;

    /// <summary>Enables LDAP group claims for Negotiate on Linux. Optional.</summary>
    public string? LdapDomain { get; set; }

    /// <summary>
    /// The account LDAP lookups bind as, with <see cref="LdapMachineAccountPassword"/>. Unset binds with the
    /// Kerberos client credentials in <c>KRB5_CLIENT_KTNAME</c> or the ticket cache.
    /// </summary>
    public string? LdapMachineAccountName { get; set; }

    public string? LdapMachineAccountPassword { get; set; }

    /// <summary>Accounts (<c>DOMAIN\user</c> or <c>user</c>) that may call <c>/api/admin/*</c>.</summary>
    public string[] AdminAccounts { get; set; } = [];

    /// <summary>A group or role claim value that grants admin.</summary>
    public string? AdminGroup { get; set; }

    /// <summary>Accounts that may publish under the <c>official</c> namespace.</summary>
    public string[] OfficialPublishers { get; set; } = [];

    /// <summary>
    /// Trust <c>X-Dev-User</c>. Always on in Development. Outside Development it lets anyone claim any
    /// account, so it is only for a server without a domain (the home lab); startup logs a warning.
    /// </summary>
    public bool AllowDevHeader { get; set; }

    /// <summary>
    /// Token issuers whose app-only tokens sign in CI pipelines. Each becomes a JwtBearer scheme; the
    /// caller's account is <see cref="MachineIssuer.Prefix"/> plus the value of <see cref="MachineIssuer.AccountClaim"/>.
    /// </summary>
    public MachineIssuer[] Machines { get; set; } = [];
}

/// <summary>An OIDC issuer CI pipelines sign in with, such as Entra ID or GitHub Actions.</summary>
public sealed class MachineIssuer
{
    /// <summary>The issuer, whose <c>/.well-known/openid-configuration</c> names its signing keys.</summary>
    public string Authority { get; set; } = string.Empty;

    /// <summary>The audience tokens must be minted for: the marketplace's app ID URI, or the URL GitHub is asked for.</summary>
    public string Audience { get; set; } = string.Empty;

    /// <summary>The claim that names the pipeline: <c>oid</c> on Entra ID, <c>repository</c> on GitHub.</summary>
    public string AccountClaim { get; set; } = string.Empty;

    /// <summary>Put before the claim value, such as <c>app:</c> or <c>github:</c>, so a machine never looks like a person.</summary>
    public string Prefix { get; set; } = string.Empty;
}

public sealed class ClientOptions
{
    public const string Section = "Client";

    /// <summary>Older desktop apps and CLIs get 426 from everything but <c>/api/me</c> and <c>/api/events</c>.</summary>
    public string MinimumVersion { get; set; } = "0.2.0";

    public string LatestVersion { get; set; } = "0.2.2";
}

/// <summary>Where new MCP reviews and problem reports are announced besides the admin portal.</summary>
public sealed class NotificationOptions
{
    public const string Section = "Notifications";

    /// <summary>Receives <c>POST { text, link }</c>, for example a Teams or Slack incoming webhook. Unset sends nothing.</summary>
    public string? WebhookUrl { get; set; }
}

/// <summary>Per-account limits on changes; reads are never limited.</summary>
public sealed class RateLimitOptions
{
    public const string Section = "RateLimits";

    public int WritesPerMinute { get; set; } = 120;

    /// <summary>Publishes and suggestions, which store an archive each.</summary>
    public int UploadsPerHour { get; set; } = 60;
}

public sealed class ValidatorOptions
{
    public const string Section = "Validator";

    /// <summary>Path to the Rust <c>validate-source</c> binary.</summary>
    public string Path { get; set; } = "validate-source";

    public int TimeoutSeconds { get; set; } = 60;
}

public sealed class DatabaseOptions
{
    public const string Section = "Database";

    public bool MigrateOnStartup { get; set; } = true;
}
