# Self-service marketplace plan

**Status:** Proposed<br>
**Date:** 2026-09-26<br>
**Scope:** Implementation plan for installing from the website, self-service teams, public and private sharing, owner review in place of admin review, and bundles. Extends [ADR 0004](docs/decisions/0004-internal-marketplace.md), [0005](docs/decisions/0005-marketplace-access-control.md), and [0006](docs/decisions/0006-web-portal-and-review.md).

## Goals

1. **Install from the website in one click.** An Install button on the portal opens the desktop app, which confirms and installs.
2. **Anyone can create a team.** People are added by name or by invite link. Teams no longer depend on AD groups.
3. **Public and private.** Spaces (personal or team) and individual skills can each be public or private. Owners share with people, teams, or a link.
4. **Owners, not admins, review.** Anyone can suggest a change to a skill and its owners decide. Admins keep one narrow gate, for MCP servers going public, plus a kill switch.
5. **Bundles.** A bundle is a named list of existing packages. You can install it all at once, or install each member on its own.

## Decisions

| Topic                      | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Invite link                | Grants **membership**, not a publish credential. Every publish is still recorded against the Windows account that made it.                                                                                                                                                                                                                                                                                                                                                                               |
| Links (invites and shares) | A 128-bit random base64url code at `/l/<code>`. Reusable and doesn't expire. Created the first time someone chooses **Copy link**. **Reset** replaces it. Stored in plaintext, because members can forward it anyway and database access already means full control.                                                                                                                                                                                                                                     |
| Opening a link             | A **share** link accepts on open, since being given access costs the recipient nothing. A **team invite** link asks first, since joining makes you a publisher and a visible member.                                                                                                                                                                                                                                                                                                                     |
| Team roles                 | **Owner:** manages members and the link, sets the team's visibility, renames the team, deletes it if empty. **Member:** publishes, withdraws, shares, and edits bundles in the team's space. The last owner can't leave. Admins can do everything.                                                                                                                                                                                                                                                       |
| Visibility                 | A space is Public or Private. A skill is Same as its space, Public, or Private. The most specific setting wins. Private means owners plus the share list (people, teams, and optionally AD groups). Hidden things answer 404. Sharing grants see and install only.                                                                                                                                                                                                                                       |
| Defaults                   | Personal spaces are Public, as today. A new team chooses when it's created, with Private as the default.                                                                                                                                                                                                                                                                                                                                                                                                 |
| Review                     | No admin queue. Every publish goes live immediately. Non-owners send **suggestions**, which owners accept or decline.                                                                                                                                                                                                                                                                                                                                                                                    |
| Public MCP gate            | A package with an MCP server can be seen by the public only after an admin approves **the package** (once). Owners, team members, and people it's shared with see it straight away. Later versions don't need an admin, because the app already asks each person to approve every MCP install and update (`application/sync.rs`, background updates). The gate is per package, not per version, because each namespace has one stored archive and can't serve different versions to different audiences. |
| Kill switch                | Owners and admins can **Remove from every PC**. The index lists revoked IDs, and the app uninstalls them at its next sync.                                                                                                                                                                                                                                                                                                                                                                               |
| Bundles                    | A bundle **references** packages; it isn't a package. It has no archive, no versions, and no review, and its members always track their latest version. It can include anyone's packages that you can see. Its ID can't equal a package ID in the same namespace. It isn't followed: members added later appear as "Install the rest".                                                                                                                                                                   |
| Skill packs                | Stay as they are. A pack is one upload and one version. A bundle is a list of skills that already exist.                                                                                                                                                                                                                                                                                                                                                                                                 |
| Deep links                 | `agent-plugins://install/<ns>/<id>[/<skill>]` and `agent-plugins://open/<ns>/<id>`. They carry IDs only. The app always confirms in its own window, one dialog at a time.                                                                                                                                                                                                                                                                                                                                |

## Data model

The migrations, in phase order:

