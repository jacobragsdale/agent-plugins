# ADR 0007: Self-service teams, sharing, owner review, bundles, and installing from the website

- Status: accepted
- Date: 2026-09-26
- Amends: [ADR 0004](0004-internal-marketplace.md) (identity: teams), [ADR 0005](0005-marketplace-access-control.md) (team namespaces, "no list means public"), and [ADR 0006](0006-web-portal-and-review.md) (admin review of every first version and every MCP version)

## Context

Every step after writing a skill needed someone else. A team namespace was a server setting tied to an AD group, so a team could not exist until an administrator edited the configuration. Sharing a skill with a few people meant typing their exact Windows account names into an allowlist, and "only us" could not be expressed at all, because an empty list meant public. An admin had to approve every package's first version and every version with an MCP server before anyone else could see it, even when the skill was meant for two colleagues. Installing from the portal meant reading instructions, switching to the app, and searching for the name again.

The people publishing and installing are mostly not developers. Each of these steps is where they stall.

## Decision

### Anyone can create a team

A team is a namespace kept in the database. It is not configured, and it is not an AD group: `Auth:TeamNamespaces` is removed. The person who creates a team is its owner. Owners add people by account name, or share an invite link. Anyone signed in who opens the link and chooses **Join** becomes a member.

Members publish, withdraw, share, and edit bundles in the team's namespace, which is what owning a namespace already meant. Owners can also add and remove members, reset the invite link, rename the team, choose who can see its skills, and delete it while it is empty. The last owner cannot leave. Admins can do everything. A team's namespace never changes, because installed skill names are built from it. A person whose derived name equals a team's still gets `u-<name>`.

The invite link grants membership, not a publishing credential. A publish is still attributed to the Windows account that made it, members can be removed one by one, and a leaked link shows up in the member list. Links are 128-bit random codes, stored in plain text so members can copy them again. Anyone who can read the database can already change everything, so hashing would protect nothing.

### Visibility is explicit, and sharing is a list

A space (personal or team) is **Public** or **Private**. A package or bundle is **Same as its space**, **Public**, or **Private**, and the most specific setting wins. So a private team can publish one public skill, and a public space can hold one private skill.

Private means the space's owners plus a share list of people, teams, and, where the server resolves them, AD groups. Being on the list lets someone see and install, never publish. The list stays when general access changes. Anything a caller may not see still answers 404, so the desktop app's handling of a source or package that is gone applies unchanged ([ADR 0005](0005-marketplace-access-control.md)).

A share link adds whoever opens it to the share list. Because being given access costs the recipient nothing, the portal redeems a share link as soon as it opens. A team invite asks first, because joining makes you a publisher and a visible member. A link exists only after someone chooses **Copy link**, and **Reset** replaces it. People already on the list stay on it.

New teams choose their visibility when they are created, and Private is the default. Personal spaces stay Public, which keeps today's behaviour.

### Owners review changes, admins keep one gate

There is no admin queue for versions. Every publish is live immediately, and version review states are gone: pending versions became live and rejected ones became withdrawn.

Anyone who can see a package but does not own its space can **suggest a change** by uploading files with a message. The package's owners see which files changed, then accept it or decline it with a note. Accepting publishes the stored upload under a version number the owner chooses, credited to the person who suggested it. People who are trusted to publish directly are the team's members; everyone else suggests.

One admin gate remains. A package whose live version has an MCP server is hidden from the public until an admin approves it. Owners, team members, and people it is shared with see it immediately. The approval belongs to the package, not the version. Each namespace is one stored archive, so the public cannot be shown an older version while members get a newer one. Later MCP versions are covered by the desktop app, which already asks each person to approve every MCP install and update.

### A kill switch replaces the gate before publishing

The owner or an admin can **Remove from every PC**. The package leaves the catalog, the index lists its ID under `revoked`, and every desktop app uninstalls it at its next sync, backing up copies the person edited. A revoked ID is sent to anyone who could see the package and to anyone whose last report still lists it installed, so losing access does not keep someone from getting the removal. Revoking can be undone, but apps do not reinstall on their own. Withdrawing a version, as before, leaves installed copies alone.

The release that removes the review queue also raises the minimum client version, so every running app understands `revoked`.

### Bundles are lists of packages

A bundle is a name, a description, and one to fifty packages, owned by a space. The packages can come from anyone and must be ones the person saving the bundle can see. A bundle has no archive, no versions, and no review, and its members always follow their latest version. Installing a bundle installs every member that is not installed yet, each in its own transaction, with one approval covering any MCP servers. Every member can still be installed on its own. Members added later show as **Install the rest**. They are not pushed to people who installed the bundle earlier.

Bundle and package IDs are unique together within a namespace, so `ns/id` always names one thing. Each caller sees only the members they may see, and sharing a bundle does not share its members. Skill packs stay as they are: a pack is one upload with one version, and a bundle groups skills that already exist.

### Installing from the website opens the app

The desktop app registers the `agent-plugins://` link type for the current user each time it starts. Its uninstaller removes it. The portal's **Install in Agent Plugins** button opens `agent-plugins://install/<ns>/<id>`, or `/<ns>/<pkg>/<skill>` for one skill of a pack. The app comes forward, syncs if the package is new to it, and asks in its own window before it changes anything, with the usual MCP approval.

Any website can open such a link, so a link carries only catalog IDs, is parsed strictly, can never add a source or grant the MCP approval, and shows at most one confirmation at a time.

The portal picks the button from the account's latest heartbeat: **Install**, **Installed**, **Get Agent Plugins**, or **Update Agent Plugins** for apps older than 0.2.0. Edge and Chrome ask once before opening the app. A corporate deployment can allow the marketplace origin with the `AutoLaunchProtocolsFromOrigins` policy so they never ask.

## Consequences

Teams, sharing, and review need no administrator, and a skill reaches its intended audience in one step. The cost is less inspection before publishing. Skill versions were never reviewed after the first one, and they already updated on every PC automatically, so what goes away is a look at each package's first version. For public MCP servers the admin now approves once per package, not once per version. The kill switch and publisher accountability are what respond to a bad package.

The server gains team, membership, link, directory, suggestion, and bundle tables, and version review fields are removed. Every read path goes through one visibility check that also applies the public MCP gate. The desktop app gains a link handler, a Teams dialog, sharing, bundles, and revoked-package removal. The CLI's `access` verb becomes `share`, and new `team`, `bundle`, `review`, and `revoke` verbs are added.

AD groups remain optional share-list entries where `Auth:LdapDomain` resolves them, but nothing depends on them.
