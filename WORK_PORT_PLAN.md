# Port Agent Plugins to work: agent runbook

You are an agent moving this repository from Jacob's home lab onto company infrastructure: Azure DevOps (repo and pipelines), AKS (runtime), the `deployments` repo (Kubernetes config), the company artifact repository, and the `rangular` Angular framework. Jacob builds the Windows installers on his own workstation.

This file was written against commit `b1c9e14` (version 0.2.2) on 2026-09-27. File and line references can drift. If one doesn't match, search for the symbol it names.

## Rules for the agent

1. **Work one phase at a time.** Every phase ends with a **Checkpoint**. Don't start the next phase until the checkpoint passes. Paste the checkpoint output into your progress notes.
2. **Never guess company facts.** Hostnames, registries, the IdP, secret tooling, and namespaces all go in the [Facts table](#facts-table). When a fact is missing and you can't discover it read-only, stop and ask Jacob. Each **ASK** in this document is a hard stop.
3. **Copy house patterns.** Before you write a pipeline, manifest, or secret, find the most similar existing app in the `deployments` repo and in Azure DevOps and copy how it does things. This runbook describes _what_ has to exist. The house patterns decide _how_ it's written (Helm vs Kustomize, pipeline templates, secret tooling).
4. **Keep the fork diff small.** Prefer new files and small edits over rewrites, so later merges from upstream stay easy. Section 12 lists every file the port is expected to touch.
5. **Never do these:**
   - Deploy `server/compose.yaml`. It trusts `X-Dev-User`, so anyone could claim any account.
   - Set `ASPNETCORE_ENVIRONMENT=Development` or `Auth__AllowDevHeader=true` anywhere reachable. Development mode turns the dev header on no matter what `AllowDevHeader` says (`Program.cs`: `var devHeader = builder.Environment.IsDevelopment() || authOptions.AllowDevHeader`).
   - Commit a plaintext secret, token, keytab, or `.npmrc` that contains a token.
   - Bake credentials into an image layer.
6. **Keep the repo's gates green.** They're listed in `AGENTS.md`. The Rust crate denies all warnings, so clippy must be clean. CI also checks that `server/openapi.json` is current, `cargo fmt`, Prettier, ESLint with zero warnings, and the Angular strict build.

---

## 1. What you are porting

There are three deliverables, and they come from one repository:

| Deliverable                                                                                                                                                                                                                              | Source                                               | Built where                                 | Runs where                          |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- | ------------------------------------------- | ----------------------------------- |
| **Desktop app**: Tauri 2 (Rust in `src-tauri/`, React in `src/`). Ships as an NSIS `.exe` (per user) and an `.msi`. The installers also carry a console twin, `agent-plugins.com`, for the CLI.                                          | `src-tauri/`, `src/`                                 | Jacob's Windows workstation                 | Employee Windows PCs                |
| **Marketplace server**: .NET 10 API, one container image. The image also holds the **Angular portal** (`website/`, served at `/`) and the Rust **`validate-source`** binary, which the API runs on every upload.                         | `server/`, `website/`, `src-tauri/` (validator only) | Azure DevOps pipeline (`server/Dockerfile`) | AKS                                 |
| **Backing services**: PostgreSQL 16 holds the index, sharing, teams, usage, and the generated archives. A **blob store** holds every published version, immutably. Kerberos accepts Windows sign-in. `/downloads` serves the installers. | none                                                 | none                                        | Managed PostgreSQL, Artifactory, AD |

Request flow:

```text
Windows PC (desktop app / CLI) ──HTTPS + "Authorization: Negotiate <kerberos>"──┐
Browser (portal, same origin) ──HTTPS + Negotiate (or Bearer, if Option B)──────┤
                                                                                  ▼
                                   AKS ingress (TLS) ──► marketplace-api pod :8080
                                                         ├─► PostgreSQL :5432
                                                         ├─► Artifactory (HTTPS, service token)
                                                         └─► AD LDAP :389 (optional, for groups)
```

Read these first. Together they take about 15 minutes:

- `docs/rollout-guide.md` is the existing production guide, written for Docker. Most of it still applies. This runbook translates it to AKS.
- `server/README.md` covers authentication, configuration, and tests.
- `docs/marketplace-api.md#configuration` lists every server setting. Every setting can be supplied as a `Section__Key` environment variable.
- `docs/development.md` covers the verification loop.

What changes from the home lab:

| Home lab                                                      | Work                                                                                  |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| GitHub + GitHub Actions (`.github/workflows/ci.yml`)          | Azure Repos + Azure Pipelines                                                         |
| Docker Compose on one host, deployed by `release.sh`          | AKS, configured from the `deployments` repo                                           |
| Artifact Keeper stores packages (`ArtifactKeeperStore.cs`)    | The company artifact repository (assumed to be JFrog Artifactory; confirm in Phase 0) |
| `X-Dev-User` header (the home lab has no domain)              | Kerberos (`Negotiate`) with a keytab, and possibly rangular/OIDC for the portal       |
| Built-in URL `https://marketplace.ragsdale.dev`               | The company FQDN                                                                      |
| Public registries (Docker Hub, npm, crates.io, NuGet, GitHub) | Artifactory remotes, probably behind a proxy with a TLS-inspecting corporate CA       |

---

## Facts table

Fill this in during Phase 0 and keep it up to date. Every `<PLACEHOLDER>` in this document refers to a row here.

| Key                                                                                                                                                | Value | Source / how found |
| -------------------------------------------------------------------------------------------------------------------------------------------------- | ----- | ------------------ |
| `<ADO_ORG>` / `<ADO_PROJECT>`                                                                                                                      |       |                    |
| `<ADO_REPO>` (suggest `agent-plugins`)                                                                                                             |       |                    |
| `<AGENT_POOL>`: Microsoft-hosted or self-hosted? Does it have Docker? Can it reach the internet?                                                   |       |                    |
| ADO service connections: container registry, AKS/kube, Artifactory                                                                                 |       |                    |
| ADO variable groups / Key Vault links used by similar apps                                                                                         |       |                    |
| `<MARKETPLACE_FQDN>`, e.g. `agent-plugins.corp.example`                                                                                            |       |                    |
| Is the DNS record an A record or a CNAME? (Kerberos cares; see §13)                                                                                |       |                    |
| TLS certificate mechanism (cert-manager issuer, Key Vault CSI, wildcard secret)                                                                    |       |                    |
| `<K8S_NAMESPACE>` / cluster name                                                                                                                   |       |                    |
| Ingress controller (ingress-nginx, AGIC/App Gateway, Traefik) and its class name                                                                   |       |                    |
| Is a WAF in front? What are its body-size and upload limits?                                                                                       |       |                    |
| Pod CIDR or ingress controller IP range (for `Server__TrustedProxies`)                                                                             |       |                    |
| `deployments` repo URL, layout (Helm, Kustomize, raw), and how it's applied (Argo CD, Flux, pipeline `kubectl`/`helm`)                             |       |                    |
| Secret mechanism in `deployments` (SealedSecrets, SOPS, ExternalSecrets + Key Vault, CSI)                                                          |       |                    |
| Where app names are registered (an Argo ApplicationSet list, a values file, a catalog)                                                             |       |                    |
| Closest existing app to copy (a .NET API with ingress and a database)                                                                              |       |                    |
| `<REGISTRY>`: where images are pushed (ACR or an Artifactory Docker repo)                                                                          |       |                    |
| Mandated base images (the company's hardened `aspnet`/`sdk` images?)                                                                               |       |                    |
| `<ARTIFACTORY_URL>`: product and version (`/artifactory/api/system/version`)                                                                       |       |                    |
| Artifactory remotes for npm, NuGet, crates (cargo), Docker Hub, MCR, Debian apt, GitHub releases, rustup                                           |       |                    |
| `<PKG_REPO>`: generic repo for marketplace packages; service account; token                                                                        |       |                    |
| `<INSTALLERS_REPO>`: generic repo for desktop installers                                                                                           |       |                    |
| Corporate root CA certificate (PEM) for TLS inspection or internal CAs                                                                             |       |                    |
| HTTP(S) proxy for build agents and pods, if any                                                                                                    |       |                    |
| PostgreSQL: Azure Database for PostgreSQL Flexible Server or in-cluster? Host, DB, user, SSL mode                                                  |       |                    |
| AD realm (e.g. `CORP.EXAMPLE`), NetBIOS domain (`CORP`), DNS domain                                                                                |       |                    |
| Service account for the SPN, and who can run `setspn` and `ktpass`                                                                                 |       |                    |
| Can AKS pods reach domain controllers on 88/389? (only needed for AD groups)                                                                       |       |                    |
| Admin accounts or admin AD group                                                                                                                   |       |                    |
| Official publishers (accounts that may publish to `official`)                                                                                      |       |                    |
| rangular: npm package name, version, the Angular versions it supports, the IdP it uses, token type, and which claim identifies the user            |       |                    |
| Code-signing: Authenticode certificate (PFX, cert store, HSM, Azure Key Vault, Trusted Signing)?                                                   |       |                    |
| Tauri updater key (minisign): does it exist, and is it wanted? (see §10.4)                                                                         |       |                    |
| Can PCs reach `github.com` release downloads? (the app downloads `uv` from there)                                                                  |       |                    |
| Teams/Slack webhook for admin notifications (optional)                                                                                             |       |                    |
| Entra tenant ID, and who can create app registrations                                                                                              |       |                    |
| Can ADO project admins create Azure Resource Manager service connections with workload identity federation? Must they be scoped to a subscription? |       |                    |

---

## Phase 0: Discovery (read-only, no changes)

Goal: fill in the Facts table and make decisions D1–D6. Don't write code yet.

### 0.1 The `deployments` repo

1. Clone it and map its layout: `git ls-files | head -200`, and `tree -L 3` if available.
2. Find how an app is declared: grep for an app you know, and for `kind: Deployment`, `kind: Ingress`, `kustomization.yaml`, `Chart.yaml`, `values*.yaml`, and `Application` (Argo).
3. Pick the **closest existing app**: an HTTP API with an ingress, a database connection string in a Secret, and ideally .NET. Read every file for that app and write down:
   - The naming convention (app name, namespace, labels, annotations).
   - How the image tag is set, and who bumps it (a pipeline commit, a PR, Argo Image Updater).
   - How secrets are stored. Is anything plaintext? If so, ask before you follow that pattern.
   - The pod `securityContext`, resource requests and limits, probes, NetworkPolicy, PodDisruptionBudget, HPA.
   - The ingress annotations (body size, timeouts, TLS, WAF policy).
   - Where the app's name is registered so the GitOps tool picks it up.
4. Find how changes reach the cluster: Argo CD or Flux (look for `Application` or `Kustomization` CRs), or a pipeline running `kubectl apply` or `helm upgrade`. Find how environments (dev, test, prod) are split.

### 0.2 Azure DevOps

1. Find an existing pipeline for a similar containerized service. Read its YAML and every template it `extends` or includes (`resources.repositories`, `template:`).
2. Note the agent pool, whether it runs Docker (the server tests use Testcontainers and need a Docker daemon), and whether Windows agents exist.
3. Note the service connection names, the variable groups, and any required steps (image scanning, SBOM, SonarQube, approvals).

### 0.3 The artifact repository

1. Confirm the product: `curl -fsS <ARTIFACTORY_URL>/artifactory/api/system/ping` returns `OK` on JFrog Artifactory. If it's something else (Azure Artifacts, Nexus), **ask**. Phase 3 assumes Artifactory's REST API.
2. For each client (npm, NuGet, cargo, Docker, Debian, generic), open **Set Me Up** in the Artifactory UI and copy the exact URL and config snippet. Use those snippets verbatim; Artifactory URL shapes vary by version.
3. Ask Jacob for (or request) two things:
   - A **generic** local repo for marketplace packages (`<PKG_REPO>`), with a service account whose access token has **read, deploy, and delete** on it. Delete is needed for admin purges and package deletes.
   - A generic repo for desktop installers (`<INSTALLERS_REPO>`). It can be the same repo under a different path prefix.

### 0.4 rangular

1. Find the package in the Artifactory npm remote or virtual repo (`npm view <package> --registry <npm-url>`). Read its README, its `peerDependencies`, and one internal app that uses it.
2. Answer these questions:
   - **Angular versions supported.** The portal is on **Angular 22** (`website/package.json`). If rangular requires an older major, that's a blocker for Option B; report it.
   - **IdP and protocol.** Is it Entra ID (MSAL), Ping, Okta, or something else? OIDC code flow with PKCE?
   - **How the token reaches the API.** Is it an `HttpInterceptor` with a bearer token, or a cookie from a BFF or gateway?
   - **Which claim names the user**, and in what format (`jane@company.com`, `CORP\jane`, `jane`)?
   - **What the app needs registered**: client ID, authority, redirect URI, scopes/audience.
3. Collect sample token claims for one real user, if an existing app can show them.

### 0.5 Kerberos feasibility

Ask Jacob:

- Can an AD service account be created, with `setspn -S HTTP/<MARKETPLACE_FQDN> <svc>` and a `ktpass` keytab (AES256)? Who does it?
- Is `<MARKETPLACE_FQDN>` in the browsers' Local Intranet zone, or in Edge/Chrome `AuthServerAllowlist`? Most corporate GPOs already cover `*.corp.example`.
- Should admin come from an AD group? That needs LDAP from the pod to domain controllers.

### 0.6 Decisions (ASK Jacob; record the answers in the Facts table)

**D1. Authentication.** This is the most important decision. The desktop app signs in with Kerberos through Windows SSPI (`src-tauri/src/marketplace.rs`, `auth_headers`). Rewriting that is expensive. The portal can do either.

| Option                                        | What it is                                                                                                                   | Code change                                                                                        | When to choose                                                                                |
| --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| **A (recommended if a keytab is obtainable)** | Kerberos everywhere. Edge and Chrome send Kerberos to the portal automatically on domain PCs. rangular is not used for auth. | **None**                                                                                           | An SPN and keytab can be issued, and policy doesn't force rangular auth on every Angular app. |
| **B**                                         | The desktop app and CLI keep Kerberos. The portal uses rangular (OIDC bearer tokens). The server accepts both.               | Server: add JwtBearer and claim canonicalization (§4.2). Portal: wire rangular (§4.3). CSP update. | rangular auth is mandatory for web apps, and a keytab is still available for the desktop app. |
| **C**                                         | No Kerberos at all. The desktop app needs OAuth too.                                                                         | Large Rust change (new `AuthMode`, browser/PKCE or WAM login, token cache). Multi-week.            | Only if no keytab is possible. Stop and scope it separately with Jacob.                       |

**D2. Package store.** Artifactory generic repo `<PKG_REPO>` (Phase 3). Confirm.

**D3. Image registry.** ACR or Artifactory Docker. Follow the closest existing app.

**D4. PostgreSQL.** Recommended: Azure Database for PostgreSQL Flexible Server 16, with managed backups (this replaces the nightly `pg_dump` in the rollout guide). The schema uses **no Postgres extensions** (verified), so the flexible server needs no `azure.extensions` allowlisting.

**D5. Installer hosting.** Recommended: installers live in `<INSTALLERS_REPO>`, and an init container copies the current version into the pod at start (§7.4). The alternative is an Azure Files PVC mounted at `/srv/downloads`.

**D6. Signing.** See §10.4. Is the key Jacob has a Tauri updater key (minisign) or an Authenticode code-signing certificate?

**Checkpoint 0:** every row of the Facts table is filled or marked "ask Jacob", and D1–D6 are answered.

---

## Phase 1: Get the repo into Azure DevOps

1. **Create the repo.** Ask Jacob, or run `az repos create --org https://dev.azure.com/<ADO_ORG> --project <ADO_PROJECT> --name <ADO_REPO>`.
2. **Import the history.** **ASK** whether to keep the full history (the simplest option) or start from one squashed commit. The history mentions the home lab (`ragsdale.dev`, VM notes in docs).
   - Full history: `git remote add ado <url> && git push ado main`
   - Squashed: `git checkout --orphan work-main && git commit -m "Import Agent Plugins 0.2.2" && git push ado work-main:main`
3. **Keep a path back to upstream (optional).** `git remote add upstream https://github.com/jacobragsdale/agent-plugins.git`, if work machines can reach GitHub. Merge upstream into a branch and open a PR. Section 12 lists the files that will conflict.
4. **Remove GitHub-only files.** Delete `.github/`; Phase 6 replaces it. Keep `.githooks/`: `pnpm install` configures a pre-commit hook that runs Prettier and `cargo fmt` checks.
5. **Point package managers at Artifactory**, using the Set Me Up snippets. Commit config **without credentials**; credentials come from environment variables or pipeline secrets.
   - **npm/pnpm:** a root `.npmrc` with `registry=<npm-virtual-url>`, plus `@<rangular-scope>:registry=<url>` if rangular lives in a separate repo. Auth comes from `//<host>/…/:_authToken=${NPM_TOKEN}`; pnpm expands environment variables in `.npmrc`. The lockfile doesn't pin registry hosts, so `pnpm install --frozen-lockfile` works against the mirror unchanged. The `patches/` directory still applies.
   - **NuGet:** `server/nuget.config` with `<clear/>` and one `<add key="artifactory" value="<nuget-v3-url>"/>`. Credentials go in `<packageSourceCredentials>` using `%NUGET_USER%`/`%NUGET_TOKEN%`, or through the pipeline's `NuGetAuthenticate`/service connection. This also covers `dotnet tool restore` (`server/dotnet-tools.json`, `dotnet ef`).
   - **Cargo:** `src-tauri/.cargo/config.toml`:

     ```toml
     [source.crates-io]
     replace-with = "artifactory"

     [source.artifactory]
     registry = "sparse+<cargo-remote-index-url>"   # from Set Me Up; keep the trailing slash

     [registries.artifactory]
     index = "sparse+<cargo-remote-index-url>"
     ```

     If the remote needs auth, set `CARGO_REGISTRIES_ARTIFACTORY_TOKEN="Bearer <token>"` in the environment and add `[registry] global-credential-providers = ["cargo:token"]`. `Cargo.lock` checksums don't change.

   - **Rust toolchain:** if `static.rust-lang.org` is blocked, set `RUSTUP_DIST_SERVER` and `RUSTUP_UPDATE_ROOT` to an Artifactory generic remote that proxies it.
6. **Branch policy on `main`.** Ask Jacob to set it: require the CI pipeline from Phase 6 as build validation, and require a PR.

**Checkpoint 1:**

- The repo exists in ADO with `main` pushed.
- `pnpm config get registry` prints the Artifactory URL. pnpm 11 reads only registry and auth settings from `.npmrc`; everything else belongs in `pnpm-workspace.yaml`.
- On a machine that has only Artifactory access, these succeed: `pnpm install --frozen-lockfile`, `cargo fetch --manifest-path src-tauri/Cargo.toml`, and `dotnet restore server/Marketplace.slnx`.

---

## Phase 2: Company-specific code and config changes

Make each change as its own small commit.

1. **Built-in marketplace URL.** `src-tauri/src/locator.rs:11`:

   ```rust
   pub(crate) const MARKETPLACE_URL: &str = "https://<MARKETPLACE_FQDN>";
   ```

   The `MarketplaceUrl` registry policy (`HKLM/HKCU\Software\Policies\AgentPlugins`) still overrides it, and debug builds honor `AGENT_PLUGINS_MARKETPLACE_URL`. The `ragsdale.dev` strings in `app_state.rs` and `src/ipc/fixtures/app-state.json` are test fixtures. Leave them alone, or the fixture test fails.

2. **App identifier.** **ASK** before changing this. In `src-tauri/tauri.conf.json`, `"identifier": "dev.jacobragsdale.agentplugins"` should become `com.<company>.agentplugins`. Change it now or never: the identifier names the app's data and WebView folders, so changing it after the fleet has installed the app orphans their state. Test PCs that have Jacob's personal build installed must uninstall it first. Both builds install into `%LOCALAPPDATA%\Agent Plugins`.
3. **Metadata (optional).** Update `authors` in `src-tauri/Cargo.toml` and decide about `LICENSE` (MIT, © Jacob Ragsdale); that's Jacob's call. Replacing `ragsdale.dev` examples in `docs/` is cosmetic.
4. **Portal CSP.** Only for Option B; see §4.3. The CSP is in `server/src/Marketplace.Api/Endpoints/PortalHosting.cs`.
5. **`uv` download.** `src-tauri/src/startup.rs:~2022` downloads `uv` from `https://github.com/astral-sh/uv/releases/latest/download/...` on PCs that don't have it. MCP servers launched through `uvx` depend on it.
   - Ask Jacob to run `Invoke-WebRequest https://github.com/astral-sh/uv/releases/latest/download/uv-x86_64-pc-windows-msvc.zip -OutFile $env:TEMP\uv.zip` on a work PC.
   - If it's blocked, point the URLs at an Artifactory generic remote that proxies GitHub releases, keeping `latest/download/<file>`. Alternatively, IT can deploy uv, and the app then skips the download.
   - Also tell Jacob that MCP servers pull from PyPI and npm at run time (`uvx`, `npx`). PCs may need `UV_DEFAULT_INDEX` and npm `registry` pointed at Artifactory by policy. That's fleet configuration, not code.

**Checkpoint 2:** every gate in `docs/development.md` → "Verify a change" passes, including `cargo clippy --all-targets -- -D warnings` and `cargo test`.

---

## Phase 3: Store packages in Artifactory

The server talks to storage only through `IArtifactStore` (`server/src/Marketplace.Api/Storage/IArtifactStore.cs`), which has four methods. Add an `ArtifactoryStore` next to `ArtifactKeeperStore` and select it by configuration. Don't delete Artifact Keeper support; keeping it makes upstream merges trivial.

### 3.1 Contract the new store must keep

| Method                  | Required behavior                                                                                                                                                                                                                                            | Artifactory call                                                                                                                                                   |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `PutAsync(path, bytes)` | **Immutable**: if the path already exists, throw `ArtifactConflictException(full)`. `PublishService.StoreAsync` relies on this to adopt identical leftovers from failed publishes. Verify the SHA-256. Return `new StoredArtifact(full, sha256hex, length)`. | `HEAD /artifactory/<repo>/<full>`: 200 means conflict. Otherwise `PUT /artifactory/<repo>/<full>` with the bytes. The 201 JSON has `checksums.sha256`; compare it. |
| `GetAsync(path)`        | Return the bytes. On 404, throw **`InvalidOperationException`**; the callers catch that type. Cache downloads like `ArtifactKeeperStore._downloads` (256 MB `MemoryCache`).                                                                                  | `GET /artifactory/<repo>/<full>`                                                                                                                                   |
| `DeleteAsync(path)`     | Hard delete. A 404 is fine. Any other failure becomes `ProblemException(502, …)`. Evict from the cache.                                                                                                                                                      | `DELETE /artifactory/<repo>/<full>`                                                                                                                                |
| `CheckAsync()`          | `true` when the repo answers with the service token. This drives `/api/health` → `"artifactStore": "ok"`.                                                                                                                                                    | `GET /artifactory/api/repositories/<repo>`                                                                                                                         |

**Why HEAD before PUT:** Artifactory overwrites on PUT whenever the account has Delete/Overwrite permission, and deletes need that same permission. The HEAD check is what keeps the store immutable. There's a race between HEAD and PUT: two identical concurrent publishes could both pass the check. The database's unique version row stops the second publish, so accept the race and mark it with a `ponytail:` comment.

**Reuse from `ArtifactKeeperStore.cs`:**

- `FullPath`, which rejects `..`, backslashes, and leading `/`, and applies `Prefix`.
- The `SendAsync` retry: one retry on a network error or a 502/503/504, then `Unavailable(...)` becomes a 503 `ProblemException`.
- The download cache.

Drop the login, token refresh, and `ExpiryOf` logic: Artifactory takes a static access token, `Authorization: Bearer <token>`.

**Paths.** URL-encode each path segment. Package paths are ASCII-safe today, but encode anyway.

**Trash can.** A leaked-secret purge (`DELETE /api/admin/packages/{ns}/{id}/versions/{v}`) must actually remove the bytes. If the repo's trash can is on, the deleted archive is kept for N days. After a successful DELETE, also call `DELETE /artifactory/api/trash/clean/<repo>/<full>` and ignore a 404. If the service account isn't allowed to, log it, and document the manual trash purge in the incident steps of `docs/rollout-guide.md` §6.

### 3.2 Wiring

1. Add to `server/src/Marketplace.Api/Configuration/MarketplaceOptions.cs`:

   ```csharp
   public sealed class ArtifactoryOptions
   {
       public const string Section = "Artifactory";
       public string BaseUrl { get; set; } = string.Empty;   // https://<ARTIFACTORY_URL> (no /artifactory suffix; the store adds it)
       public string Repository { get; set; } = "agent-plugins";
       public string Prefix { get; set; } = "marketplace";
       public string Token { get; set; } = string.Empty;
   }
   ```

2. In `Program.cs`, replace the `AddHttpClient<ArtifactKeeperStore>` block and the `IArtifactStore` registration with a switch on `Artifactory:BaseUrl`:

   ```csharp
   builder.Services.Configure<ArtifactoryOptions>(builder.Configuration.GetSection(ArtifactoryOptions.Section));
   var artifactory = builder.Configuration.GetSection(ArtifactoryOptions.Section).Get<ArtifactoryOptions>() ?? new ArtifactoryOptions();
   if (artifactory.BaseUrl.Length > 0)
   {
       builder.Services.AddHttpClient<ArtifactoryStore>(client =>
       {
           client.BaseAddress = new Uri(artifactory.BaseUrl.TrimEnd('/') + "/");
           client.Timeout = TimeSpan.FromSeconds(120);
       }).ConfigurePrimaryHttpMessageHandler(() => new SocketsHttpHandler { PooledConnectionLifetime = TimeSpan.FromMinutes(2) });
       builder.Services.AddSingleton<IArtifactStore>(services => services.GetRequiredService<ArtifactoryStore>());
   }
   else
   {
       // existing ArtifactKeeperStore registration, unchanged
   }
   ```

3. Add `"Artifactory": { "BaseUrl": "", "Repository": "agent-plugins", "Prefix": "marketplace", "Token": "" }` to `appsettings.json`.
4. **Tests.** Copy `server/tests/Marketplace.Api.Tests/ArtifactKeeperStoreTests.cs` and its scripted `HttpMessageHandler` into `ArtifactoryStoreTests.cs`. Cover these cases:
   - A PUT to a new path sends HEAD (404) and then PUT (201) with a matching sha256.
   - A PUT to an existing path (HEAD 200) throws `ArtifactConflictException`.
   - A PUT whose response reports a different sha256 throws.
   - A GET that returns 404 throws `InvalidOperationException`.
   - One 503 is retried; two become a 503 `ProblemException`.
   - A DELETE that returns 404 succeeds.
   - `CheckAsync` returns false on 401.

   The integration suite (`MarketplaceApiFactory`) already swaps in an in-memory store, so it's unaffected.

5. **Docs.** Add the `Artifactory__*` settings to the configuration table in `docs/marketplace-api.md`.

**Checkpoint 3:**

- `dotnet build server/Marketplace.slnx -warnaserror` passes.
- `dotnet run --project server/tests/Marketplace.Api.Tests` passes. `dotnet test` doesn't work: the repo uses Microsoft.Testing.Platform.
- `server/openapi.json` is unchanged, because this adds no endpoints.
- Optional live check with a local API pointed at the real `<PKG_REPO>` (dev header, localhost only): publish `examples/hello` from the portal, and see the zip under `<PKG_REPO>/marketplace/`.

---

## Phase 4: Authentication

### 4.1 Option A: Kerberos only (no code)

All the work is configuration, done in Phases 7 and 8:

1. The SPN `HTTP/<MARKETPLACE_FQDN>` is set on the service account, and an AES256 keytab exists. Commands are in `docs/rollout-guide.md` §1.
2. The keytab is mounted in the pod, with `KRB5_KTNAME` and `KRB5_CLIENT_KTNAME` pointing at it. `krb5.conf` is mounted at `/etc/krb5.conf`.
3. Browsers send Kerberos to the FQDN, through the Local Intranet zone or Edge/Chrome `AuthServerAllowlist`.
4. The FQDN is an **A record**, or SPNs cover every name that CNAME canonicalization produces (see §13).

### 4.2 Option B: add bearer tokens for the portal (server)

Everything downstream keys off `principal.Identity.Name`. `IdentityResolver.Resolve` in `server/src/Marketplace.Api/Auth/MarketplaceIdentity.cs` turns it into the account, and then the namespace, admin status, blocks, and team membership. Namespace ownership compares account strings **exactly** (`IsClaimant`). **So the same person must get byte-for-byte the same account string from Kerberos and from a JWT.** Otherwise someone who publishes from the portal and installs from the app ends up with two identities, e.g. namespaces `jane` and `jane-2`.

Steps:

1. **Find the Kerberos form first.** Deploy Option A config (or a test deployment), then from a domain PC run:

   ```powershell
   Invoke-RestMethod -UseDefaultCredentials https://<MARKETPLACE_FQDN>/api/me | ConvertTo-Json
   ```

   Record `account`. It's most likely `jane@CORP.EXAMPLE.COM`, the Kerberos principal with the realm uppercase. That string is the canonical form.

2. **Add the package.** `Microsoft.AspNetCore.Authentication.JwtBearer` 10.x, matching the framework, in `Marketplace.Api.csproj`.
3. **Add options** to `AuthOptions`:
   - `JwtAuthority` (issuer or metadata URL)
   - `JwtAudience`
   - `JwtAccountClaim`: the claim holding the sAMAccountName or UPN prefix, e.g. `onprem_sam_account_name`, `samaccountname`, or `preferred_username`
   - `JwtRealm`, e.g. `CORP.EXAMPLE.COM`
   - `JwtGroupsClaim`, e.g. `groups` or `roles`
4. **Register the scheme** when `JwtAuthority` is set, and canonicalize the identity in `OnTokenValidated`:

   ```csharp
   authentication.AddJwtBearer(JwtBearerDefaults.AuthenticationScheme, options =>
   {
       options.Authority = authOptions.JwtAuthority;
       options.Audience = authOptions.JwtAudience;
       options.MapInboundClaims = false;
       options.Events = new JwtBearerEvents
       {
           OnTokenValidated = context =>
           {
               var principal = context.Principal!;
               var raw = principal.FindFirst(authOptions.JwtAccountClaim)?.Value;
               if (string.IsNullOrWhiteSpace(raw)) { context.Fail("The token has no account claim."); return Task.CompletedTask; }
               var user = IdentityResolver.Username(raw.Trim());
               var identity = new ClaimsIdentity(JwtBearerDefaults.AuthenticationScheme, ClaimTypes.Name, ClaimTypes.Role);
               identity.AddClaim(new Claim(ClaimTypes.Name, $"{user}@{authOptions.JwtRealm}"));
               // Given/surname drive the display name in Resolve.
               if (principal.FindFirst("given_name") is { } given) identity.AddClaim(new Claim(ClaimTypes.GivenName, given.Value));
               if (principal.FindFirst("family_name") is { } family) identity.AddClaim(new Claim(ClaimTypes.Surname, family.Value));
               foreach (var group in principal.FindAll(authOptions.JwtGroupsClaim)) identity.AddClaim(new Claim(ClaimTypes.Role, group.Value));
               context.Principal = new ClaimsPrincipal(identity);
               return Task.CompletedTask;
           },
       };
   });
   ```

   Only derive the account from the UPN prefix if the company guarantees that UPN prefix equals sAMAccountName. **ASK** Jacob; if it isn't guaranteed, the IdP must emit the sAMAccountName (Entra: an optional or mapped claim of `onpremisessamaccountname`).

5. **Route requests to the right scheme.** In `Program.cs`, extend `ForwardDefaultSelector` so a `Bearer` header goes to JwtBearer. Keep the dev-header branch as it is, and let everything else fall to Negotiate:

   ```csharp
   options.ForwardDefaultSelector = context =>
       jwt && context.Request.Headers.Authorization.ToString().StartsWith("Bearer ", StringComparison.OrdinalIgnoreCase)
           ? JwtBearerDefaults.AuthenticationScheme
           : devHeader && (!authOptions.EnableNegotiate || context.Request.Headers.ContainsKey(DevHeaderAuthenticationHandler.UserHeader))
               ? DevHeaderAuthenticationHandler.SchemeName
               : NegotiateDefaults.AuthenticationScheme;
   ```

   Add the scheme name to `schemes` so `/api/health` reports it.

6. **Groups.** Kerberos+LDAP groups arrive as the AD group's **CN**. Token groups are often GUIDs or sAMAccountNames. If group-based admin (`Auth__AdminGroup`) or share lists by group must work the same way from the portal, the token must carry the same names. Otherwise use `Auth__AdminAccounts__N`, and tell Jacob that portal-side group sharing differs.
7. **Leave the CSRF guard.** The `Sec-Fetch-Site` filter in `MarketplaceEndpoints.cs` still applies, and it's correct.
8. **Tests.** Add integration tests that mint a JWT with a test signing key, configured through `JwtBearerOptions.TokenValidationParameters` in the test factory. Assert the following, then regenerate `server/openapi.json` (command in `server/README.md`):
   - `/api/me` returns `account == "jane@CORP.EXAMPLE.COM"` for a token with `samaccountname=jane`.
   - A dev-header or Negotiate identity with the same account string owns the same namespace.
   - A token without the account claim gets a 401.

### 4.3 Option B: portal (`website/`)

1. `pnpm --filter website add <rangular-package>`, then commit the lockfile.
2. Follow rangular's README to register its providers in `website/src/app/app.config.ts`, and its interceptor for requests to `/api/`. Keep the existing `devUserInterceptor` (`website/src/app/session.ts`); it only acts when the server offers the dev header.
3. In `Session` (`session.ts`), the `signed-out` state should start the rangular login rather than showing the dev sign-in bar. `/downloads/*` stays anonymous: the static file middleware runs before authentication.
4. **CSP** (`PortalHosting.cs`, `ContentSecurityPolicy`):
   - Add the IdP origins to `connect-src` (discovery, token endpoint, JWKS).
   - Add them to `frame-src` if rangular renews tokens silently in an iframe.
   - Keep `script-src 'self'`. If rangular needs inline scripts or `eval`, stop and report it.
5. Register the redirect URI `https://<MARKETPLACE_FQDN>/` (plus rangular's callback path) with the IdP. Ask Jacob who does this.
6. Gates: `pnpm --filter website lint`, `test`, and `build`, plus `pnpm format:check`. The lint rules cap templates at five branches, so split components rather than raising the limit.

**Checkpoint 4 (Option B only):** for two real people, `/api/me` returns the same `account` and `namespace` from the desktop CLI (`agent-plugins whoami --json`) and from the portal's network tab. Test at least one person whose UPN differs from their sAMAccountName, if the company has such people.

### 4.4 CI publishing (config only)

Skill repositories publish from Azure Pipelines with an Entra token (`docs/publish-from-ci.md`). The server code, the pipeline template, and the tests already exist and were verified end to end with GitHub Actions at home. What's left is configuration:

1. **App registration** for the marketplace (ASK Jacob who creates it): Application ID URI `api://agent-plugins-marketplace` (or the house naming), and `requestedAccessTokenVersion: 2` in the manifest. It needs no redirect URI and no secret. Record the tenant ID and the app's client ID in the Facts table.
2. **ConfigMap** (§7.1): the `Auth__Machines__0__*` entry, with the audience set to the app's **client ID**. Version 2 access tokens carry the client ID as `aud`, not the app ID URI.
3. **Template defaults:** in `ci/publish-skills/azure-pipelines.yml`, set `marketplace` to `https://<MARKETPLACE_FQDN>`, and `audience` to the app ID URI from step 1.
4. **A test run:**
   - Create a test skills repo and a WIF service connection for it.
   - Run the pipeline from `docs/publish-from-ci.md`. It should fail with `app:<oid> is not a member of …`.
   - Add that account to a test team, and run it again. It should publish.
5. **Two unknowns to settle during the test run.** Record what happens in the Facts table.
   - Whether `AzureCLI@2` signs in with a service principal that has no subscription role. If it refuses, give the connection's identity `Reader` on an empty resource group. Or replace the task with a PowerShell step that exchanges the service connection's OIDC token (`System.OidcRequestUri`) for an Entra token with a `client_assertion` request for `api://agent-plugins-marketplace/.default`.
   - Whether "Assignment required" on the marketplace's enterprise app blocks pipelines that aren't assigned. Team membership is the real check either way; leave it off unless the house requires it.

**Checkpoint 4.4:** a PR build in the test repo shows the plan in its summary, a merge publishes, and a rerun reports every skill unchanged.

---

## Phase 5: Container image

`server/Dockerfile` builds from the **repository root**. Its stages are: Rust validator, then Angular portal, then .NET publish, then the ASP.NET runtime with krb5. Changes needed:

1. **Base images.** Replace each `FROM` with the Artifactory Docker remote or ACR mirror, or with the company's mandated hardened images:
   - `rust:1-bookworm`
   - `node:22-bookworm-slim`
   - `mcr.microsoft.com/dotnet/sdk:10.0`
   - `mcr.microsoft.com/dotnet/aspnet:10.0`

   Pin by digest. If you use `ARG` prefixes to limit merge conflicts, declare them before the first `FROM`.

2. **Package managers inside the build.** Pass credentials as **BuildKit secrets** so nothing lands in a layer:

   ```dockerfile
   # web stage
   RUN --mount=type=secret,id=npmrc,target=/root/.npmrc npm install --global pnpm@11.9.0
   RUN --mount=type=secret,id=npmrc,target=/root/.npmrc --mount=type=cache,target=/root/.local/share/pnpm/store \
       pnpm install --frozen-lockfile --filter website --ignore-scripts
   ```

   Do the same for cargo in the validator stage (copy `src-tauri/.cargo/config.toml`, and pass the token as a secret env through `--mount=type=secret,id=cargo_token,env=CARGO_REGISTRIES_ARTIFACTORY_TOKEN`) and for NuGet in the build stage (copy `server/nuget.config`, secret-mount the credentials). Build with `docker build --secret id=npmrc,src=$HOME/.npmrc ...`.

3. **apt in the runtime stage.** The runtime installs `krb5-user libgssapi-krb5-2 ca-certificates curl`. Point apt at the Artifactory Debian remote (rewrite `/etc/apt/sources.list.d/debian.sources`) or at the proxy. Keep krb5: it's required for Negotiate under both Option A and Option B.
4. **Corporate CA.** If Artifactory, the proxy, LDAP, or PostgreSQL present certificates from an internal or TLS-inspecting CA:
   - In **every** Debian stage: `COPY corp-root-ca.crt /usr/local/share/ca-certificates/` and then `update-ca-certificates`.
   - In the web stage, also set `ENV NODE_EXTRA_CA_CERTS=/usr/local/share/ca-certificates/corp-root-ca.crt`.
   - The runtime stage needs it too: the API calls Artifactory over HTTPS.
   - A public CA cert is fine to commit. Otherwise mount it as a secret.
5. **Keep everything else.** The image runs as `USER app` (UID 1654), and the Dockerfile sets `Validator__Path`, `Server__DownloadsPath=/srv/downloads`, and `DOTNET_EnableDiagnostics=0`.

**Checkpoint 5:**

- `docker build -f server/Dockerfile -t marketplace-api:local .` succeeds using only Artifactory.
- `docker run --rm marketplace-api:local /app/bin/validate-source --help` runs, or exits with its usage message.
- The image has no `.npmrc` or tokens: `docker history --no-trunc` and `docker run --rm --entrypoint sh marketplace-api:local -c 'ls -la /root 2>/dev/null; find / -name .npmrc 2>/dev/null'` show nothing.

---

## Phase 6: Azure Pipelines

Create two pipelines, or one multi-stage file, following the house templates found in 0.2. Below is the job content to port from `.github/workflows/ci.yml`; everything in it must keep passing.

### 6.1 CI (PR validation and `main`)

**Job `app` (Rust and React).** Prefer a **Windows** agent. The desktop code has `cfg(windows)` paths that Linux clippy never compiles, and Windows needs no WebKit. If only Linux agents exist, install `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`, and make Jacob run the Windows gates locally before each release (§10.2).

```text
node 22, pnpm 11.9.0 (npm i -g pnpm@11.9.0), rust stable + clippy + rustfmt
pnpm install --frozen-lockfile
pnpm typecheck
pnpm lint
pnpm test
pnpm format:check
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --features tools --bins -- -D warnings
```

**Job `web` (Linux):**

```text
pnpm install --frozen-lockfile
pnpm --filter website lint
pnpm --filter website test
pnpm --filter website build
```

**Job `server` (Linux, needs Docker for Testcontainers):**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features --features tools --bin validate-source
dotnet restore server/Marketplace.slnx
dotnet build server/Marketplace.slnx --no-restore -warnaserror
# OpenAPI drift check: keep port 5088; the document records the URL it was served from.
ASPNETCORE_ENVIRONMENT=Development Database__MigrateOnStartup=false Kestrel__Endpoints__Http__Url=http://127.0.0.1:5088 \
  dotnet run --project server/src/Marketplace.Api --no-build --no-launch-profile > /dev/null &
curl -sf --retry 30 --retry-connrefused --retry-delay 1 -o server/openapi.json http://127.0.0.1:5088/openapi/v1.json
kill $!
pnpm exec prettier --write server/openapi.json
git diff --exit-code server/openapi.json
MARKETPLACE_VALIDATOR=$(Build.SourcesDirectory)/src-tauri/target/debug/validate-source \
  dotnet run --project server/tests/Marketplace.Api.Tests --no-build
```

- Testcontainers pulls `postgres:16-alpine` and `testcontainers/ryuk` from Docker Hub. Set `TESTCONTAINERS_HUB_IMAGE_NAME_PREFIX=<artifactory-docker-remote-host>/` so they come from the mirror. If Ryuk is blocked by policy, set `TESTCONTAINERS_RYUK_DISABLED=true`.
- ADO task mapping: `UseDotNet@2` (`version: 10.0.x`), `NodeTool@0` (`versionSpec: 22.x`), rustup or a preinstalled toolchain, and `Cache@2` for the pnpm store, `~/.cargo/registry`, `src-tauri/target`, and `~/.nuget/packages`.
- Registry auth: `npmAuthenticate@0` (with `workingFile: .npmrc`), `NuGetAuthenticate@1`, or secret variables mapped to `NPM_TOKEN`, `CARGO_REGISTRIES_ARTIFACTORY_TOKEN`, and the NuGet credentials. Use whatever the house pattern is.

### 6.2 Release (on `main`, after CI)

1. Build the image with BuildKit secrets (Phase 5), tagged with the short commit SHA `$(Build.SourceVersion)`, and push it to `<REGISTRY>/agent-plugins/marketplace-api:<sha7>`. Capture the digest.
2. Run whatever scanning or signing the house templates require.
3. **Promote through the `deployments` repo**, the way the closest existing app does it. Usually: check out the `deployments` repo as a second `resources.repositories` entry with `persistCredentials: true`, update the image reference to `…:<sha7>@<digest>`, commit, and push or open a PR (`az repos pr create`). The build service identity needs Contribute on that repo; ask Jacob.
4. **Skip the release** when a commit touches only the desktop app and docs. Use path filters, or accept rebuilding the image; the image is deterministic enough.

**Checkpoint 6:** a PR runs CI green on every job; a merge to `main` pushes an image and produces the `deployments` change.

---

## Phase 7: Kubernetes (in the `deployments` repo)

Translate the reference below into the house format (Helm values, Kustomize base and overlays, and so on). The names are suggestions; follow the house convention. Start with a non-production environment if one exists.

### 7.1 Configuration (ConfigMap, non-secret)

```yaml
ASPNETCORE_ENVIRONMENT: Production # NEVER Development
Server__PublicBaseUrl: https://<MARKETPLACE_FQDN>
Server__CatalogName: <Company> marketplace
Server__CatalogDescription: Skills and MCP servers published by people at <Company>.
Server__TrustedProxies__0: <pod CIDR or ingress controller subnet, e.g. 10.244.0.0/16>
Server__DownloadsPath: /srv/downloads
Artifactory__BaseUrl: https://<ARTIFACTORY_URL>
Artifactory__Repository: <PKG_REPO>
Artifactory__Prefix: marketplace
Auth__EnableNegotiate: "true"
Auth__AllowDevHeader: "false"
Auth__LdapDomain: <dns domain> # omit if no AD groups / no LDAP reachability
Auth__AdminGroup: <group CN> # or Auth__AdminAccounts__0: CORP\jane (domain-qualified)
Auth__OfficialPublishers__0: CORP\<account>
Client__MinimumVersion: 0.2.0
Client__LatestVersion: <first work release, e.g. 0.3.0>
KRB5_KTNAME: /etc/krb5/krb5.keytab
KRB5_CLIENT_KTNAME: /etc/krb5/krb5.keytab
# CI publishing (§4.4):
Auth__Machines__0__Authority: https://login.microsoftonline.com/<tenant-id>/v2.0
Auth__Machines__0__Audience: <marketplace app client ID>
Auth__Machines__0__AccountClaim: oid
Auth__Machines__0__Prefix: "app:"
# Option B only:
Auth__JwtAuthority: <issuer>
Auth__JwtAudience: <api audience>
Auth__JwtAccountClaim: <claim>
Auth__JwtRealm: <REALM>
Auth__JwtGroupsClaim: groups
```

Also create a second ConfigMap, `krb5-conf`, holding `krb5.conf` for the realm. Its `[libdefaults] default_realm` is the realm; list the `[realms]` KDCs only if LDAP or group lookup is used.

### 7.2 Secrets (use the house mechanism; never plaintext in Git)

| Key                              | Value                                                                                                                                                                                            |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `ConnectionStrings__Marketplace` | `Host=<pg host>;Port=5432;Database=marketplace;Username=<user>;Password=<pw>;SSL Mode=Require` (Azure flexible server requires TLS)                                                              |
| `Artifactory__Token`             | the `<PKG_REPO>` service account token                                                                                                                                                           |
| `Notifications__WebhookUrl`      | optional Teams webhook                                                                                                                                                                           |
| `krb5.keytab` (file)             | the binary keytab. Create it with `kubectl create secret generic marketplace-krb5 --from-file=krb5.keytab=marketplace.keytab --dry-run=client -o yaml`, then seal or encrypt it per house style. |

If the house requires Entra (managed identity) auth for PostgreSQL instead of a password, stop and report it: Npgsql would need a periodic token provider, which is a code change.

### 7.3 Deployment

```yaml
spec:
  replicas: 1 # see note
  strategy: { type: RollingUpdate, rollingUpdate: { maxSurge: 1, maxUnavailable: 0 } }
  template:
    spec:
      securityContext: { runAsNonRoot: true, runAsUser: 1654, runAsGroup: 1654, fsGroup: 1654, seccompProfile: { type: RuntimeDefault } }
      initContainers:
        - name: installers # §7.4; drop if D5 chose a PVC
          image: <REGISTRY>/agent-plugins/marketplace-api:<sha7>@<digest> # reuses the API image: it has curl
          command: ["sh", "-c", "<script from §7.4>"]
          envFrom: [{ configMapRef: { name: marketplace-api } }, { secretRef: { name: marketplace-api } }]
          env: [{ name: INSTALLERS_URL, value: "https://<ARTIFACTORY_URL>/artifactory/<INSTALLERS_REPO>/agent-plugins" }]
          volumeMounts: [{ name: downloads, mountPath: /srv/downloads }]
          securityContext: { allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: { drop: [ALL] } }
      containers:
        - name: api
          image: <REGISTRY>/agent-plugins/marketplace-api:<sha7>@<digest>
          ports: [{ containerPort: 8080, name: http }]
          envFrom: [{ configMapRef: { name: marketplace-api } }, { secretRef: { name: marketplace-api } }]
          resources: { requests: { cpu: 250m, memory: 512Mi }, limits: { memory: 1536Mi } }
          startupProbe: { httpGet: { path: /api/health, port: http }, periodSeconds: 5, failureThreshold: 60 } # migrations run at start
          readinessProbe: { httpGet: { path: /api/health, port: http }, periodSeconds: 15, timeoutSeconds: 5 }
          livenessProbe: { tcpSocket: { port: http }, periodSeconds: 30 } # not /api/health: a DB outage must not restart pods
          securityContext: { allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: { drop: [ALL] } }
          volumeMounts:
            - { name: tmp, mountPath: /tmp } # validator scratch + upload temp files
            - { name: downloads, mountPath: /srv/downloads, readOnly: true }
            - { name: krb5-keytab, mountPath: /etc/krb5, readOnly: true }
            - { name: krb5-conf, mountPath: /etc/krb5.conf, subPath: krb5.conf, readOnly: true }
      volumes:
        - { name: tmp, emptyDir: { sizeLimit: 2Gi } }
        - { name: downloads, emptyDir: { sizeLimit: 1Gi } }
        - { name: krb5-keytab, secret: { secretName: marketplace-krb5, defaultMode: 0440 } }
        - { name: krb5-conf, configMap: { name: krb5-conf } }
```

Notes:

- **One replica.** Migrations run at startup (`Database__MigrateOnStartup=true`). The rate limiter, the health probe cache, and the 256 MB download cache are all per process. More replicas work, but each keeps its own caches and limits. If you need high availability, first set `Database__MigrateOnStartup=false` on all but a migration Job, and tell Jacob, because the `SelfService` regeneration step in `Program.cs` only runs on the migrating instance.
- **`/api/health`** returns 503 when PostgreSQL is unreachable, and 200 otherwise. The store and LDAP status are fields in the body and are probed at most once a minute. Use it for readiness, not liveness.
- **`readOnlyRootFilesystem`.** If something fails with EROFS, find the path in the logs and add an `emptyDir` for it rather than dropping the setting. ASP.NET may log a warning that data-protection keys are ephemeral. That's harmless: the app issues no cookies.
- **Memory.** Uploads are capped at about 51 MB per request, and archives are held in memory during publish. Watch `kubectl top pod` during the smoke test and adjust.

### 7.4 Installer hosting (D5 = Artifactory plus an init container)

Installers live in `<INSTALLERS_REPO>/agent-plugins/<version>/`, alongside the `manifest.json` for that version (§10.5). At start, the init container fetches the version named by `Client__LatestVersion`, so a single `deployments` change both announces and serves a release:

```sh
set -eu
v="$Client__LatestVersion"
mkdir -p "/srv/downloads/releases/$v"
auth="Authorization: Bearer $Artifactory__Token"
curl -fsS -H "$auth" -o /srv/downloads/manifest.json "$INSTALLERS_URL/$v/manifest.json"
exe="Agent Plugins_${v}_x64-setup.exe"
curl -fsS -H "$auth" -o "/srv/downloads/releases/$v/$exe" "$INSTALLERS_URL/$v/Agent%20Plugins_${v}_x64-setup.exe"
echo "$(sed -n 's/.*"sha256": *"\([0-9a-f]\{64\}\)".*/\1/p' /srv/downloads/manifest.json | head -1)  /srv/downloads/releases/$v/$exe" | sha256sum -c -
```

The token this uses belongs to the `<PKG_REPO>` account. If `<INSTALLERS_REPO>` needs a different one, add a separate secret key.

If D5 chose a PVC instead: use an Azure Files (`azurefile-csi`, RWX) PVC mounted read-only at `/srv/downloads`, and have Jacob upload with `az storage file upload-batch`. Don't mount it read-write in the API pod: whoever can write it can replace the installer the whole fleet downloads.

### 7.5 Service and Ingress

- Service: ClusterIP, port 80 → `http` (8080).
- Ingress: host `<MARKETPLACE_FQDN>`, TLS per the house mechanism.
  - **Body size ≥ 64 MB.** For ingress-nginx: `nginx.ingress.kubernetes.io/proxy-body-size: "64m"`, and `proxy-read-timeout`/`proxy-send-timeout: "120"`.
  - For App Gateway or WAF: raise the request body and file upload limits. WAF rules may flag zip uploads or JSON containing code, so scope any exclusions to `POST /api/packages/*/versions` and `/suggestions`.
  - The proxy must pass the **`Authorization` header unchanged** (ingress-nginx does by default) and must set `X-Forwarded-For` and `X-Forwarded-Proto`.
- **NetworkPolicy**, if the house uses them:
  - Ingress: from the ingress controller only.
  - Egress: DNS, PostgreSQL 5432, Artifactory 443, and DC 88/389 (TCP and UDP) only if LDAP is on.

### 7.6 Register the app

Add the app wherever the GitOps tool or the pipeline lists apps (found in 0.1), in every environment you're targeting.

**Checkpoint 7:** the manifests render (`kustomize build` or `helm template`) with no plaintext secrets; the PR is reviewed by whoever owns `deployments`.

---

## Phase 8: First deployment and smoke test

After the `deployments` change is applied, run these checks in order. Stop at the first failure.

| #   | Check                                                                                               | Expect                                                                                                                                                                              |
| --- | --------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `kubectl -n <ns> rollout status deploy/marketplace-api`                                             | Rolled out                                                                                                                                                                          |
| 2   | `kubectl -n <ns> logs deploy/marketplace-api`                                                       | Migrations applied. **No** warnings about `AllowDevHeader` or `TrustedProxies`.                                                                                                     |
| 3   | `curl -fsS https://<MARKETPLACE_FQDN>/api/health`                                                   | `environment: Production`, `artifactStore: ok`, `ldap: ok` if configured, `authSchemes` includes `Negotiate` (and Bearer for Option B), `latestClientVersion` matches the ConfigMap |
| 4   | `curl -fsS -H 'X-Dev-User: CORP\x' https://<MARKETPLACE_FQDN>/api/me`                               | **401**. If this returns a user, the dev header is on: roll back immediately.                                                                                                       |
| 5   | Jacob, on a domain PC: `Invoke-RestMethod -UseDefaultCredentials https://<MARKETPLACE_FQDN>/api/me` | His account, namespace, `isAdmin` as configured                                                                                                                                     |
| 6   | Jacob opens the portal in Edge                                                                      | Signed in without a prompt (Option A), or through the rangular login (Option B)                                                                                                     |
| 7   | Portal → Publish `examples/hello`                                                                   | The package appears; the zip exists under `<PKG_REPO>/marketplace/...`                                                                                                              |
| 8   | Upload a zip of about 40 MB                                                                         | Not a 413 from the ingress or WAF (the server's own limit is about 51 MB)                                                                                                           |
| 9   | Download page                                                                                       | `/downloads/manifest.json` loads; the installer downloads and its SHA-256 matches                                                                                                   |
| 10  | Desktop app on a work PC (after Phase 10), installed from the portal                                | Install succeeds; the Admin page shows the PC's heartbeat within about 15 minutes                                                                                                   |
| 11  | Revoke the test package (portal → Remove from every PC)                                             | The app uninstalls it on its next sync                                                                                                                                              |
| 12  | Admin purge of the test version                                                                     | Artifactory no longer has the file, including in the trash can if §3.1 cleans it                                                                                                    |
| 13  | Checkpoint 4.4 in the test skills repo                                                              | The PR build's summary shows the plan; the merge publishes; the rerun reports every skill unchanged                                                                                 |

Record the results, then clean up the test package.

---

## Phase 9: Operations handover

Write these down (in the repo's `docs/` or the team wiki; ask Jacob which):

- **Backups.** A managed Postgres PITR retention period, or a nightly `pg_dump` CronJob if Postgres is in-cluster. Include the `<PKG_REPO>` backup policy. Restore order: Artifactory first, then the database (`docs/rollout-guide.md` §5).
- **Upgrades.** Every server release that includes a migration needs a backup or restore point taken first.
- **Keytab rotation.** If the service account password changes, the keytab stops working and **everyone** gets 401. Set the account's password to not expire, or put a rotation runbook (new `ktpass` → update the secret → restart) on the calendar.
- **Incidents.** `docs/rollout-guide.md` §6 applies as written. Add the Artifactory trash step if §3.1 couldn't automate it.
- **Monitoring.** Alert on readiness failures, and on `/api/health` `artifactStore != ok` for 5 minutes or more.

---

## Phase 10: Windows desktop build (Jacob's workstation)

The agent can prepare scripts. Jacob runs them.

### 10.1 One-time workstation setup

1. Visual Studio 2022 Build Tools with the **Desktop development with C++** workload (MSVC and a Windows 10/11 SDK).
2. Rust via rustup, toolchain `stable-x86_64-pc-windows-msvc`, plus `clippy` and `rustfmt`. Set `RUSTUP_DIST_SERVER` if static.rust-lang.org is blocked.
3. Node 22, and `npm i -g pnpm@11.9.0`.
4. The `.npmrc`, cargo config, and tokens from Phase 1, as user environment variables.
5. **Bundler tools.** On its first `tauri build`, Tauri downloads NSIS, the `nsis_tauri_utils` plugin, and WiX 3.14 from GitHub into `%LOCALAPPDATA%\tauri\`. If GitHub is blocked, the build fails at bundling. Fix it by seeding `%LOCALAPPDATA%\tauri\NSIS` and `%LOCALAPPDATA%\tauri\WixTools314` from an allowed source (an Artifactory GitHub remote), or allow those downloads once.

### 10.2 Before a release

Run the full gates on Windows. These cover the `cfg(windows)` code that Linux CI doesn't compile:

```powershell
pnpm install --frozen-lockfile
pnpm typecheck; pnpm lint; pnpm test; pnpm format:check
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

### 10.3 Version bump

Set the new version `X.Y.Z` in all of these:

- `package.json` (`version`)
- `src-tauri/Cargo.toml` (`version`), then `cargo check` to refresh `Cargo.lock`
- `src-tauri/tauri.conf.json` (`version`)
- `server/src/Marketplace.Api/Marketplace.Api.csproj` (`<Version>`), if the server changed too
- `server/src/Marketplace.Api/appsettings.json` and the `MarketplaceOptions.cs` default `LatestVersion` (these are defaults only; the ConfigMap wins)
- `server/releases/manifest.json` `release.version` (the development sample)

### 10.4 Signing: which key is which

- **Tauri updater key** (minisign, `TAURI_SIGNING_PRIVATE_KEY`). This only signs update bundles for `tauri-plugin-updater`. **The app doesn't use the updater plugin**, so this key does nothing today. Updates reach PCs through the portal download page, Intune or ConfigMgr, and the `Client__LatestVersion` "new version available" notice. Adding in-app auto-update is a feature: the plugin, `plugins.updater.pubkey`, an endpoint serving `latest.json`, and `bundle.createUpdaterArtifacts: true`. **ASK** before building it.
- **Authenticode code-signing certificate.** This is what SmartScreen, Defender, AppLocker, and WDAC check. If the company has one:
  - Certificate in the Windows store: add to `src-tauri/tauri.windows.conf.json` under `bundle.windows`: `"certificateThumbprint": "<thumbprint>", "digestAlgorithm": "sha256", "timestampUrl": "<company or public RFC 3161 TSA>"`.
  - Key Vault, Trusted Signing, or an HSM: use `"signCommand": "<tool> ... %1"` instead.
  - **The console twin isn't signed by Tauri.** `agent-plugins-console.exe` is added by `windows/installer-hooks.nsh` and `windows/console-twin.wxs` straight from `target/release`, and it's installed as `agent-plugins.com`. Build in two steps so you can sign it in between:

    ```powershell
    pnpm tauri build --no-bundle
    signtool sign /sha1 <thumbprint> /fd sha256 /tr <tsa> /td sha256 src-tauri\target\release\agent-plugins-console.exe
    pnpm tauri bundle
    ```

  - Afterwards, install the app and check that `Get-AuthenticodeSignature "$env:LOCALAPPDATA\Agent Plugins\*.exe","$env:LOCALAPPDATA\Agent Plugins\agent-plugins.com"` shows `Valid` for every file.

### 10.5 Build and publish

```powershell
pnpm tauri build            # or the two-step signed build above
$v   = (Get-Content src-tauri\tauri.conf.json | ConvertFrom-Json).version
$exe = "src-tauri\target\release\bundle\nsis\Agent Plugins_${v}_x64-setup.exe"
$msi = "src-tauri\target\release\bundle\msi\Agent Plugins_${v}_x64_en-US.msi"
$item = Get-Item -LiteralPath $exe
$manifest = [ordered]@{
  schemaVersion = 1
  release = [ordered]@{
    version = $v
    platforms = @(
      [ordered]@{ id = "windows"; name = "Windows"; format = ".exe"; architecture = "x64"
                  file = "releases/$v/$($item.Name)"; sizeBytes = $item.Length
                  sha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLower() }
      [ordered]@{ id = "linux"; name = "Linux"; format = ".AppImage"; architecture = "x64"; file = $null; sizeBytes = $null; sha256 = $null }
    )
  }
}
# UTF-8 without BOM: the portal's JSON reader rejects a BOM, and PowerShell 5's Set-Content -Encoding utf8 writes one.
[IO.File]::WriteAllText("$PWD\manifest.json", ($manifest | ConvertTo-Json -Depth 5))
```

Then:

1. Upload `manifest.json`, the `.exe`, and the `.msi` to `<INSTALLERS_REPO>/agent-plugins/$v/`, using `jf rt upload` or `curl -T` with a token. The MSI is for IT (Intune or ConfigMgr); the portal serves only the `.exe`.
2. Open a `deployments` PR that sets `Client__LatestVersion: $v`. The pod restarts, and the init container fetches the new installer. With Kustomize `configMapGenerator`, the hash suffix triggers the rollout; otherwise run `kubectl rollout restart`.
3. Raise `Client__MinimumVersion` only when older apps must stop working. They get `426` everywhere except sign-in and usage reports.

The agent should turn §10.5 into `scripts/publish-windows-release.ps1`, taking the version, the Artifactory URL, and the token from the environment. It must not echo the token.

---

## Phase 11: Fleet rollout (IT)

Hand `docs/rollout-guide.md` §4 to whoever owns Intune or GPO, with these values filled in:

- **Install.** The MSI via Intune or ConfigMgr (`msiexec /i "Agent Plugins_<v>_x64_en-US.msi" /qn`, and add `ALLUSERS=""` for a per-user install), or the NSIS `.exe` with `/S`. Deploy the WebView2 Evergreen runtime first on Windows 10 PCs without internet access.
- **Policy** under `HKLM\Software\Policies\AgentPlugins`:
  - `MarketplaceUrl = https://<MARKETPLACE_FQDN>`. Redundant after Phase 2, but it's harmless and lets IT move the server without a rebuild.
  - `LaunchAtLogin = 1`. Revocations and updates only happen while the app runs.
- **Edge and Chrome:**
  - `AutoLaunchProtocolsFromOrigins` allows `agent-plugins` from `https://<MARKETPLACE_FQDN>`.
  - `AuthServerAllowlist` includes `<MARKETPLACE_FQDN>`, if it's not already in the Intranet zone (Option A portal sign-in).
- **Runtime reachability for MCP servers.** Covers `uv` from GitHub (Phase 2.5), PyPI and npm through Artifactory, and the proxy bypass for `<MARKETPLACE_FQDN>`.
- **Pilot.** Roll out to about 10 people first. Check the Admin page's installs and heartbeats before going wider.

---

## 12. Files the port is expected to touch

Keep this list current; it's the merge-conflict map for upstream syncs.

| File                                                                                  | Change                                                  |
| ------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| `src-tauri/src/locator.rs`                                                            | `MARKETPLACE_URL`                                       |
| `src-tauri/tauri.conf.json`                                                           | `identifier` (if D-approved), version                   |
| `src-tauri/tauri.windows.conf.json`                                                   | Authenticode settings (if any)                          |
| `src-tauri/src/startup.rs`                                                            | `uv` URLs (only if GitHub is blocked)                   |
| `src-tauri/.cargo/config.toml`                                                        | new: Artifactory source replacement                     |
| `.npmrc`, `server/nuget.config`                                                       | new: registries, no credentials                         |
| `server/Dockerfile`                                                                   | base images, BuildKit secrets, apt source, corporate CA |
| `server/src/Marketplace.Api/Storage/ArtifactoryStore.cs`                              | new                                                     |
| `server/src/Marketplace.Api/Configuration/MarketplaceOptions.cs`                      | `ArtifactoryOptions` (and JWT options for Option B)     |
| `server/src/Marketplace.Api/Program.cs`                                               | store selection (and the JWT scheme for Option B)       |
| `server/src/Marketplace.Api/appsettings.json`                                         | `Artifactory` section                                   |
| `server/src/Marketplace.Api/Endpoints/PortalHosting.cs`                               | CSP (Option B only)                                     |
| `server/tests/Marketplace.Api.Tests/ArtifactoryStoreTests.cs`                         | new                                                     |
| `website/package.json`, `website/src/app/app.config.ts`, `website/src/app/session.ts` | rangular (Option B only)                                |
| `pnpm-lock.yaml`                                                                      | rangular (Option B only)                                |
| `docs/marketplace-api.md`                                                             | new settings                                            |
| `.github/` → `azure-pipelines*.yml`                                                   | pipelines                                               |
| `scripts/publish-windows-release.ps1`                                                 | new                                                     |
| `ci/publish-skills/azure-pipelines.yml`                                               | `marketplace` and `audience` defaults (§4.4)            |

---

## 13. Gotchas (read before debugging)

- **Kerberos needs the FQDN.** The desktop app asks SSPI for `HTTP/<host as written in the URL>`. Browsers may canonicalize a CNAME first and ask for `HTTP/<canonical name>`, which fails if only the friendly name has an SPN. Use an A record, or register both SPNs, or set Edge/Chrome `DisableAuthNegotiateCnameLookup=true`. Also:
  - Clocks must agree within 5 minutes.
  - A keytab `kvno` that doesn't match the account (someone reset the password) means 401 for everyone.
  - Linux Negotiate supports **Kerberos only, not NTLM**. A PC that falls back to NTLM (off-VPN, or a raw IP in the URL) gets 401. The app's preflight explains this to people.
- **The dev header.** `ASPNETCORE_ENVIRONMENT=Development` turns it on silently. Smoke check #4 exists to catch this.
- **Upload limits apply at every hop:** the ingress, the WAF, App Gateway, and any corporate proxy. The server's limit is about 51 MB.
- **Artifactory overwrite.** The service account can overwrite, so immutability depends on HEAD-before-PUT in `ArtifactoryStore`. Don't remove that check.
- **Artifactory trash can** keeps "purged" leaked secrets. See §3.1.
- **Corporate CA.** Node, cargo, apt, NuGet, and the runtime each read CAs differently; see Phase 5.4. The desktop app uses the Windows certificate store (`rustls-platform-verifier`), so an internal CA pushed by GPO just works there. Don't switch it back to `rustls-native-certs`.
- **PowerShell JSON.** PowerShell 5's `Set-Content -Encoding utf8` writes a BOM, which the app and portal JSON readers reject. Use `[IO.File]::WriteAllText`.
- **Recursive deletes on Windows.** Use only `Remove-Item -LiteralPath <path> -Recurse -Force`, after asserting the resolved path is absolute and inside the expected parent. Never build `rd /s` or `rm -rf` from interpolated paths across shells.
- **OpenAPI drift.** Any endpoint change needs `server/openapi.json` regenerated on port 5088 (`server/README.md`), or CI fails.
- **Server tests.** Use `dotnet run --project server/tests/Marketplace.Api.Tests`; `dotnet test` fails on .NET 10 here. Set `MARKETPLACE_VALIDATOR` to a copied `validate-source` binary if Rust builds run at the same time.
- **The Windows installer's per-user MSI** installs into `%LOCALAPPDATA%\Agent Plugins` regardless of `INSTALLDIR`, the same place as the NSIS build. Don't mix the two installers on one PC.