| Phase | Change                                                                                                                                                                                                                    |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2     | `Publishers.Kind` (`personal`, `team`, `official`). Migrated as: `official` becomes official, and any other row where `Account == Namespace` becomes team.                                                                |
| 2     | `TeamMembers(Namespace, Account, IsOwner, JoinedAt)`, primary key `(Namespace, Account)`. Accounts match exactly, ignoring case.                                                                                          |
| 2     | `People(Account, DisplayName, LastSeenAt)`. Updated at most daily by the identity filter. Supplies display names and the directory search.                                                                                |
| 2     | `Links(Code, Kind, Target, CreatedBy, CreatedAt)`. `Kind` is `invite` or `share`; `Target` is `ns`, `ns/pkg`, or `ns/bundle`. Unique on `(Kind, Target)`.                                                                 |
| 3     | `AccessRules.Private` (existing rows become `true`) and `AccessRules.Teams[]`.                                                                                                                                            |
| 4     | `Packages.RevokedAt`, `RevokedBy`, `McpApprovedBy`, `McpApprovedAt`, `McpReviewNote`.                                                                                                                                     |
| 4     | `Suggestions(Id, Namespace, PackageId, StoragePath, ArchiveDigest, SizeBytes, ManifestJson, BaseVersion, Message, SuggestedBy, SuggestedAt, State, DecidedBy, DecidedAt, DecisionNote, AcceptedVersion)`.                 |
| 4     | Drop `PackageVersions.ReviewState`, `ReviewedBy`, `ReviewedAt`, `ReviewNote`, after converting: pending becomes live; rejected becomes yanked; a package whose live MCP version an admin approved is marked MCP-approved. |
| 5     | `Bundles(Namespace, BundleId, Name, Description, Members[], UpdatedBy, CreatedAt, UpdatedAt)`.                                                                                                                            |

## Phases

```
0 ─► 1 Install from the website        (independent; ship first)
0 ─► 2 Teams ─► 3 Visibility and sharing ─► 4 Owner review + kill switch
                                         └─► 5 Bundles (can run beside 4)
```

Every phase passes the full CI gate before it merges:

- `pnpm typecheck`, `lint`, `test`, `format:check`, `build`, including the portal
- `cargo fmt --check`, `clippy -D warnings` (default features and the `tools` feature), `cargo test`
- `dotnet build -warnaserror`, the `openapi.json` freshness check, and the integration tests

`AGENTS.md` forbids dead code, so removed features take every reference with them.

Every phase is also checked end to end on the Windows harness before release.

### Phase 0: Prerequisites

- Write ADR 0007 from the Decisions table above. Each later phase amends it if the code changes a decision.
- Mount `/downloads` on the home-lab marketplace, which currently gives a 404 there. **Get Agent Plugins** depends on it. This change is in the home-server repo and uses that repo's skill.
- Check whether the live config sets `Auth:TeamNamespaces` (also in the home-server repo). Phase 2's migration keeps those teams, but they will have no members.

### Phase 1: Install from the website

**Desktop**

_Registering the link handler_

- In `startup.rs`, next to the PATH step, write `HKCU\Software\Classes\agent-plugins`:
  - `URL Protocol`
  - `DefaultIcon`
  - `shell\open\command` = `"<running agent-plugins.exe>" "%1"`
- Rewrite the key only when it differs. This works with both the NSIS and MSI installers, and `winreg` is already a dependency.
- `remove-from-path` (`cli.rs`, which the uninstaller calls) also deletes the key.

_Handling the link_

- `cli::maybe_run` returns `None` for a first argument starting with `agent-plugins:`, which opens the window. Today that argument exits with status 2.
- A new `deep_link.rs` holds `parse(&str) -> Option<DeepLink>`. The scheme, the verb, and each segment must match the source-id and package-id patterns. Anything else returns `None`.
- `lib.rs:69`: the single-instance callback parses its arguments, saves the pending link, sends a `deep-link` event, and opens the window.
  - On a cold start, `setup` parses `std::env::args()`.
  - The IPC call `take_pending_link` lets the frontend pick up a link that arrived before it was listening.

