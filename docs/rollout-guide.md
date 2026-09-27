# Roll out Agent Plugins in a company

This guide takes an IT or platform admin from nothing to a marketplace that a few thousand Windows PCs use. It covers:

- running the marketplace server behind your reverse proxy with Kerberos sign-in;
- pointing a fleet of desktop apps at it without rebuilding them;
- keeping it backed up; and
- answering an incident.

[The API reference](marketplace-api.md) has every setting, and [the server README](../server/README.md) covers building and testing the server.

## What you need

- A Linux host with Docker, reachable from the PCs at one DNS name, for example `marketplace.corp.example`.
- A TLS certificate for that name, and a reverse proxy that terminates TLS (nginx, Caddy, Traefik, an F5, …).
- PostgreSQL 16, in the same compose project or a managed instance.
- Artifact Keeper with a generic repository (for example `files`) and a service account that can upload, download, and delete in it.
- An Active Directory service account for the server, and permission to register an SPN for it.
- A software distribution tool for the PCs (Intune, Configuration Manager, or Group Policy).

## 1. Set up Kerberos

The server accepts only Windows sign-in (`Negotiate`, which is Kerberos on Linux). PCs must use the fully qualified name in the URL, and clocks must agree within five minutes.

1. Register the SPN on the service account:

   ```powershell
   setspn -S HTTP/marketplace.corp.example svc-marketplace
   ```

2. Create a keytab for it:

   ```powershell
   ktpass -out marketplace.keytab -princ HTTP/marketplace.corp.example@CORP.EXAMPLE -mapuser CORP\svc-marketplace -crypto AES256-SHA1 -ptype KRB5_NT_PRINCIPAL -pass *
   ```

3. Copy `marketplace.keytab` and a `krb5.conf` for the realm to the host, readable only by the container.

The keytab serves two purposes:

- `KRB5_KTNAME` points the server at it to accept tickets.
- `KRB5_CLIENT_KTNAME` lets it look up AD groups over LDAP. Groups are optional: they grant admin through `Auth__AdminGroup` and can appear in share lists. When you would rather bind LDAP with a password, set `Auth__LdapMachineAccountName` and `Auth__LdapMachineAccountPassword` instead.

## 2. Run the server

`server/compose.yaml` in the repository is the development stack: it trusts an `X-Dev-User` header that lets anyone claim any account. Never deploy it. A production stack looks like this:

```yaml
name: marketplace

services:
  api:
    image: registry.corp.example/marketplace-api:0.3.0
    restart: unless-stopped
    environment:
      ASPNETCORE_ENVIRONMENT: Production
      ConnectionStrings__Marketplace: Host=db;Port=5432;Database=marketplace;Username=marketplace;Password=${DB_PASSWORD}
      Server__PublicBaseUrl: https://marketplace.corp.example
      Server__TrustedProxies__0: 10.20.0.5 # your reverse proxy
      ArtifactKeeper__BaseUrl: https://artifacts.corp.example
      ArtifactKeeper__Repository: files
      ArtifactKeeper__Prefix: marketplace
      ArtifactKeeper__Username: svc-marketplace-store
      ArtifactKeeper__Password: ${ARTIFACT_KEEPER_PASSWORD}
      Auth__LdapDomain: corp.example
      Auth__AdminGroup: Marketplace-Admins
      Auth__OfficialPublishers__0: CORP\platform-bot
      Client__MinimumVersion: 0.2.0
      Client__LatestVersion: 0.3.0
      Notifications__WebhookUrl: ${ADMIN_WEBHOOK_URL} # optional: a Teams or Slack incoming webhook
      KRB5_KTNAME: /etc/krb5.keytab
      KRB5_CLIENT_KTNAME: /etc/krb5.keytab
    volumes:
      - ./marketplace.keytab:/etc/krb5.keytab:ro
      - ./krb5.conf:/etc/krb5.conf:ro
      - ./downloads:/srv/downloads:ro
    ports:
      - "127.0.0.1:8080:8080"
    depends_on:
      db:
        condition: service_healthy

  db:
    image: postgres:16-alpine
    restart: unless-stopped
    environment:
      POSTGRES_USER: marketplace
      POSTGRES_PASSWORD: ${DB_PASSWORD}
      POSTGRES_DB: marketplace
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U marketplace -d marketplace"]
      interval: 10s
      retries: 10

volumes:
  pgdata:
```

