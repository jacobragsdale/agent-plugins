import type { JSX } from "react";
import { Button, Text, TextField } from "@radix-ui/themes";
import type { AppIdentity, PreflightCheck } from "../ipc/schemas";

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
      <TextField.Root
        className="search-field"
        placeholder="Search skills, publishers, tags…"
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

export function SyncMeta({ checked, problems, blocked }: Readonly<{ checked: string; problems: readonly PreflightCheck[]; blocked: boolean }>): JSX.Element {
  const problem = problems.at(0) ?? null;
  return (
    <>
      <div className="sync-meta">
        <Text color="gray" size="1">
          Last checked: {checked}
        </Text>
      </div>
      {problem === null ? null : (
        <Text as="p" color="red" size="2">
          {problem.title}: {problem.detail}
          {problems.length > 1 ? ` (+${String(problems.length - 1)} more)` : ""}
          {blocked ? " Agent Plugins will not install or sync until it is resolved." : ""} Open System status for details.
        </Text>
      )}
    </>
  );
}
