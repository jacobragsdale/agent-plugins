# TODO

Roadmap items that aren't scheduled yet. The work port has its own runbook in `WORK_PORT_PLAN.md`.

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