Build the image from the repository root with `docker build -f server/Dockerfile -t registry.corp.example/marketplace-api:0.3.0 .`. It includes the web portal and the package validator. The server applies database migrations when it starts.

Then set up the rest around it:

1. **Reverse proxy.** Forward `https://marketplace.corp.example` to `http://127.0.0.1:8080`, passing the `Authorization` header unchanged, and set `X-Forwarded-For` and `X-Forwarded-Proto`. List the proxy's address in `Server__TrustedProxies` so nobody else can claim to be it; with the list empty the server trusts any sender and logs a warning. Allow uploads of at least 51 MB.
2. **Admins.** Use `Auth__AdminGroup` or `Auth__AdminAccounts__0`, `__1`, and so on. Write accounts with their domain (`CORP\jane`): an entry with a domain matches that account only, while a bare `jane` matches a jane in any domain.
3. **Check it.** `curl https://marketplace.corp.example/api/health` should show `"artifactStore": "ok"`, and `"ldap": "ok"` when you set a domain. Then open the portal in Edge on a domain PC: it should know who you are without asking.

## 3. Publish the desktop app installers

The portal's download button reads `downloads/manifest.json` and serves `downloads/releases/<version>/`:

```text
downloads/
  manifest.json
  releases/0.3.0/Agent Plugins_0.3.0_x64-setup.exe
  releases/0.3.0/Agent Plugins_0.3.0_x64_en-US.msi
```

`manifest.json` names each file with its size and SHA-256; `server/releases/manifest.json` in the repository shows the shape. After you copy a new release:

1. Set `Client__LatestVersion`, and restart. Apps below it show "A new version is available".
2. Raise `Client__MinimumVersion` only when older apps must stop. They then get `426 Update Agent Plugins` from everything except sign-in and usage reports, and the app sends people to the download page.

## 4. Roll out the desktop app

### Install it

Two installers are built for every release:

| Installer                         | Installs for                                                                                               | Silent command                                                                    |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `Agent Plugins_<v>_x64-setup.exe` | The current user, in `%LOCALAPPDATA%\Agent Plugins`. Needs no admin rights.                                | `"Agent Plugins_0.3.0_x64-setup.exe" /S`                                          |
| `Agent Plugins_<v>_x64_en-US.msi` | Every user when run elevated; the current user with `ALLUSERS=""`. Suits Intune and Configuration Manager. | `msiexec /i "Agent Plugins_0.3.0_x64_en-US.msi" /qn` (add `ALLUSERS=""` per user) |

The app needs the Microsoft Edge WebView2 Runtime. Windows 11 includes it. On Windows 10, the installers download it during setup, which fails on a PC without internet access; deploy the WebView2 Evergreen Standalone Installer first.

### Point it at your marketplace

Set these values under `HKLM\Software\Policies\AgentPlugins`, or `HKCU\…` for one person, through Group Policy Preferences or an Intune remediation script. A value under HKLM wins over HKCU, and either wins over what the app was built with.

| Value            | Type        | Meaning                                                                                                                                 |
| ---------------- | ----------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `MarketplaceUrl` | `REG_SZ`    | Your marketplace, for example `https://marketplace.corp.example`.                                                                       |
| `DownloadUrl`    | `REG_SZ`    | Where "Update Agent Plugins" sends people. Defaults to `<MarketplaceUrl>/#download`.                                                    |
| `LaunchAtLogin`  | `REG_DWORD` | `1` always starts the app with Windows, `0` never does. Unset, it starts with Windows until the person turns that off in the tray menu. |

