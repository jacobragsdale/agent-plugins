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
}

public sealed class ClientOptions
{
    public const string Section = "Client";

    public string MinimumVersion { get; set; } = "0.1.0";

    public string LatestVersion { get; set; } = "0.1.0";
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
