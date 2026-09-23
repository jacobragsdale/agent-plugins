# Web portal

The Agent Plugins web portal: the download page, browsing, publishing (upload a skill folder, pack, or zip), My skills, admin review, and the technical reference. It is an Angular app that the marketplace server builds into its image and serves at `/` beside `/api` ([ADR 0006](../docs/decisions/0006-web-portal-and-review.md)), so the browser's Windows sign-in covers both.

## Develop

Start the server and its database, then the Angular dev server, which proxies `/api` and `/downloads` to port 8080:

```bash
docker compose -f server/compose.yaml up --build -d
pnpm --filter website start
```

Open <http://localhost:4200>. The development server trusts a typed account name, so the portal shows a sign-in bar; `jacob` is an admin in `server/compose.yaml`. Publishing needs a reachable Artifact Keeper (`ARTIFACT_KEEPER_URL`).

## Check

```bash
pnpm --filter website lint
pnpm --filter website test
pnpm --filter website build     # the Angular compiler, including strict template checks
pnpm format:check               # from the repository root
```

Lint enforces the house TypeScript and Angular rules, including a limit of five branches per template: split a component rather than raising it.

## Publish installers

The download page reads `/downloads/manifest.json`, which the server serves from `Server:DownloadsPath` (`server/releases/` in the development compose file).

1. Create `server/releases/releases/<version>/`.
2. Copy the Windows `.exe` and Linux `.AppImage` into it.
3. Record each file's path (relative to `server/releases/`), byte size, and SHA-256 digest in `server/releases/manifest.json`.

Installers are ignored by Git; the tracked manifest has both downloads disabled. Changes appear without restarting the server.