_Frontend (`App.tsx`)_

- On a link, sync if the item isn't in the catalog, then show `LinkInstallDialog` ("Install X? Adds N skills to …").
- Install through `changeItem`, which already runs `reviewApproval` for MCP servers.
- `open` scrolls to the card and highlights it.
- A new link replaces a dialog that is still waiting.

**CLI**

- For parity with per-skill links, `install <ns>/<pkg>/<skill>` installs one component of a pack. `application::install_item` already takes a component ID.

**Server**

- `/api/me` gains `app: { version, os, lastSeenAt, installed[] } | null`.
  - It is built from the caller's `Heartbeats` row, if seen in the last 30 days, the same window as the installed base.
  - `installed[]` also applies install and uninstall events that arrived after that heartbeat.
- Regenerate `openapi.json`.

**Portal**

- A new `shared/install-button.ts`. Its states:
  - **Install in Agent Plugins**
  - **✓ Installed** with **Open in Agent Plugins**
  - **Get Agent Plugins** (no app seen)
  - **Update Agent Plugins** (`app.version` is older than the first version that handles links)
- Clicking sets `location.href` to the link and shows "Opening Agent Plugins… Nothing happened? Get the app".
- It re-fetches `/api/me` when the tab becomes visible again, so the button changes to Installed.
- It goes on `package-side.html` (replacing the numbered install steps) and on `shared/package-card.ts`.
- The copyable CLI command (`package-parts.ts:357`) moves under **Other ways to install**.
- Pack pages get a per-skill Install.

**Docs**

- `app-reference.md`: links and the confirmation dialog.
- `cli-reference.md`: how a link argument is routed, and the component install.
- `marketplace-api.md`: `/api/me.app`.
- `install-a-package.md`: "From the website", plus the Edge and Chrome `AutoLaunchProtocolsFromOrigins` policy that removes the browser's prompt.

**Tests**

- Rust: `deep_link::parse` accepts valid links and rejects a wrong scheme, extra segments, bad IDs, percent-encoded separators, and query strings.
- Rust: `maybe_run` routes link arguments to the window.
- Server: `/api/me.app` with a missing, stale, or fresh heartbeat, and with an install event after the heartbeat.
- Portal: a spec for how the install button picks its state.

**Done when** a click in Edge on the Windows VM installs a skill and an MCP server, the MCP server through its approval dialog, both with the app already running and with it closed.

### Phase 2: Teams

**Server**

_Migration and identity_

- The `AddTeams` migration from the data model above.
- Identity (`Auth/MarketplaceIdentity.cs` and `ResolveMarketplaceIdentityAsync`):
  - Load memberships and append them to `Namespaces`.
  - Expose `IsTeamOwner(ns)`.
  - Update `People`.
  - `IsReserved`, `Lane`, and `NamespaceDisplayName` read `Publishers.Kind` and `DisplayName`. The `u-<name>` rule is kept, so a personal name that equals a team still gets `u-<name>`.

_Removed_

- `TeamNamespaceOptions`, `AuthOptions.TeamNamespaces`, the validation in `Program.cs:42-52`, and the team settings in `MarketplaceApiFactory.cs:57-59`.
- LDAP groups and `X-Dev-Groups` stay, for optional group entries in access rules.

_New code_

- A new `Teams/TeamService.cs` and these endpoints:

