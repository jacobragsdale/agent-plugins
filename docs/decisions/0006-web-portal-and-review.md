# ADR 0006: Web portal and publish review

- Status: accepted
- Date: 2026-09-22
- Extends: [ADR 0004](0004-internal-marketplace.md) and [ADR 0005](0005-marketplace-access-control.md)

## Context

Publishing needed the CLI, and most intended publishers are not developers. The download page was a separate static site that knew nothing about the marketplace. Anything published went live for everyone at once, including MCP servers, which run programs on colleagues' PCs.

## Decision

### The portal is served by the marketplace, from the same origin

`website/` is an Angular application built into the server image and served from `wwwroot` at `/`, beside `/api`. Same origin means the browser's Windows sign-in (Negotiate) covers the API with no CORS and no tokens, and one deployment carries both. Static files and the SPA fallback need no authentication; unknown `/api/*` routes stay JSON 404s. The installer folder is served at `/downloads` from `Server:DownloadsPath`. The Caddy site is gone. Where the development header is enabled, the portal asks for an account name and sends it as `X-Dev-User`.

### New packages and MCP servers wait for an admin

Each version has a review state: pending, approved, or rejected. A version is approved on publish when the publisher is an admin, or when the package already has a live version and this version contains no MCP server. Everything else waits. Only approved, non-withdrawn versions are live: the index, catalog, namespace archives, and readers of package detail, stats, and files see nothing else. Owners and admins see pending and rejected versions with the reviewer's note. A pending version leaves the listing (name, description, tags) untouched until approved. Rejecting requires a note; a rejected version number stays used.

Reviewers are admins; there is no separate role. Notifications are in the portal only: a count on the Admin link, status on My skills.

### Browser uploads are wrapped by the same Rust code as the CLI

The publish endpoint also accepts files with relative paths, or a zip without a manifest. The portal has no in-browser editor: skills are written locally and uploaded, and a change is a new upload. The server runs `validate-source stage`, which is the CLI's wrapping moved into `src-tauri/src/staging.rs`: a skill directory, a folder of skill directories (a skill pack: one package with one skill component each), an MCP document, or a source tree. Validation runs with `--secrets`, so every upload, CLI or browser, is refused if it looks like it carries credentials.

### Moderation

Admins resolve reports (`ResolvedAt`, `ResolvedBy`) and can withdraw any version, since `Owns` already includes admins.

**Amended 2026-09-22:** withdrawing is reversible. `PUT …/versions/{version}/yank` withdraws a version and `DELETE` restores it, for owners and admins alike; the package's listing follows whichever version is live afterwards. The version number stays used either way.

## Consequences

Packages may hold several components (a skill pack), which ADR 0005's "one skill or one MCP document each" no longer describes; access rules stay per package. The first publish of every non-admin package now needs a reviewer, so admins must watch the queue. The CLI prints `submitted … for review` for pending versions. File downloads from the portal rely on the browser's Windows sign-in, so in the development-header setup only previews work, not direct file links.
