# Marketplace server

The .NET 10 API that the Agent Plugins app and CLI talk to. It keeps the package index, per-user usage, and generated namespace archives in PostgreSQL, and stores every published version immutably in Artifact Keeper. [ADR 0004](../docs/decisions/0004-internal-marketplace.md) records the design; [the API reference](../docs/marketplace-api.md) documents the endpoints.

## Layout

| Path                                  | Purpose                                                                  |
| ------------------------------------- | ------------------------------------------------------------------------ |
| `src/Marketplace.Api`                 | The API: auth, data model, Artifact Keeper client, publish, events.      |
| `src/Marketplace.Api/Data/Migrations` | EF Core migrations; applied at startup (`Database:MigrateOnStartup`).    |
| `tests/Marketplace.Api.Tests`         | Integration tests against a throwaway PostgreSQL container.              |
| `Dockerfile`                          | Multi-stage image: Rust validator, .NET publish, ASP.NET runtime + krb5. |
| `compose.yaml`                        | Local Development stack with the `X-Dev-User` header enabled.            |
| `openapi.json`                        | Generated from `/openapi/v1.json`; regenerate after changing endpoints.  |
| `releases/`                           | Installers and `manifest.json` for the portal's download page.           |

## Run locally

```bash
cargo build --manifest-path ../src-tauri/Cargo.toml --no-default-features --bin validate-source
dotnet run --project src/Marketplace.Api      # Development: DevHeader enabled, PostgreSQL on localhost
curl -H 'X-Dev-User: CORP\jacob' http://localhost:8080/api/me
```

Or `docker compose up --build` from this directory. Publishing needs a reachable Artifact Keeper (`ArtifactKeeper__*`).

The image also carries the web portal (`website/`, built in the Dockerfile's `web` stage) under `wwwroot` and the JSON Schemas under `wwwroot/schema`, and serves installers from `Server__DownloadsPath` at `/downloads` (`releases/` in the compose file). Running with `dotnet run` serves the API only unless you copy a portal build into `src/Marketplace.Api/wwwroot`; for portal work use the Angular dev server ([website/README.md](../website/README.md)).

## Authentication

Production registers only `Negotiate` (Kerberos). The container needs:

1. An AD service account with the SPN `HTTP/<server fqdn>` (`setspn -S HTTP/marketplace.corp.example svc-marketplace`).
2. A keytab for that account (`ktpass`), mounted at `/etc/krb5.keytab` with `KRB5_KTNAME` pointing at it, and a `krb5.conf` for the realm.
3. `Auth__LdapDomain=corp.example` if team namespaces (`Auth__TeamNamespaces__0__Namespace`, `__Group`, `__DisplayName`), group-based admin, or group access lists are wanted. Groups arrive as the AD group's CN.

Negotiate on Linux is Kerberos-only: clients must use the fully qualified name in the URL, and clocks must agree within five minutes. The app's preflight checks both.

`Auth__AllowDevHeader=true` trusts `X-Dev-User`; use it only where a domain is unavailable (the home lab). `ASPNETCORE_ENVIRONMENT=Development` forces it on. `Auth__EnableNegotiate=false` exists for the test host.

## Configuration

Every setting in [the API reference](../docs/marketplace-api.md#configuration) can be supplied as `Section__Key` environment variables. The home-lab deployment is `stacks/marketplace/compose.yaml` in the home-server repository; a corporate deployment changes the public URL, the keytab, the Artifact Keeper endpoint and credential, and `AllowDevHeader`.

## Tests

```bash
dotnet run --project tests/Marketplace.Api.Tests
```

The suite starts PostgreSQL through Testcontainers, replaces Artifact Keeper with an in-memory store, and uses the real Rust validator when `src-tauri/target/debug/validate-source` exists or `MARKETPLACE_VALIDATOR` names one.

## Regenerate the OpenAPI document

```bash
ASPNETCORE_ENVIRONMENT=Development Database__MigrateOnStartup=false Kestrel__Endpoints__Http__Url=http://127.0.0.1:5088 \
  dotnet run --project src/Marketplace.Api --no-launch-profile &
curl -s http://127.0.0.1:5088/openapi/v1.json > openapi.json
pnpm --dir .. format   # the checked-in document is Prettier-formatted
```