| Endpoint                                   | Who                                | Does                                                                                                                                                                                                                            |
| ------------------------------------------ | ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /api/teams`                          | anyone                             | Takes `{ namespace, displayName, visibility }`. The namespace uses the `source.id` pattern and must be unclaimed: no `Publishers` row, not `official`. Uses the existing namespace advisory lock. The caller becomes the owner. |
| `GET /api/teams`                           | anyone                             | The caller's teams.                                                                                                                                                                                                             |
| `GET /api/teams/{ns}`                      | members                            | Members with display names, and the invite link.                                                                                                                                                                                |
| `PUT /api/teams/{ns}`                      | owner                              | Changes the display name. The namespace can never change, because installed skill names are built from it.                                                                                                                      |
| `PUT /api/teams/{ns}/members/{account}`    | owner                              | Adds a member, or sets `{ owner }`.                                                                                                                                                                                             |
| `DELETE /api/teams/{ns}/members/{account}` | owner, or yourself                 | Removes a member, or leaves. The last owner can't.                                                                                                                                                                              |
| `POST /api/teams/{ns}/invite`              | members to create, owners to reset | Creates or resets the invite link.                                                                                                                                                                                              |
| `DELETE /api/teams/{ns}`                   | owner                              | Only when the team has no packages or bundles. Removes its members, links, and rules.                                                                                                                                           |
| `GET /api/directory?q=`                    | anyone                             | Up to 10 people from `People`, plus teams the caller can see.                                                                                                                                                                   |
| `GET /api/links/{code}`                    | anyone                             | A preview: `{ kind, target, name, by }`.                                                                                                                                                                                        |
| `POST /api/links/{code}`                   | anyone                             | Redeems the link. In this phase, `invite` joins the team.                                                                                                                                                                       |

**CLI** (`cli.rs`, with HTTP helpers in `marketplace.rs`)

```
agent-plugins team                                   my teams
agent-plugins team create <ns> --name "…" [--public] prints the invite link
agent-plugins team <ns>                              members + invite link
agent-plugins team add <ns> <account>
agent-plugins team remove <ns> <account>
agent-plugins team leave <ns>
agent-plugins team invite <ns> [--reset]
agent-plugins team join <link-or-code>
```

**Portal**

- `/teams`: a list, plus **Create a team**. It suggests a namespace from the display name and asks who can see the team's skills.
- `/teams/:ns`: members, **Add people** (searches the directory), **Make owner**, **Remove**, **Copy/Reset invite link**, **Leave**, **Rename**, **Delete**.
- `/l/:code`: for a team invite, a preview and a **Join** button.
- A **Teams** link in the top bar.
- A **Create a team** link in the space picker on `publish.ts`.

**Desktop**

- IPC: `list_teams`, `create_team`, `get_team`, `add_team_member`, `remove_team_member`, `leave_team`, `reset_team_invite`, `open_link`. Each is a thin wrapper around the functions the CLI uses.
- `components/TeamsDialog.tsx`, opened from a **Teams** header button: my teams, create, members, copy invite link, and an **Open a link** box.
- `AppIdentity` gains `namespaces`.

**Agents**

- `marketplace/skills/publish/SKILL.md` and the Create a skill prompt in `tutorial.rs` ask "just you, or one of your teams?", using the spaces `whoami` lists.

**Docs**

- ADR 0007, teams section.
- `marketplace-api.md`: teams, directory, and links. Remove `Auth:TeamNamespaces` from the configuration table.
- `cli-reference.md`, `app-reference.md`, and `publish-to-marketplace.md` ("Publish to a team").

**Tests**

- Creating a team makes the caller its owner.
- A name held by a person, `official`, or an existing team is refused with 409.
- A member who joined by link can publish; a non-member gets 403, and so does a removed member.
- After a reset, the old code answers 404.
- The last owner can't leave. Only an empty team can be deleted.
- The `u-<name>` rule applies against teams.
- Directory search works.
- Integration tests that used the config team now create it through the API.

**Release:** if Phase 0 found configured teams, an admin opens each team's page and shares its invite link.

### Phase 3: Visibility and sharing

**Server**

_Migration and the visibility rule_

- The `AddVisibility` migration.
- `AccessService.IsVisible`:
  - Owners always see it.
  - Otherwise use the package's rule, falling back to the space's rule.
  - No rule, or `Private == false`, means visible to everyone.
  - Otherwise the caller must be listed in `Users`, `Teams`, or `Groups`.
  - The index, catalog, archive filter, and package and file endpoints already call this function.

_Access API_

- `GET` and `PUT /api/access/{ns}/{pkg?}` take `{ visibility: "public" | "private" | "inherit", users, teams, groups }` and return the current `link` as well.
  - `inherit` applies to packages only. `public` on a space deletes its rule.
  - A team space's visibility is set by team owners only.
- `POST /api/access/{ns}/{pkg?}/link` creates or resets a share link.

_Links, index, and new teams_

- `POST /api/links/{code}` for a share: if the caller can't already see the target, add their account to the rule's `Users`. Either way, return the target.
- The index gains `sharedWithYou`: true when a package is visible to you only through a share entry.
- `POST /api/teams` applies `visibility`, which defaults to private.
- `/api/health` gains `adGroups`, true when `Auth:LdapDomain` is set, so clients show a Groups field only where it works.

**CLI**

- `share` replaces `access`:

```
agent-plugins share <ns>[/<pkg>]                         show setting, list, link
agent-plugins share <ns>[/<pkg>] --public | --private | --inherit
agent-plugins share <ns>[/<pkg>] --add <account|team:ns|group:name>... --remove <…>...
agent-plugins share <ns>[/<pkg>] --link [--reset]
```

- `install <link>` redeems the link, then installs.

**Portal**

- `ShareDialog` replaces `VisibilityDialog` in `shared/dialogs.ts`. It has:
  - general access (Same as space, Private, or Public)
  - a people and team picker backed by the directory
  - Groups, only when `adGroups` is set
  - **Copy link** and **Reset**
- **Share** buttons on the package page, on each space in My skills, and on the team page.
- `/l/:code` for a share: redeem on load, then go to `/p/…` with a "_X_ shared this with you" banner and the install button.
- Visibility badges: Private, Public, Shared with you.

**Desktop**

- IPC: `get_share`, `set_share`, `share_link`.
- `ShareDialog.tsx`, opened from **Share…** on the cards of items in your namespaces.
- A **Shared with you** badge.
- **Open a link** accepts share links: it redeems, syncs, scrolls to the skill, and shows the install confirmation.

**Docs**

- ADR 0007: the visibility section, which replaces ADR 0005's "empty list means public".
- `marketplace-api.md` (access and links) and `cli-reference.md` (`share`).
- `help.html:367-376`: the `access` examples become `share`.
- `app-reference.md`.

**Tests**

- A private space answers 404 to non-members on the index, catalog, archive, package, and file endpoints.
- A public skill inside a private space is visible, and a private skill inside a public space is hidden.
- Sharing by user, team, and group each work.
- A link adds the person who opens it, repeat opens change nothing, and a forwarded link works for a third person.
- After a reset, the old code answers 404.
- Only team owners can change a team space's visibility.
- The migration keeps existing rules private.

### Phase 4: Owner review and the kill switch

Ship the kill switch in the same release that removes the admin queue. Raise `Client:MinimumVersion` to that release, so every app still running knows how to uninstall a revoked package.

**Server**

_Migration and liveness_

- The `ReplaceReview` migration from the data model above.
- A version is live when it isn't yanked and its package isn't revoked. Update every place that reads `ReviewState`:
  - `PublishService` (the approval branch, `LatestVersion`, and the review queue)
  - `CatalogService`
  - `AccessService.LivePackagesAsync`
  - `EventsService`
  - `MarketplaceEndpoints`
  - `MarketplaceDbContext`

_Publish and the public MCP gate_

- Publishing no longer has an approval step, and its response no longer includes `reviewState`.
- `IsVisible` adds the public MCP gate: if the package is visible because it's public, its live version has an MCP server, and it isn't MCP-approved, then only owners, team members, and admins see it.
- `GET /api/admin/reviews` lists public, unapproved MCP packages. `POST /api/admin/reviews/{ns}/{pkg}` approves, or declines with a note that the owner sees.

_Kill switch_

- `PUT` and `DELETE /api/packages/{ns}/{pkg}/revoke`, for owners and admins.
- The index gains `revoked[]`: revoked IDs the caller can see, plus any in the caller's latest heartbeat. That way someone who lost access still gets the removal.

_Suggestions_

| Endpoint                                            | Who                                      | Does                                                                                                                                                                                                             |
| --------------------------------------------------- | ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /api/packages/{ns}/{pkg}/suggestions`         | viewers who aren't owners                | The same multipart form, staging, validator, and credential scan as publish. Stored at `{ns}/{pkg}/suggestions/{id}.zip`. Records `BaseVersion` and a message.                                                   |
| `GET /api/packages/{ns}/{pkg}/suggestions`          | owners see all; suggesters see their own |                                                                                                                                                                                                                  |
| `GET …/suggestions/{id}/files` and `…/files/{path}` | owners, and the suggester                | Each file marked added, changed, removed, or unchanged compared with the live version, using `ArchiveInspector`.                                                                                                 |
| `POST …/suggestions/{id}`                           | owners                                   | Takes `{ decision, version?, note? }`. Accepting publishes the stored archive as `version` (the next patch by default), credited to the suggester, with the message as the changelog. Declining requires a note. |
| `DELETE …/suggestions/{id}`                         | the suggester                            | Withdraws a suggestion that is still pending.                                                                                                                                                                    |

