# Find skills at scale: implementation plan

**Status:** Ready to implement<br>
**Date:** 2026-10-10<br>
**Written against:** upstream commit `db77bc5` (version 0.2.2)<br>
**For:** an agent working in the internal (work) fork of this repository

The marketplace will hold hundreds to thousands of skills. Today the desktop app lists every space alphabetically with every card on the page, so a person scrolls past everyone else's skills to reach their own, and search only helps someone who already knows a name. This plan changes the order things appear in, lets sections collapse, makes search ranked and forgiving, and lets each person follow or hide spaces.

Nothing in this plan is implemented upstream. Upstream holds only this document and the measurements in [section 4](#4-measurements).

## 1. Rules for the agent

1. **Find code by symbol, not by line.** Your fork differs from upstream: the portal was rebuilt on the internal UI framework, sign-in uses the internal system, and there are fixes around connectivity and installing. Every reference here names a file and a symbol as they are upstream. [Appendix A](#appendix-a-symbol-map) lists them all. If a symbol is missing in the fork, find the code with the same job and write the mapping in your progress notes before you change anything.
2. **Behavior is the spec; markup is not.** Where this plan shows React or Angular, build the same behavior with the fork's components. The two pure modules, [`feed`](#53-the-feed-module) and [`search`](#63-the-search-module), are specified as functions with test tables. Port those exactly, tests included.
3. **Commit straight to `main`, one phase at a time.** No branches and no pull requests; Jacob decided this. A phase can be several commits, but every commit must leave `main` working, so run the gates before each push. Each phase ends with a **Checkpoint**. Don't start the next phase until it passes.
4. **No user-facing label for the ranking.** No "For you", "Recommended", or "Suggested" anywhere. The order is simply better; Jacob decided this.
5. **No new dependencies.** No fuzzy-search library, no virtual-list library, no state library. Everything here is a few plain functions.
6. **Leave install, sync, and planning code alone.** This work touches how the catalog is presented, one small server table, a few index fields, and the CLI's search order. If you find yourself editing `sync.rs`, `items.rs`, `planner.rs`, or the executor, stop and re-read the phase.
7. **Keep the fork's gates green.** Upstream's are in `AGENTS.md` and `.github/workflows/ci.yml`: typecheck, ESLint with zero warnings, Vitest, Prettier, `cargo fmt`, clippy with warnings denied, `cargo test`, the server build with warnings as errors, the `server/openapi.json` drift check, and the server integration tests.
8. **ASK marks a question for Jacob.** [Section 12](#12-open-questions) gives a default for each one; use it and keep going. The one question that blocks says so.

## 2. Goals and non-goals

Goals:

1. A person who opens the app sees their own spaces first, without scrolling past anyone else's.
2. Sections collapse, and the app remembers which ones.
3. Search works for someone who doesn't know the skill's name: results are ranked, flat, and tolerant of word forms and partial matches.
4. A person can follow a space to keep it on top, or hide one. The choice is stored on the server, so a second PC and the portal agree.
5. The window stays fast at 3,000 skills on a weak PC.

Non-goals are in [section 11](#11-not-doing), each with the condition that would bring it back.

Phases at a glance:

| Phase | What                        | Touches                 | Needs                                 |
| ----: | --------------------------- | ----------------------- | ------------------------------------- |
|     1 | Ranked, collapsible catalog | Desktop window          | Nothing                               |
|     2 | Ranked search               | Desktop window, CLI     | Phase 1, for tie-breaking by tier     |
|     3 | Follow and hide             | Server, desktop, portal | Phase 1                               |
|     4 | Better signals              | Server, desktop         | Phase 1; follower counts need Phase 3 |
|     5 | Portal                      | Portal                  | The phase each part mirrors           |

## 3. What exists today

| Area                 | Today                                                                                                                                                                                                                               | Where (upstream)                                                                       |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Home view            | One group per space, sorted by space name, every card mounted. Bundles sit above in one group.                                                                                                                                      | `project.rs` (`sources.sort_by` on `name`); `App.tsx` (`visibleSources`, JSX list)     |
| What a person sees   | Sync subscribes to every space the catalog lists for them and downloads one archive per space.                                                                                                                                      | `sync.rs` (`catalog_listing`, `subscribed`); server `CatalogService.CatalogAsync`      |
| Customizing          | **Manage sources → Remove** is the only lever, and the next sync adds the space back.                                                                                                                                               | `status.ts` (`reviewSourceRemoval`: "it will be added back on the next check")         |
| Search               | A package matches when every word is a substring of its id, name, description, source name, publisher, or tags. Matches are not ranked; they stay grouped by space in A–Z order.                                                    | `App.tsx` (`matchesQuery`); `lib/marketplace.ts` (`matchesAllWords`)                   |
| Show and Sort        | Show: Everything, Installed, Updates, Official, Teams, Shared with you. "Teams" means any team, not the person's. Sort: By name, Most used (which also reorders the spaces). Both reset on every launch.                            | `CatalogToolbar.tsx` (`CatalogFilters`); `App.tsx` (`matchesFilter`, `visibleSources`) |
| Keyboard             | No shortcut focuses search.                                                                                                                                                                                                         | none                                                                                   |
| Portal browse        | A flat list, 60 per page, with a lane toggle, a team picker, a tag filter, and sorts for popular, new, and updated. Same unranked matcher.                                                                                          | `website/src/app/pages/browse.ts`                                                      |
| Portal home          | Two shelves: Shared with you, and Most used.                                                                                                                                                                                        | `website/src/app/pages/home.ts`                                                        |
| CLI                  | `agent-plugins search` uses the same all-words rule. The official `marketplace` skill tells agents to call it, which is how a plain-language request finds a skill today.                                                           | `cli.rs` (`search`, `matches_all_words`); `marketplace/skills/marketplace/SKILL.md`    |
| Signals already sent | Per package: lane, tags, publish date, installs, active users (`installedBase`), shared-with-you. Per person: their teams, their own space. The server also has each PC's installed list (heartbeats) and every team's member list. | `schemas.ts` (`marketplaceMetaSchema`, `identitySchema`); server `Entities.cs`         |

The same matcher is written three times: the window, the portal, and the CLI.

## 4. Measurements

Taken upstream on 2026-10-10 with a throwaway local marketplace seeded with **993 skills in 300 spaces** (one space with 64 skills, 88 spaces with one, median two). [Appendix B](#appendix-b-reproduce-the-measurements) has the method.

### 4.1 The window

Production build of the unmodified window in headless Chrome 156 at 1280×900, with a mocked Tauri bridge that returns a state of N skills (three detected apps each). "Weak PC" is the same run with the CPU throttled 4×. "First paint" runs from the moment the state arrives until two frames after every card is in the page, so it includes the Zod parse, the React render, style, and layout.

| Skills | Spaces | Cards mounted | First paint, fast / weak PC | Clearing a search, fast / weak PC | DOM nodes | Page height |
| -----: | -----: | ------------: | --------------------------- | --------------------------------- | --------: | ----------- |
|     60 |      1 |            60 | 0.10 s / 0.59 s             | 0.05 s / 0.21 s                   |     1,103 | 9 screens   |
|    250 |     10 |           250 | 0.21 s / 1.16 s             | 0.17 s / 0.80 s                   |     4,414 | 38 screens  |
|  1,000 |    301 |         1,000 | 0.77 s / 3.55 s             | 0.65 s / 2.67 s                   |    19,783 | 193 screens |
|  3,000 |    901 |         3,000 | 1.90 s / 10.37 s            | 1.60 s / 7.99 s                   |    59,183 | 578 screens |

Two more runs approximate this plan's default view at 1,000 skills by leaving all but three spaces empty, so 135 cards are mounted:

| What is on the page                    | First paint, fast / weak PC | Clearing a search, fast / weak PC | DOM nodes |
| -------------------------------------- | --------------------------- | --------------------------------- | --------: |
| 135 cards and 301 group headers        | 0.17 s / 1.00 s             | 0.15 s / 0.67 s                   |     5,078 |
| 135 cards and 33 group headers (paged) | 0.14 s / 0.85 s             | 0.10 s / 0.40 s                   |     2,666 |

Other readings at 1,000 skills: typing a three-letter query settles in 0.24 s (0.65 s weak); a sync result that changed nothing causes no task over 50 ms (69 ms weak), because `withUnchangedItems` keeps unchanged cards; the JS heap is 56 MB (124 MB at 3,000).

### 4.2 Sync and the server

| Measure                                                                             | Result                                                                   |
| ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| First sync of 300 spaces from an empty cache (debug CLI, loopback)                  | 2.3 s                                                                    |
| Repeat sync, nothing changed                                                        | 1.7–2.0 s                                                                |
| Building the cached state (`agent-plugins list`, debug build)                       | 0.7 s                                                                    |
| App data on disk after that sync                                                    | 8.4 MB, 1,907 files                                                      |
| `GET /api/index`, 993 packages, no usage data                                       | 566 KB (45 KB gzipped), 20–90 ms                                         |
| `GET /api/catalog`, 300 spaces                                                      | 113 KB, about 10 ms                                                      |
| `GET /api/index` with 5,000 PCs' heartbeats (15 installed each) and 75,000 installs | about 200 ms per request                                                 |
| The same request answered `304 Not Modified`                                        | also about 200 ms: the document is built before its ETag can be compared |
| The two usage-count queries inside it (`EventsService.CountsAsync`), run in `psql`  | 101 ms (events) + 44 ms (heartbeats)                                     |

### 4.3 What the numbers decide

1. **Sync and index size are fine.** Don't change how the app subscribes to spaces.
2. **The cost is mounting every card.** At 1,000 skills a weak PC waits 3.5 seconds for first paint and the page is 193 screens long. At 3,000 it waits 10 seconds.
3. **Collapsing and paging fix both without a virtual list.** With about 135 cards mounted, first paint is under a second on a weak PC whether 33 or 301 headers are shown.
4. **135 expanded cards is still 20-odd screens.** That is why an expanded group shows its first 12 cards and a **Show all** link ([5.2](#52-behavior)).
5. **"Expand all" would rebuild the slow path**, so there is only **Collapse all**.
6. **Cache the usage counts before adding per-person signals.** 5,000 PCs syncing every 15 minutes is about 5.6 index requests a second, each spending about 145 ms on counts that are the same for everyone ([8.2](#82-server)).

### 4.4 Re-measure at work

These numbers come from Chrome on a fast Linux machine with no network latency. Before Phase 1, and again at each checkpoint, check on a real work PC or VDI session:

- Time from window open to the first card, and whether scrolling stutters, with the production catalog and with a seeded 1,000-skill marketplace if a test environment allows one.
- First sync time over the real network. 300 archive requests run 4–8 at a time (`parallel.rs`), so expect roughly `300 × round-trip ÷ 6`.
- `GET /api/index` p95 from the server's own logs once real heartbeats exist.

If first paint after Phase 1 is still over 1.5 seconds on a weak PC, report it rather than reaching for a virtual list.

## 5. Phase 1: ranked, collapsible catalog

Desktop window only. No Rust, server, or portal change.

### 5.1 Vocabulary

- **Space:** a marketplace namespace. The window calls it a source (`SourceState`); its id is `sourceId`.
- **Group:** one space's section in the catalog (`SourceGroup` upstream).
- **Tier:** why a space sits where it does. Lower tiers come first.

| Tier | A space is in this tier when                                 | Signal (upstream names)                                   |
| ---: | ------------------------------------------------------------ | --------------------------------------------------------- |
|    0 | The person follows it (Phase 3), or it is one of their teams | `identity.follows.followed`, `identity.teams[].namespace` |
|    1 | Something in it was shared with them                         | any `item.marketplace.sharedWithYou`                      |
|    2 | It is the official space                                     | `sourceId === "official"`                                 |
|    3 | It is their own space                                        | `identity.namespace`                                      |
|    4 | They have something installed from it                        | any item whose status counts as installed                 |
|    5 | Everything else                                              |                                                           |

The first matching row wins. Tiers are computed from all of a space's items, never from a filtered subset. A source that isn't a marketplace space (`item.marketplace === null`) can only be tier 4 or 5.

### 5.2 Behavior

**Order of groups.** Always by tier. Tiers 0–4 are ordered by space name within the tier. Tier 5 is ordered by its most-used skill's active users, highest first, then by name. The Sort menu no longer reorders groups.

**Order of cards in a group.** Cards that need attention come first (`updateAvailable`, `partiallyInstalled`, `modified`, `missing`, `removed`). The rest follow the Sort menu: **Most used** (active users, then name) or **By name**. The default becomes **Most used**.

**Collapsing.**

- Every group and the Bundles section can be collapsed from its header.
- A group starts expanded when it is tier 0. If the person has no tier 0 space, the first group in the order starts expanded, so a new person sees skills, not a list of headers. Every other group starts collapsed. Bundles start expanded.
- Once a person toggles a group, their choice wins over the default and is remembered on that PC.
- A collapsed group mounts no cards. This is the performance fix; don't hide cards with CSS.
- The header always shows the space's name, its freshness badge, and a count line such as `34 skills · 3 installed`. It adds an amber badge for updates (`1 update`) and a blue one for shares (`2 shared with you`) when there are any, so a collapsed group still says when it needs a look. A group never expands on its own.
- The space's description and its whole-space buttons (**Share…**, **Install all**, **Replace all**, **Uninstall all**) show only while the group is expanded.
- A **Collapse all** button sits beside the Show and Sort menus. It collapses every group on the page and saves that. There is no "Expand all".

**Long groups.** An expanded group shows its first 12 cards in order, then **Show all N**. Showing all is not remembered.

**The long tail.** Tier 5 shows its first 30 groups, then **Show more spaces (N more)**, 30 at a time. Not remembered.

**Bundles.** The section lists bundles from tier 0–4 spaces and any bundle shared with the person, ordered by tier and then name, followed by **Show all N bundles** when there are others.

**Filters.**

- Show = Everything, Official, or Teams: grouped as above, restricted to that lane.
- Show = Installed, Updates, or Shared with you, and the "changed on this computer" filter: every group with a match is forced open with all its matches, and the tail is not paged. These sets are small.
- While a search query is typed: until Phase 2 lands, treat it like the previous bullet.
- Whole-space buttons stay hidden while any filter or query is active, as today (`filtering`).

**Links and notices that scroll to a card.** `reveal(id)` must now also set Show to Everything and make the target reachable: its group is rendered even if it is beyond the paged tail or hidden (Phase 3), expanded even if collapsed, and showing all its cards. Hold this as one transient value (the space being revealed) that lasts until the next reveal or filter change. Don't write it to the saved preferences. A link to a bundle forces the Bundles section open and unpaged the same way.

**Remembered on this PC.** One `localStorage` entry, `agent-plugins.catalog.v1`:

```json
{ "show": "all", "sort": "used", "groups": { "data-team": false, "official": true }, "bundles": true }
```

`groups` holds only spaces the person toggled, keyed by `sourceId`. Parse it with Zod and fall back to defaults on any error, including `localStorage` throwing. Collapse state is per PC on purpose; follows (Phase 3) are what roam.

### 5.3 The `feed` module

New file `src/lib/feed.ts` with `src/lib/feed.test.ts`. Pure functions, no React, no storage.

```ts
export type SpaceTier = 0 | 1 | 2 | 3 | 4 | 5;
export type Follows = Readonly<{ followed: readonly string[]; hidden: readonly string[] }>;
export type RankedSpace = Readonly<{ source: SourceState; tier: SpaceTier }>;
export type GroupSummary = Readonly<{ total: number; installed: number; updates: number; shared: number }>;

/** Why a space sits where it does. `items` are all of the space's items, unfiltered. */
export function spaceTier(source: SourceState, items: readonly CatalogItem[], identity: AppIdentity | null, follows: Follows | null): SpaceTier;

/** Every space in page order. Hidden spaces (Phase 3) are left out. */
export function rankSpaces(sources: readonly SourceState[], itemsBySource: ReadonlyMap<string, readonly CatalogItem[]>, identity: AppIdentity | null, follows: Follows | null): readonly RankedSpace[];

/** The spaces that start expanded: tier 0, or the first space when there is no tier 0. */
export function defaultExpanded(ranked: readonly RankedSpace[]): ReadonlySet<string>;

/** Card order inside a group: needs attention first, then the Sort menu's choice. */
export function compareCards(sort: "used" | "name"): (left: CatalogItem, right: CatalogItem) => number;

/** The numbers a group's header shows. */
export function groupSummary(items: readonly CatalogItem[]): GroupSummary;
```

`lib/marketplace.ts` already has a private `isInstalled(status)`; export it and use it for tier 4 and for `installed`. `updates` counts `updateAvailable` and `partiallyInstalled`. Until Phase 3, pass `null` for `follows`.

Tests, at minimum:

| Case                                                                                                           | Expect                                               |
| -------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| Spaces `zeta` (a team of mine), `official`, `me` (my space), `alpha` (nothing of mine), `beta` (one installed) | `zeta`, `official`, `me`, `beta`, `alpha`            |
| `alpha` holds a skill shared with me                                                                           | `alpha` moves to second                              |
| Two tier 5 spaces, most-used skill at 3 and 40 active users                                                    | The 40 comes first; equal counts fall back to name   |
| A space is both my team and has a share                                                                        | Tier 0                                               |
| `identity` is `null` (signed out or offline with no saved identity)                                            | Official first, installed-from next, the rest by use |
| No tier 0 space                                                                                                | `defaultExpanded` is the first ranked space only     |
| Cards: one `updateAvailable` with 0 users, one `available` with 50                                             | The update comes first under both sorts              |
| A non-marketplace source with an installed item                                                                | Tier 4                                               |

### 5.4 Window changes

Upstream file by file; map to the fork's equivalents.

- **`src/App.tsx`**
  - Keep the unfiltered `itemsBySource` for tiers and headers, and derive the filtered one from it.
  - Replace `visibleSources` with `rankSpaces(...)`, then apply the filter and tail-paging rules.
  - Load and save the preferences entry. `catalogFilter` and `catalogSort` start from it.
  - `reveal`: add the Show reset and the transient revealed space.
  - Pass each group whether it is expanded, its `GroupSummary`, and a toggle callback. Keep callbacks stable (`useStableCallback`) so memoized cards don't re-render.
- **`src/components/SourceGroup.tsx`**: make the header a native `<details>`/`<summary>` and render the body only while open. `Bundles.tsx` already does exactly this for "Show its skills"; copy that pattern. Buttons must sit outside `<summary>`. The section keeps its DOM id (`cardDomId(source.sourceId)`), because links scroll to it.
- **`src/components/Bundles.tsx`** (`BundleGroup`): the same collapse, plus the ordering and **Show all N bundles**.
- **`src/components/CatalogToolbar.tsx`**: the **Collapse all** button.
- **`src/App.css`**: header styles for the collapsed state.

In a different UI framework, the collapse control must be operable from the keyboard, expose its expanded state to assistive technology, and not create its content while collapsed.

### 5.5 Docs

Update `docs/app-reference.md`: the "Show and sort menus" row, a new "Groups" subsection covering order, collapse, **Show all**, and **Show more spaces**, and the `agent-plugins://open/...` row (it now expands the group). Update `docs/codebase-map.md` with `lib/feed.ts`.

### 5.6 Checkpoint

- The gates pass.
- With a signed-in person who is in one team: that team's group is first and expanded, Official is next and collapsed, and their own space follows.
- Collapse a group, restart the app: it is still collapsed.
- An `agent-plugins://open/<namespace>/<id>` link to a skill in a collapsed tier 5 space beyond the first 30 scrolls to the card.
- Show → Installed lists every installed skill without expanding anything by hand.
- On the seeded or production catalog, a weak PC reaches first paint in under 1.5 seconds. Record the number.

## 6. Phase 2: search

Desktop window and CLI. No server change.

### 6.1 Behavior in the window

While the search box holds text, the grouped view is replaced by flat results in this order:

1. **Spaces** (at most five): spaces whose name, id, or description contains every word. Each row shows the name and `N skills`; choosing it clears the query and reveals that group.
2. **Bundles** that match, as today.
3. **Skills**: ranked cards, 60 at a time with **Show more**. Each card already names its publisher, so a flat list stays readable.
4. **Close matches**: shown under that heading when the query has two or more words and fewer than five skills matched all of them.

Other rules:

- Search always covers everything the person can see. Collapsed, paged, and hidden spaces never hide a result.
- The Show menu still narrows what is searched.
- The count beside the box reads `12 matches`, or `1 match and 6 close`.
- With no match and no close match, keep today's empty state (`noMatchesText`).
- Clicking a tag on a card puts that tag in the search box. Make the tag a real button.
- **Ctrl+F** and **Ctrl+K** focus the search box and select its text. Call `preventDefault()`, because WebView2 otherwise opens its own find bar on Ctrl+F. **Esc** in the box clears it. Ignore the shortcut while a dialog is open.

### 6.2 Ranking rules

**Normalize** every field and the query the same way: Unicode NFKD, drop combining marks, lowercase, turn every run of characters that isn't a letter or digit into one space, trim. `pr-review`, `PR_Review`, and `pr review` are then the same text.

**Words** are the normalized query split on spaces, at most eight.

**Stem.** A word may also match by its stem: remove the first of `ing`, `es`, `ed`, `s` that the word ends with and that leaves at least four characters. `reviewing` gives `review`; `notes` gives `note`; `string` has no stem. This is a deliberately naive rule with a known ceiling. Mark it with a `ponytail:` comment, as the codebase does for such shortcuts, naming the upgrade path (a real stemmer or server-side search).

**Points for one word against one skill** are the best of these, trying the word and its stem:

| Found                                                     | Points |
| --------------------------------------------------------- | -----: |
| Equals a word of the name or of the local id              |     10 |
| Starts a word of the name or of the local id              |      8 |
| Equals a tag                                              |      6 |
| Inside the name or the local id                           |      5 |
| Inside a tag                                              |      3 |
| Inside the space text (source name, publisher, namespace) |      3 |
| Inside the description                                    |      1 |
| Nowhere                                                   |      0 |

**A skill's score** is the sum over the words, plus 20 when the whole normalized query equals the normalized name or local id. A skill is a match only when every word scores above 0.

**Order of matches:** score, highest first; then the space's tier, lowest first; then active users, highest first; then name.

**Close matches:** skills that matched at least half the words (rounded up) but not all. Order by words matched, then as above. At most 20.

### 6.3 The `search` module

New file `src/lib/search.ts` with `src/lib/search.test.ts`. Pure.

```ts
/** One skill with its fields already normalized: name, local id, tags, space text, description, plus its space's tier. */
export type SearchDoc = Readonly<{ item: CatalogItem; name: string; localId: string; tags: readonly string[]; space: string; description: string; tier: SpaceTier }>;
export type SearchResults = Readonly<{ matches: readonly CatalogItem[]; close: readonly CatalogItem[] }>;

export function normalize(text: string): string;
export function stem(word: string): string | null;
/** Built once per state, not per keystroke. */
export function searchDocs(items: readonly CatalogItem[], tierOf: (sourceId: string) => SpaceTier): readonly SearchDoc[];
export function searchItems(docs: readonly SearchDoc[], query: string): SearchResults;
```

Build `searchDocs` in a `useMemo` keyed on the items. Scoring 3,000 docs per keystroke is then a few milliseconds.

**Test table.** Use these six skills in every implementation's tests (window, CLI, portal), with every space in the same tier unless a row says otherwise:

| Key | Name             | Id                          | Tags              | Publisher | Description                                        | Active users |
| --- | ---------------- | --------------------------- | ----------------- | --------- | -------------------------------------------------- | -----------: |
| A   | PR Review        | `data-team/pr-review`       | review, git       | Data Team | Reviews a pull request for style and bugs.         |           40 |
| B   | Review Checklist | `official/review-checklist` | review            | Official  | A checklist for reviewing code.                    |           90 |
| C   | SQL Style        | `data-team/sql-style`       | sql               | Data Team | Formats SQL and flags risky queries before review. |           10 |
| D   | Standup Notes    | `jane/standup-notes`        | meetings          | Jane Doe  | Turns rough notes into a standup update.           |            5 |
| E   | Meeting Summary  | `ops/meeting-summary`       | meetings, outlook | Ops       | Summarizes meetings from a transcript.             |           20 |
| F   | Reviewer         | `bob/reviewer`              | (none)            | Bob       | Does things.                                       |            1 |

| Query                                                 | Matches, in order | Close, in order | Why                                                                          |
| ----------------------------------------------------- | ----------------- | --------------- | ---------------------------------------------------------------------------- |
| `review`                                              | B, A, F, C        |                 | A and B score 10 and B has more users; F starts a word (8); C by description |
| `pr review`                                           | A                 | B, F, C         | A matches both words and its name exactly                                    |
| `pr-review`                                           | A                 | B, F, C         | Normalizes to the query above                                                |
| `PR_Review`                                           | A                 | B, F, C         | Same                                                                         |
| `reviewing`                                           | B, A, F, C        |                 | The stem `review` scores like the word                                       |
| `meetings`                                            | E, D              |                 | The stem `meeting` equals a word of E's name (10); D by tag (6)              |
| `sql`                                                 | C                 |                 |                                                                              |
| `standup update`                                      | D                 |                 | Name word plus description                                                   |
| `meeting notes`                                       | D                 | E               | E has no `notes` or `note`                                                   |
| `data team`                                           | A, C              |                 | Space text only; A has more users                                            |
| `xyz`                                                 |                   |                 | One word never produces close matches                                        |
| `review`, with `data-team` tier 0 and the rest tier 5 | A, B, F, C        |                 | Equal scores break toward the person's own spaces                            |

### 6.4 Window changes

- **`src/App.tsx`**: when the deferred query is non-empty, render the flat results instead of the groups. `matchCount` and `noMatches` come from `SearchResults`. Remove `matchesQuery`. Add the keyboard listener.
- **`src/components/ItemCard.tsx`**: tags become buttons that call a stable `onTag` from `CardExtrasContext`.
- **`src/lib/marketplace.ts`**: `matchesAllWords` stays for bundles and space rows, but normalize both sides with `normalize`.

### 6.5 CLI

`agent-plugins search` (`cli.rs`, `search` and `matches_all_words`) gets the same normalization, stem, points, and order. It has no tiers, so ties go to active users and then id. Close matches print after the matches under a `close matches:` line, and `--json` gains a `close` array beside `packages` and `bundles`. Port the test table as a Rust test.

Then update `docs/cli-reference.md` (`search`) and the Search section of `marketplace/skills/marketplace/SKILL.md`, which still says every word must appear. That skill is published in the `official` space, so publish the new version at work once the CLI ships. Agents send multi-word, plain-language queries, so close matches matter most here.

### 6.6 Docs

`docs/app-reference.md`: the "Search box" row and a "Search results" subsection with the rules in 6.1 and a plain-words version of 6.2.

### 6.7 Checkpoint

- The gates pass, including the test table in TypeScript and in Rust.
- Typing `reviewing` finds a skill named "PR Review".
- Typing two words where only one exists anywhere shows **Close matches**.
- Ctrl+F focuses the box and no browser find bar appears on Windows.
- A skill in a collapsed, paged tier 5 space is found by search.

## 7. Phase 3: follow and hide

Server, desktop, and portal. Deploy the server before releasing the desktop build ([section 10](#10-compatibility-and-rollout)).

### 7.1 Behavior

- **Follow** a space: it becomes tier 0, so it sits with the person's teams and starts expanded.
- **Hide** a space: it leaves the grouped catalog and the Bundles section. It is still searched, its installed skills still appear under Show → Installed and Updates, and a link to something in it still opens. A line at the bottom of the catalog, `3 hidden spaces`, opens a list with **Show again** on each.
- Any space can be followed or hidden, including Official, a team the person is in, and their own. Hide wins over team membership.
- Controls live in the group header, visible whether or not the group is expanded: **Follow** (which reads **Following** once set, and clears on a second click) and **Hide**. A team the person belongs to shows only **Hide**. Space rows in search results show **Follow** too.
- A change shows at once and is confirmed by the next sync. When the marketplace can't be reached, the change is undone and the usual offline or error notice appears.
- When the server doesn't report follows (an older server, or signed out), the controls don't render.

### 7.2 Server

**Entity** (`Data/Entities.cs`, `MarketplaceDbContext`):

```csharp
public sealed class Follow
{
    public required string Account { get; set; }      // lower-cased; see below
    public required string Namespace { get; set; }
    public FollowState State { get; set; }             // Followed or Hidden, stored as a string
    public DateTime UpdatedAt { get; set; }
}
```

Key `(Account, Namespace)`, an index on `Namespace`, `Account` max 256, `Namespace` max 16, `State` with `HasConversion<string>()` like the other enums. Add the migration with the repo's `dotnet ef` tool (`dotnet tool restore`, then `dotnet ef migrations add AddFollows --project server/src/Marketplace.Api`).

**The account key.** Upstream compares accounts case-insensitively (`Account.ToLower() == identity.Account.ToLower()` in the heartbeat and event queries). Store `identity.Account` lower-cased and look it up lower-cased. **Your fork's sign-in differs:** use whatever your fork treats as the stable key for one person, the same value it stores on `Heartbeat.Account`, so [8.2](#82-server) can join the two.

**Endpoints**, in the authenticated group of `MarketplaceEndpoints.cs`:

| Request                                                                     | Answer                                                                                                                                                                                                                            |
| --------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `PUT /api/follows/{namespace}` with `{ "state": "followed" }` or `"hidden"` | `204`. `404` when the space doesn't exist or the caller can see nothing in it and isn't a member (one answer for both, so a private space's existence doesn't leak). `422` for any other state, or past 200 rows for the account. |
| `DELETE /api/follows/{namespace}`                                           | `204`, whether or not a row existed.                                                                                                                                                                                              |
| `GET /api/me`                                                               | Gains `"follows": { "followed": ["data-team"], "hidden": ["noisy"] }`, each list sorted.                                                                                                                                          |

"Can see something in it" is the test `CatalogService.CatalogAsync` already applies per space: at least one live package passes `AccessService.IsVisible`. Membership is `identity.Namespaces.Contains(ns)`.

Two existing filters need a look:

- **Blocked accounts** may not change anything, apart from sending usage reports (`POST /api/events`) and `/api/notifications/read`. Follows are a personal preference, so exempt `/api/follows/` the same way.
- **CI pipelines** (`IsMachine`) may only publish; the existing filter already refuses everything else, which is right here.

**Cleanup.** `TeamService.DeleteAsync` deletes a team's rows from several tables; add `Follows` for that namespace.

**Tests.** One integration test in the style of `SelfServiceTests.cs` (`factory.ClientFor("TEST\\name")`), named for the behavior, covering: follow a public space and see it in `/api/me`; another person's `/api/me` is unaffected; hide replaces follow; delete clears it; a private space the caller can't see answers `404`, as does a made-up one; deleting the team removes the rows; a blocked account can still follow.

**Also:** regenerate `server/openapi.json` the way CI's drift check does, and add a "Follows" section and the new `/api/me` field to `docs/marketplace-api.md`.

### 7.3 Desktop

- **`src-tauri/src/marketplace.rs`**: a `Follows { followed: Vec<String>, hidden: Vec<String> }` struct, and on `Me` an `Option<Follows>` with `#[serde(default)]`. `None` means the server didn't say.
- **`src-tauri/src/app_state.rs`**: the same optional field on `MarketplaceIdentity`. `project.rs` copies it across where it builds the identity. The identity is already cached to disk (`write_identity_cache`), so the order survives being offline.
- **`src-tauri/src/ipc.rs`**: one command beside the team commands, using the existing `call` and `space` helpers.

  ```rust
  #[tauri::command]
  pub(crate) async fn set_follow(namespace: String, state: Option<String>) -> Result<(), IpcError> {
      let path = format!("follows/{}", space(&namespace)?);
      match state {
          Some(state) => call(Method::PUT, path, Some(json!({ "state": state }))).await.map(drop),
          None => call(Method::DELETE, path, None).await.map(drop),
      }
  }
  ```

  Register it in `generate_handler!` (`src-tauri/src/lib.rs`) and add `"set_follow"` to `IpcCommand` (`src/ipc/client.ts`).

- **`src/ipc/schemas.ts`**: `identitySchema` gains `follows`, an object of two string arrays, `.nullable().default(null)`.
- **Fixture**: add follows to the Rust fixture state, then regenerate `src/ipc/fixtures/app-state.json` (`UPDATE_FIXTURES=1 cargo test --manifest-path src-tauri/Cargo.toml app_state_fixture`, then Prettier).
- **Window**: pass `identity.follows` to `rankSpaces` and `spaceTier` (they already take it). Add the header controls and the hidden-spaces line. On a click, update `state.identity.follows` optimistically, call `set_follow`, then `synchronize()`; on failure put the old value back and show the error through the existing path.
- **Tests**: extend `feed.test.ts`: a followed tier 5 space becomes tier 0; a hidden team is left out of `rankSpaces`; `follows: null` behaves like empty lists.

No CLI command for follows.

### 7.4 Portal

- `meSchema` gains `follows`. Note that upstream's `meSchema` and `indexPackageSchema` are `z.strictObject`, so the portal must ship in the same release as the server change.
- `Api.setFollow(namespace, state | null)`.
- **Follow** and **Hide** on the team page and on the Browse header shown for one space (`?space=`).
- Browse leaves hidden spaces out of its default list. A search, the Installed filter, or `?space=` for that space still shows them.

### 7.5 Docs

`docs/app-reference.md` (a "Follow and hide" subsection) and `docs/marketplace-api.md`. If your fork keeps a list of database migrations or deployment steps, add the `AddFollows` migration to it.

### 7.6 Checkpoint

- The gates pass, including the new server test and the OpenAPI drift check.
- Follow a tier 5 space on one PC: it moves to the top there, and after a sync on a second PC and a reload of the portal it is on top there too.
- Hide a team: it leaves the catalog, its installed skills still appear under Installed, and **Show again** brings it back.
- Point the new desktop build at a server without Phase 3: no Follow or Hide controls, no errors.

## 8. Phase 4: better signals

Server and desktop. Small, and independent of Phase 3 except for follower counts.

### 8.1 New fields

| Field                             | Meaning                                                                                          | Used for                                                                 |
| --------------------------------- | ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------ |
| `teamUsers` on each index package | How many other members of the caller's teams have it installed, by heartbeat in the last 30 days | Card order within a group, search ties, tier 5 order, a line on the card |
| `recentInstalls` on each package  | Distinct accounts that installed it in the last 30 days                                          | The portal's "Most used" shelf, so it shows what is rising               |
| `createdAt` passed to the desktop | When the package was first published. The index already sends it; the desktop drops it.          | "New" on cards                                                           |
| `followers` per space             | How many people follow it (Phase 3)                                                              | Tier 5 order                                                             |

### 8.2 Server

- **`IndexPackage`** (`CatalogService.cs`): append `int TeamUsers` and `int RecentInstalls`. **`IndexDocument`**: append `IReadOnlyDictionary<string, int> Followers`, holding only spaces with at least one.
- **`EventsService.CountsAsync`**: also return recent installs, from the same `Events` rows filtered to the last 30 days (`AppWindow`).
- **Team users.** Collect the member entries of the caller's teams (`TeamMembers` where the namespace is one of `identity.Teams`). Team entries can be written several ways (`CORP\jane`, `jane@corp`, `jane`), so match them to heartbeat accounts with `IdentityResolver.EntryMatches`, the way membership itself is decided, not with string equality. In your fork, use whatever function decides team membership. Leave the caller out. Count distinct matched accounts per installed package.
- **Cache the usage data.** Register `IMemoryCache` and keep, for a configurable `Events:CountsCacheSeconds` (default 60), both the `CountsAsync` result and the list of `(Account, Installed)` heartbeat rows inside the window. Both are the same for every caller; only the team filter is per person. This removes roughly 145 ms of the 200 ms measured in [4.2](#42-sync-and-the-server). Don't invalidate on write: heartbeats arrive as often as index requests, so that would empty the cache constantly. Set the setting to 0 in the integration tests, which assert counts right after posting events.
- **Publish warning.** Where `PublishService` builds `Warnings`, add one when the package has no tags: `No tags. Add a few topic words so people can find this by topic.` It appears in dry runs and publishes.
- **Tests**: `teamUsers` counts a teammate's install and not the caller's or a stranger's; an entry written as a bare username matches; `recentInstalls` ignores an install older than 30 days; the tag warning appears. Regenerate `openapi.json`; update `docs/marketplace-api.md` (`GET /api/index`).

### 8.3 Desktop

- **Rust**: `IndexPackage` and `MarketplaceMeta` gain `team_users`, `recent_installs`, and `created_at`, all `#[serde(default)]`; `MarketplaceMeta::from_index` copies them. `Index` gains `followers`; `project.rs` puts a `followers` count on each `SourceState`. Regenerate the fixture.
- **Schemas**: matching fields with defaults (`0`, `0`, `null`, `0`).
- **`compareCards("used")`**: team users, then active users, then name.
- **New.** A skill is new when it was created in the last 14 days, isn't installed, and its space is tier 0 or 1. New skills sort after the ones needing attention and carry a **New** badge. This is how a team's latest skill reaches the top of its group without having users yet.
- **Tier 5 order**: the space's highest `teamUsers`, then `followers`, then its most-used skill's active users, then name.
- **Search ties**: team users before active users.
- **Card line**: when `teamUsers` is above 0, add `3 on your teams` to the line that already shows installs and active users.
- **Tests**: extend `feed.test.ts` and the search table's tie cases.

### 8.4 Checkpoint

- The gates pass.
- Two test accounts in one team: one installs a tier 5 skill and syncs; within the cache window the other sees `1 on your teams` on it, and it leads its group.
- `GET /api/index` p95 in the server's logs is lower than before the phase. Record both numbers.

## 9. Phase 5: portal

The portal was rebuilt in your fork, so this phase is behavior only.

1. **Search**: port `normalize`, `stem`, the points, and the order from [6.2](#62-ranking-rules), with the test table. Show close matches under their own heading.
2. **Browse order**: the default sort keeps its "Most used" label but orders by the caller's own spaces first (tier 0 and 1), then team users, then active users. Hidden spaces are left out as in [7.4](#74-portal).
3. **Home shelves**: keep "Shared with you". Add "From your teams" (skills from tier 0 spaces, newest first, at most six, omitted when empty). "Most used" orders by `recentInstalls`, then active users.
4. **Follow and Hide**: from [7.4](#74-portal), if not already shipped with Phase 3.

Tiers need the caller's teams and follows (`/api/me`) and each package's namespace, all of which the portal already loads.

**Checkpoint:** the fork's portal gates pass; a member of a team sees that team's skills first on Browse and in "From your teams"; the search table passes.

## 10. Compatibility and rollout

| Combination             | Result                                                                                                                                             |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| Old desktop, new server | Works. The Rust structs ignore unknown JSON fields (confirm your fork has no `deny_unknown_fields` on `Me` or the index types).                    |
| New desktop, old server | Works. Follows are `None`, so the controls don't render; new counts default to 0.                                                                  |
| Portal and server       | Ship together. Strict schemas reject unknown fields, so a portal tab left open across a deploy can fail its next `/api/me` until it is reloaded.   |
| Phases 1 and 2          | Desktop only; release whenever they pass.                                                                                                          |
| Phase 3                 | Run the migration and deploy the server and portal first, then release the desktop build.                                                          |
| Phase 4                 | Server first. The index's bytes change more often once `teamUsers` is in it, so desktops re-download it more; it is 45 KB gzipped at 1,000 skills. |

Nothing here needs a minimum client version bump.

## 11. Not doing

| Left out                                        | Why                                                                                                                           | Revisit when                                                             |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| A virtual list                                  | Collapsing and paging bound the mounted cards; measured in [4.1](#41-the-window)                                              | First paint stays over 1.5 s on a weak PC after Phase 1                  |
| Syncing only followed spaces                    | 300 spaces sync in about 2 seconds; changing subscription touches install and removal paths                                   | First sync over the real network takes more than about 20 seconds        |
| Per-skill favorites                             | Installing a skill is the favorite; follows cover "the spaces I care about"                                                   | People ask to bookmark without installing                                |
| A fuzzy or typo-tolerant library, or embeddings | Stems, partial matches, and normalization cover word forms; the agent-side `marketplace` skill covers plain-language requests | People report misses that are spelling mistakes                          |
| Server-side search                              | The window must search offline and instantly; every client already holds the index                                            | The index grows past what clients should download (tens of thousands)    |
| A category taxonomy                             | Tags exist and are searchable                                                                                                 | Tags fragment so far that topic browsing fails                           |
| Tags for skills published from CI               | Upstream's repository publish has no way to set tags; they come only from the portal and the CLI                              | Teams that publish from CI want tags; read them from `SKILL.md` metadata |
| A CLI command for follows                       | Nobody asked                                                                                                                  | Someone scripts their setup                                              |
| "Expand all"                                    | It mounts every card, which is the slow path this plan removes                                                                | Never, unless a virtual list lands                                       |
| Recording what people search for                | It would show vocabulary gaps, but it logs what people type                                                                   | Jacob decides the privacy trade is worth it                              |

## 12. Open questions

None of these block. Use the default, say so in the commit message, and Jacob can change it afterward.

| Question (ASK)                                               | Default in this plan                   |
| ------------------------------------------------------------ | -------------------------------------- |
| How many cards an expanded group shows before **Show all**   | 12                                     |
| How many tier 5 groups show before **Show more spaces**      | 30                                     |
| Should the Show menu's choice be remembered across launches? | Yes, with Sort and the collapse state  |
| Wording of the controls                                      | Follow / Following / Hide / Show again |
| How long a skill counts as new                               | 14 days                                |
| May a person hide Official?                                  | Yes                                    |
| The usage cache's lifetime                                   | 60 seconds                             |

One question does block Phase 3 if the answer isn't obvious in your fork: **which value is the stable key for one person** across the follows table, heartbeats, and team entries ([7.2](#72-server)). If the fork's sign-in can present the same person under two spellings and there is no existing function that reconciles them, ASK before writing the migration.

## Appendix A: symbol map

Upstream locations at `db77bc5`.

**Desktop window**

| File                                | Symbols                                                                                                                                                                                                                                                            |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `src/App.tsx`                       | `App`; state `query`, `deferredQuery`, `catalogFilter`, `catalogSort`, `driftOnly`, `revealing`; memos `itemsBySource`, `visibleBundles`, `visibleSources`; `filtering`; `reveal`; `matchesQuery`; `matchesFilter`; `usersOf`; `withUnchangedItems`; `synchronize` |
| `src/components/SourceGroup.tsx`    | `SourceGroup`, `FreshnessBadge`                                                                                                                                                                                                                                    |
| `src/components/Bundles.tsx`        | `BundleGroup` (its `<details>` with content rendered only while open is the collapse pattern to copy)                                                                                                                                                              |
| `src/components/ItemCard.tsx`       | `ItemCard` (memoized), the tag badges, `MarketplaceBadges`, `CardExtrasContext`                                                                                                                                                                                    |
| `src/components/CatalogToolbar.tsx` | `CatalogToolbar`, `CatalogFilters`                                                                                                                                                                                                                                 |
| `src/lib/marketplace.ts`            | `matchesAllWords`, `isInstalled` (private), `spaceLabel`, `ownsSpace`, `cardDomId`, `bundleSummary`                                                                                                                                                                |
| `src/lib/connectivity.ts`           | `noMatchesText`, `catalogBody`                                                                                                                                                                                                                                     |
| `src/lib/status.ts`                 | `reviewSourceRemoval`, whose prompt says a listed source comes back                                                                                                                                                                                                |
| `src/ipc/schemas.ts`                | `identitySchema`, `marketplaceMetaSchema`, `itemSchema`, `sourceSchema`, `appStateSchema`                                                                                                                                                                          |
| `src/ipc/client.ts`                 | `IpcCommand`, `invokeParsed`                                                                                                                                                                                                                                       |
| `src/ipc/fixtures/app-state.json`   | Generated by the Rust test `app_state_fixture_matches_the_checked_in_contract`                                                                                                                                                                                     |

**Desktop Rust**

| File                                           | Symbols                                                                                           |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `src-tauri/src/marketplace.rs`                 | `Me`, `TeamMembership`, `IndexPackage`, `Index`, `fetch_me`, `api`, `api_json`, `namespace_path`  |
| `src-tauri/src/app_state.rs`                   | `MarketplaceIdentity`, `MarketplaceMeta::from_index`, `SourceState`, `AppState`, the fixture test |
| `src-tauri/src/application/project.rs`         | Builds `AppState`; sorts `sources` by name; builds `MarketplaceIdentity`; `write_identity_cache`  |
| `src-tauri/src/ipc.rs`, `src-tauri/src/lib.rs` | `call`, `space`, the team commands; `generate_handler!`                                           |
| `src-tauri/src/cli.rs`                         | `search`, `matches_all_words`                                                                     |
| `src-tauri/src/parallel.rs`                    | `map`, 4–8 workers                                                                                |

**Server** (under `server/src/Marketplace.Api/`)

| File                                  | Symbols                                                                                                      |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `Endpoints/MarketplaceEndpoints.cs`   | `/me` and `MeView`; `/index`; `Conditional`; the endpoint filter that refuses blocked accounts and pipelines |
| `Catalog/CatalogService.cs`           | `IndexPackage`, `IndexDocument`, `IndexAsync`, `CatalogAsync`                                                |
| `Events/EventsService.cs`             | `CountsAsync`, `AppWindow`, `AppAsync`                                                                       |
| `Access/AccessService.cs`             | `IsVisible`, `LivePackagesAsync`, `RulesAsync`                                                               |
| `Auth/MarketplaceIdentity.cs`         | `MarketplaceIdentity` (`Teams`, `Namespaces`, `IsMachine`, `IsBlocked`), `IdentityResolver.EntryMatches`     |
| `Teams/TeamService.cs`                | `DeleteAsync`                                                                                                |
| `Packages/PublishService.cs`          | `Warnings` on `PublishedVersion` and `DryRunView`                                                            |
| `Data/`                               | `Entities.cs`, `MarketplaceDbContext.cs`, `Migrations/`                                                      |
| `server/tests/Marketplace.Api.Tests/` | One partial class, `MarketplaceApiTests`; `factory.ClientFor("TEST\\name")`                                  |
| `server/openapi.json`                 | Checked in; CI regenerates it and fails on a difference                                                      |

**Portal** (under `website/src/app/`; rebuilt in the fork)

| File              | Symbols                                                |
| ----------------- | ------------------------------------------------------ |
| `api.ts`          | `meSchema`, `indexPackageSchema` (both strict), `Api`  |
| `format.ts`       | `parseBrowse`, `browseParams`, `BrowseSort`            |
| `pages/browse.ts` | `BrowsePage.matches`, `order`, `pageSize`              |
| `pages/home.ts`   | The `shared` and `popular` shelves                     |
| `pages/team.ts`   | The team page                                          |
| `session.ts`      | `Session.me`, `refreshMe` (runs on every window focus) |

**Docs to keep current:** `docs/app-reference.md`, `docs/marketplace-api.md`, `docs/cli-reference.md`, `docs/codebase-map.md`, `marketplace/skills/marketplace/SKILL.md`.

## Appendix B: reproduce the measurements

Everything ran against a local stack; none of it is checked in.

1. **Stack.** PostgreSQL in Docker on a loopback port. The API with `dotnet run --project server/src/Marketplace.Api --no-launch-profile` in Development mode, with `Kestrel__Endpoints__Http__Url`, `ConnectionStrings__Marketplace`, `Server__PublicBaseUrl`, `ArtifactKeeper__BaseUrl`, and `Validator__Path` (a copy of `validate-source`) set in the environment. The package store was a 50-line in-memory stand-in that answers login, `PUT`, `GET`, and `DELETE`. Never run Development mode anywhere reachable: it trusts `X-Dev-User`.
2. **Seed.** 300 dev users each sent one `POST /api/namespaces/{user}/publish` with a zip of `skills/<name>/SKILL.md` files. Sizes followed `1 / rank^0.7`, scaled to 1,000 in total. Seeding took 107 seconds.
3. **Usage.** 5,000 heartbeats were inserted with the SQL below, and 75,000 install events with the same pattern into `Events`:

   ```sql
   WITH ids AS (SELECT array_agg("Namespace" || '/' || "PackageId") AS all_ids FROM "Packages")
   INSERT INTO "Heartbeats" ("Account","Device","OccurredAt","ClientVersion","OsBuild","Agents","Installed","ChecksJson","InstalledVersionsJson","ReceivedAt")
   SELECT 'CORP\user' || g, 'pc', now(), '0.2.2', '10.0', ARRAY['cursor'],
          (SELECT array_agg(x) FROM (SELECT x FROM ids, unnest(all_ids) x ORDER BY random() + g * 0 LIMIT 15) picked),
          '[]'::jsonb, '{}'::jsonb, now()
   FROM generate_series(1, 5000) g;
   ```

4. **Sync.** The debug CLI with `AGENT_PLUGINS_QA_ROOT` (an empty folder under the temp directory), `AGENT_PLUGINS_MARKETPLACE_URL`, and `AGENT_PLUGINS_DEV_USER`, timing `agent-plugins sync` and `agent-plugins list`.
5. **Window.** `vite build`, served with `vite preview` on loopback. Headless Chrome was driven over the DevTools protocol from a Node script that:
   - installs a mocked bridge before the page loads, so the real window runs unchanged:

     ```js
     window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
     window.__TAURI_INTERNALS__ = {
       metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
       transformCallback: (callback) => {
         const id = Math.floor(Math.random() * 1e9);
         window["_" + id] = callback;
         return id;
       },
       unregisterCallback() {},
       async invoke(command) {
         if (command === "load_cached_manifest_state") return JSON.parse(STATE_TEXT);
         if (command === "sync_manifest_state") return new Promise((resolve) => setTimeout(() => resolve(JSON.parse(STATE_TEXT)), 6000));
         return command.startsWith("plugin:event|") ? 1 : null;
       }
     };
     ```

   - builds `STATE_TEXT` from `src/ipc/fixtures/app-state.json`, replacing `items` and `sources` with one entry per seeded package and space;
   - sets `Emulation.setCPUThrottlingRate` to 1 or 4;
   - waits until `document.querySelectorAll(".skill-card").length` equals the item count, then two animation frames;
   - reads long tasks from a `PerformanceObserver` and script, style, and layout time from `Performance.getMetrics`.

At work, the real window can be measured the same way without the mock: start the app with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>` and attach over the DevTools protocol. That gives numbers for the real engine on a real PC.
