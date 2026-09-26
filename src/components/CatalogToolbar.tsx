import type { JSX } from "react";
import { Button, Spinner, Text, TextField, VisuallyHidden } from "@radix-ui/themes";
import type { AgentProfile, AppIdentity, PreflightCheck } from "../ipc/schemas";

/** Opens `profile`'s app with the create-a-skill prompt; hidden while no such app is detected. */
export function CreateSkillButton({ profile, running, onClick }: Readonly<{ profile: AgentProfile | null; running: boolean; onClick: (profile: AgentProfile) => void }>): JSX.Element | null {
  if (profile === null) {
    return null;
  }
  return (
    <Button
      variant="soft"
      loading={running}
      disabled={running}
      onClick={() => {
        onClick(profile);
      }}
    >
      Create a skill
    </Button>
  );
}

export function StatusButton({
  problems,
  identity,
  disabled,
  onClick
}: Readonly<{ problems: readonly PreflightCheck[]; identity: AppIdentity | null; disabled: boolean; onClick: () => void }>): JSX.Element {
  const problem = problems.at(0) ?? null;
  return (
    <Button variant="soft" {...(problem === null ? {} : { color: "red" as const })} disabled={disabled} onClick={onClick}>
      <span className="identity-chip">
        <span className={problem === null ? "status-dot" : "status-dot status-dot-problem"} aria-hidden="true" />
        {statusButtonLabel(problems, identity)}
      </span>
    </Button>
  );
}

function statusButtonLabel(problems: readonly PreflightCheck[], identity: AppIdentity | null): string {
  const problem = problems.at(0);
  if (problem === undefined) {
    return identity === null ? "System status" : identity.namespace;
  }
  return problems.length === 1 ? problem.title : `${String(problems.length)} problems`;
}

export function CatalogToolbar({
  query,
  matches,
  driftOnly,
  onQueryChange,
  onClearDrift
}: Readonly<{ query: string; matches: number; driftOnly: boolean; onQueryChange: (query: string) => void; onClearDrift: () => void }>): JSX.Element {
  const searching = query.trim().length > 0;
  return (
    <div className="catalog-toolbar">
      <VisuallyHidden>
        <label htmlFor="package-search">Search packages</label>
      </VisuallyHidden>
      <TextField.Root
        id="package-search"
        type="search"
        className="search-field"
        placeholder="Search packages, publishers, tags…"
        value={query}
        onChange={(event) => {
          onQueryChange(event.currentTarget.value);
        }}
      />
      {searching ? (
        <Text color="gray" size="1">
          {String(matches)} {matches === 1 ? "match" : "matches"}
        </Text>
      ) : null}
      {driftOnly ? (
        <>
          <Text color="amber" size="1">
            Packages with local changes
          </Text>
          <Button size="1" variant="soft" onClick={onClearDrift}>
            Show all
          </Button>
        </>
      ) : null}
    </div>
  );
}

export function SyncMeta({ checked, checking, problems, blocked }: Readonly<{ checked: string; checking: boolean; problems: readonly PreflightCheck[]; blocked: boolean }>): JSX.Element {
  const problem = problems.at(0) ?? null;
  return (
    <>
      <div className="sync-meta">
        <Text color="gray" size="1">
          Last checked: {checked}
        </Text>
        {checking ? (
          <Text className="sync-checking" color="gray" size="1" role="status">
            <Spinner size="1" /> Checking…
          </Text>
        ) : null}
      </div>
      {problem === null ? null : (
        <Text as="p" color="red" size="2">
          {problem.title}: {problem.detail}
          {problems.length > 1 ? ` (+${String(problems.length - 1)} more)` : ""}
          {blocked ? " Agent Plugins can't install or update packages until this is fixed." : ""} Open System status for details.
        </Text>
      )}
    </>
  );
}