- `/api/mine` gains suggestions waiting on you and the state of your own.

**CLI**

- `publish` to a package you can see but don't own offers to send it as a suggestion. Today the CLI refuses a namespace you don't own before uploading.
- The "submitted … for review" branch is removed.
- New verbs:

```
agent-plugins review                                   suggestions waiting on you
agent-plugins review <id> --accept [--version x.y.z]
agent-plugins review <id> --decline "<note>"
agent-plugins revoke <ns>/<pkg> [--undo]
```

**Portal**

- Package page:
  - **Suggest a change**, which is `publish.ts` in a new `suggest` mode.
  - For owners, a **Suggestions** tab and **Remove from every PC** (a danger confirmation).
  - The status "Waiting for an admin before everyone can see it".
- My skills: a "Waiting on you" list, opening a suggestion view with changed-file markers, the file viewer, **Accept** with a version, and **Decline** with a note.
- Admin: the public MCP queue, reports, and revoking any package.
- Removed: the pending and rejected version UI in `shared/status.ts` and `package-parts.ts`, and the review wording in `help.html`.

**Desktop**

- In `application/sync.rs`, after the index loads, uninstall every installed item listed in `revoked[]`. This uses `uninstall_item_components(…, force_modified: true)`, so edited copies are backed up.
- It then shows a background notice: "Removed _X_: its publisher or an admin pulled it from every PC."
- Owners see an "_N_ suggestions waiting" badge that opens My skills in the portal.

