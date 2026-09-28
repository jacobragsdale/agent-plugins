# TODO

Work to do now, before the port. `WORK_PORT_PLAN.md` is the runbook for bringing this into the internal network later.

When an item here lands, update `WORK_PORT_PLAN.md` in the same change so the port stays as small as possible:

- drop steps the item made unnecessary
- turn code steps into config steps
- add any new config or facts the port has to supply

## Marketplace for cloud workers and CI

Cloud workers running agents should pull their skills from the marketplace, and teams whose skills live in a repo should validate and publish from CI. Neither needs a Linux build of the CLI: workers and pipelines use `curl`.

### 0. Machine identities

Can start before the port. It pre-builds most of `WORK_PORT_PLAN.md` §4.2, so update that section when this lands.

- [ ] JwtBearer for people, as §4.2 specifies: `Jwt*` options, the scheme registered only when `JwtAuthority` is set, claims canonicalized in `OnTokenValidated`, and `Bearer` routed in `ForwardDefaultSelector`. Off by default.
- [ ] Machine principals: a `JwtMachineClaim` option (`azp`/`appid` on Entra, `sub` on GitHub) maps a token with no person claim to `app:<value>`.
  - Share lists and team membership match it only by the exact name, never by the username rule (`jane` ↔ `CORP\jane`).
  - It gets no personal namespace, so it publishes only through team membership.
  - It is never an admin.
- [ ] Tests that mint JWTs with a test signing key:
  - a person token resolves to the same account as the dev header
  - a machine sees only what is shared with it
  - a machine in a team can publish, and one outside it is refused
  - a machine is never an admin
- [ ] End to end at home with GitHub Actions OIDC (`token.actions.githubusercontent.com`): a workflow publishes a test skill with `curl`.
- [ ] At work: the canonical Kerberos account form, the claim that holds sAMAccountName, the realm, and the Entra app registration. Then managed or workload identity for workers, and Negotiate and Bearer side by side on the real ingress.

### 1. Validation template for skill repos

Can start now.

- [ ] An Azure Pipelines template that runs `/app/bin/validate-source` from the server image against the repo on every PR.
- [ ] A how-to next to `docs/publish-to-marketplace.md`.

### 2. Bundle endpoint for workers

Can start now against dev-header auth.

- [ ] `GET /api/bundles/{ns}/{id}/skills.tar.gz`:
  - returns the live versions of the member packages the caller can see, flattened to `<skill>/SKILL.md`
  - has a strong `ETag` and answers `304`
  - skips MCP components
  - records an install for the caller, so worker pools show up in stats
- [ ] Tests for visibility filtering, revoked members, and the `ETag`.
- [ ] Regenerate `server/openapi.json` and document the endpoint in `docs/marketplace-api.md`.

### 3. Worker how-to

Needs machine identities.

- [ ] A boot snippet: get a token, then `curl | tar -xz` into `~/.claude/skills` or `~/.agents/skills`.
- [ ] A refresh loop for long-lived workers. The `ETag` keeps each check cheap, and a revoked package is gone at the next refresh.
- [ ] Setup: make a bundle for the pool and share it with the pool's identity or AD group.

### 4. CI publish

Needs machine identities.

- [ ] Extend the validation template: on a tag, `curl` `POST /packages/{ns}/{id}/versions` with the pipeline's token and the version from the tag. The server validates it again.
- [ ] Setup: add the pipeline's identity to the team namespace.

### Not planned until someone needs it

- **A headless Linux CLI.** It would need a `cli` Cargo feature without Tauri. `curl` covers both workers and CI.
- **MCP servers in workers.** They need per-worker config and secrets, which belong to the pool's own deployment.
- **Marketplace-issued tokens.** Only needed if workers run somewhere without Entra.

## Desktop app performance

Weak Windows VMs with antivirus are the target. The first pass (2026-09-27) made these changes:

- Removed animations, the gradient background and Radix's card layers.
- Memoized cards and deferred the search filter.
- Kept agent detection out of state reloads.
- Moved item command file work onto blocking workers.

What's left, in order of payoff:

- [ ] **Measure on a Windows VM.** Profile scrolling, typing in search, and Install over CDP into WebView2 (see the Windows harness notes), before and after each item below.
- [ ] **Stop rebuilding the whole state after every click.** `refreshAfterOperation` calls `load_cached_manifest_state`, which rebuilds everything under `operation_lock`. Return the changed item's state from the command instead.
- [ ] **Persist each activated revision's catalog and digests** to a JSON file next to the revision. Revisions are immutable, and today the first load after launch hashes every skill file again (`catalog.rs:235,253`, `digest.rs:53`).
- [ ] **Read `choices` and `invocation` once per rebuild**, not once per item and per plan (`project.rs:329`, `planner.rs:174`).
- [ ] **Memoize `directory_digest_with_leftovers`** (`matching.rs:105`). It re-hashes every edited skill on every rebuild.
- [ ] **Release `sync_lock` in `run_preflight`** before the network checks (`sync.rs:53-80`), so links and focus syncs don't wait on a slow server.
- [ ] **Send a small "changed" signal instead of the whole `AppState`** on `scheduled-sync` when nothing but the timestamps moved.
- [ ] **Compute the program search directories once at startup** (`startup.rs:725-750`).