```powershell
reg add HKLM\Software\Policies\AgentPlugins /v MarketplaceUrl /t REG_SZ /d https://marketplace.corp.example /f
reg add HKLM\Software\Policies\AgentPlugins /v LaunchAtLogin /t REG_DWORD /d 1 /f
```

Keep `LaunchAtLogin` on. The app removes a revoked package, and picks up updates, only while it runs. Its heartbeat is also how the admin page knows who has what.

### Let the portal open the app without asking

The portal's **Install in Agent Plugins** button opens an `agent-plugins://` link. Edge and Chrome ask once per site before opening an app. Allow your marketplace so they don't ask. Set the policy `AutoLaunchProtocolsFromOrigins` to:

```json
[{ "protocol": "agent-plugins", "allowed_origins": ["https://marketplace.corp.example"] }]
```

Set it in Edge (`HKLM\Software\Policies\Microsoft\Edge`) and in Chrome (`HKLM\Software\Policies\Google\Chrome`), as a `REG_SZ` holding that JSON, or through their administrative templates.

## 5. Back up and upgrade

The database holds the index, sharing, teams, usage, and the generated namespace archives the PCs download. Artifact Keeper holds every published version, which the server needs to rebuild a namespace archive on the next publish.

- Dump the database nightly: `docker exec marketplace-db-1 pg_dump -U marketplace -Fc marketplace > marketplace-$(date +%F).dump`. Adjust the container name to your compose project.
- Back up the Artifact Keeper repository after the dump, so it holds every version the dump lists. A version the database lists but Artifact Keeper lacks blocks publishing to its namespace until someone restores it or [purges that version](marketplace-api.md#delete-apiadminpackagesnamespacepackageidversionsversion).
- To restore, restore Artifact Keeper first, then `pg_restore` the database.
- Before every upgrade, dump the database; then pull the new image and restart. Migrations run on start.
- After the upgrade that adds `FleetAndFeedback`, a package whose MCP server an admin approved earlier goes back to **Waiting** the next time its owner publishes a version with an MCP server. Versions from before the upgrade don't record what their server starts, so that approval can't be compared, and the gate asks once more. Expect a short burst of reviews after the upgrade.
- Admin and official-publisher entries match an account only in the domain written: `CORP\jane` matches `CORP\jane` and `jane@corp.example.com`, never `EUROPE\jane`. A bare `jane` still matches any domain. Check the Admin page as each admin after upgrading.

## 6. Respond to an incident

When a package is malicious or leaks a secret:

1. **Remove it from every PC.** On the package's page in the portal, choose **Remove from every PC** (or `PUT /api/packages/{ns}/{id}/revoke`). Each running app uninstalls it at its next sync, within 15 minutes. It backs up copies the person edited. Only an admin can undo an admin's removal.
2. **See who had it.** On the Admin page, look the package up under installs, or call `GET /api/admin/packages/{ns}/{id}/installs?format=csv`. It lists every PC whose last heartbeat had it, with the version. PCs that stay off keep it until their app next runs.
3. **Stop the publisher if needed.** Block the account on the Admin page (`PUT /api/admin/blocks/CORP%5Cjane`). A blocked account can no longer publish, share, suggest, or join, and its personal space disappears from every catalog.
4. **Purge a leaked secret.** Delete the version for good (`DELETE /api/admin/packages/{ns}/{id}/versions/{version}`). That deletes the stored archive, and the accepted suggestion it came from, in Artifact Keeper. Then rotate the credential: purging cannot recall copies already on PCs.
5. **Find out what happened.** The audit log (`GET /api/admin/audit`, or CSV) records every publish, share, link, team change, revocation, review, block, and purge, with who did it and when.

When someone leaves the company, hand their personal space to a colleague on the Admin page (`PUT /api/admin/namespaces/{ns}/owner`). It becomes a team with the same name, so every installed skill keeps working.