**Agents**

- `tutorial.rs:48` drops "submitted … for review".
- `publish/SKILL.md` explains suggestions.

**Docs**

- ADR 0007: the review section, which amends ADR 0006.
- `marketplace-api.md`: review, suggestions, and revoke.
- `publish-to-marketplace.md`, `cli-reference.md`, `app-reference.md` (the revoked notice), and `help.html`.

**Tests**

- A non-admin's first version goes live immediately.
- An MCP package in a public space is hidden from others until an admin approves it, while owners, members, and people it's shared with see it.
- Making an MCP package public puts it in the admin queue.
- A revoked package leaves the catalog and appears in `revoked[]`, including for someone who lost access but reported it installed.
- Suggestions:
  - A viewer can suggest; a non-viewer gets 404.
  - An accepted suggestion is credited to the suggester.
  - Declining requires a note.
  - A changed base version is flagged.
  - A credential in a suggestion is refused.
- The migration: pending becomes live, rejected becomes yanked, approved MCP packages are marked approved.
- Rust: sync uninstalls a revoked item, and backs up an edited copy.

### Phase 5: Bundles

**Server**

- The `AddBundles` migration.
- `PUT`, `GET`, and `DELETE /api/bundles/{ns}/{id}`, owner only:
  - A bundle has 1 to 50 members. Each must be live and visible to whoever saves it, and can come from any namespace.
  - Bundle and package IDs are unique per namespace, checked both ways: `PUT` checks packages, and publish checks bundles.
- The index gains `bundles[]`: `{ id, namespace, bundleId, name, description, publisher, lane, members[], updatedAt }`.
  - Members are filtered per caller, and a bundle with no visible members is left out.
  - A bundle's own visibility uses `IsVisible(ns, bundleId)`, so Share and share links work unchanged.
  - Sharing a bundle does **not** share its members. The Share dialog names the members the person won't see.
- `/api/mine` gains bundles.

**CLI**

- `bundle <ns>/<id>`, `bundle set <ns>/<id> --name … [--description …] <ns/pkg>...`, `bundle delete <ns>/<id>`.
- `install <ns>/<bundle>` installs all members in one batch, with a single `--approve-mcp`.
- `search` lists bundles too.

**Portal**

- `/b/:ns/:id`: member cards, **Install all** (a deep link), Share, and Edit for owners.
- A Bundles section on Browse.
- **New bundle** on My skills: name, space, and a member picker that searches the index.

**Desktop**

- Change `bulk_plan` and `bulk_run` (`application/items.rs:204-371`) to take item IDs across sources, grouped by source. A source's **Install all** passes its own IDs; a bundle passes its members.
- `components/BundleGroup.tsx` above the sources: **Install all**, **Install the rest**, **Uninstall all** (the confirmation lists the members), and member rows built from `ItemCard`.
- `components/BundleDialog.tsx`: name, space, and a searchable list of packages to tick.
- IPC: `run_items`, `save_bundle`, `delete_bundle`.
- Deep links to a bundle open a confirmation that lists the members.

**Docs**

- ADR 0007: the bundles section.
- `marketplace-api.md`, `cli-reference.md`, and `app-reference.md`.
- `publish-to-marketplace.md`: a "Pack or bundle?" table.

**Tests**

- Server:
  - create, update, and delete
  - member validation
  - visibility
  - ID collisions in both directions
  - index filtering
- Rust: a source's Install all behaves as before, and a batch across sources works.

## Release, every phase

1. Run `pg_dump` on the marketplace database before any release with a migration. The database is not backed up.
2. Stop the Windows harness, then run `release.sh` from the home-server repo. It ships agent-plugins HEAD, which must be on `origin/main`.
3. Build the desktop app (NSIS cross-build, MSI in the VM), publish it to `/downloads`, and bump `Client:LatestVersion`.
4. In Phase 4, also bump `Client:MinimumVersion`.

## Left out, and when to add it

| Item                                                  | Add when                                    |
| ----------------------------------------------------- | ------------------------------------------- |
| Publish tokens for CI                                 | A team wants to publish from a pipeline     |
| Link expiry                                           | Forwarded links become a problem            |
| A "via link" marker in share lists                    | Owners ask how someone got access           |
| Auto-installing new bundle members                    | A team wants additions pushed to everyone   |
| Bundle install statistics                             | Publishers ask for them                     |
| Sharing a bundle also shares members you own          | Bundles of private skills become common     |
| Line-by-line diff for suggestions                     | Changed-file markers aren't enough          |
| Reviewing suggestions in the desktop app              | The portal handoff turns out to be friction |
| macOS and Linux deep links (`tauri-plugin-deep-link`) | Mac users need to install from the site     |

## To confirm before starting

- Whoever signs off the corporate rollout accepts removing the admin review queue. ADR 0006 introduced it for that audience.
- Whether the live config sets `Auth:TeamNamespaces` (Phase 0).
