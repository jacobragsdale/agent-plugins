import type { JSX, ReactNode } from "react";
import { Button, Select, Spinner, Text, TextField, VisuallyHidden } from "@radix-ui/themes";
import type { PreflightCheck } from "../ipc/schemas";

export function StatusButton({ problems, disabled, onClick }: Readonly<{ problems: readonly PreflightCheck[]; disabled: boolean; onClick: () => void }>): JSX.Element {
  const problem = problems.at(0) ?? null;
  return (
    <Button variant="soft" {...(problem === null ? {} : { color: "red" as const })} disabled={disabled} onClick={onClick}>
      <span className="identity-chip">
        <span className={problem === null ? "status-dot" : "status-dot status-dot-problem"} aria-hidden="true" />
        {statusButtonLabel(problems)}
      </span>
    </Button>
  );
}

function statusButtonLabel(problems: readonly PreflightCheck[]): string {
  const problem = problems.at(0);
  if (problem === undefined) {
    return "Status";
  }
  return problems.length === 1 ? problem.title : `${String(problems.length)} problems`;
}

/** Which cards the catalog shows, and in what order. */
export function CatalogFilters({
  filter,
  sort,
  onFilterChange,
  onSortChange
}: Readonly<{ filter: string; sort: string; onFilterChange: (filter: string) => void; onSortChange: (sort: string) => void }>): JSX.Element {
  return (
    <>
      <Select.Root size="1" value={filter} onValueChange={onFilterChange}>
        <Select.Trigger aria-label="Show" />
        <Select.Content>
          <Select.Item value="all">Everything</Select.Item>
          <Select.Item value="installed">Installed</Select.Item>
          <Select.Item value="updates">Updates</Select.Item>
          <Select.Item value="official">Official</Select.Item>
          <Select.Item value="team">Teams</Select.Item>
          <Select.Item value="shared">Shared with you</Select.Item>
        </Select.Content>
      </Select.Root>
      <Select.Root size="1" value={sort} onValueChange={onSortChange}>
        <Select.Trigger aria-label="Sort" />
        <Select.Content>
          <Select.Item value="name">By name</Select.Item>
          <Select.Item value="used">Most used</Select.Item>
        </Select.Content>
      </Select.Root>
    </>
  );
}

export function CatalogToolbar({
  query,
  matches,
  driftOnly,
  onQueryChange,
  onClearDrift,
  children
}: Readonly<{ query: string; matches: number; driftOnly: boolean; onQueryChange: (query: string) => void; onClearDrift: () => void; children?: ReactNode }>): JSX.Element {
  const searching = query.trim().length > 0;
  return (
    <div className="catalog-toolbar">
      <VisuallyHidden>
        <label htmlFor="package-search">Search skills and connectors</label>
      </VisuallyHidden>
      <TextField.Root
        id="package-search"
        type="search"
        className="search-field"
        placeholder="Search skills, connectors, publishers, tags…"
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
            Skills changed on this computer
          </Text>
          <Button size="1" variant="soft" onClick={onClearDrift}>
            Show all
          </Button>
        </>
      ) : null}
      <div className="toolbar-actions">{children}</div>
    </div>
  );
}

/** The header line for a failed check, in words a person acts on; System status has the technical detail. */
export function plainProblem(problem: PreflightCheck): string {
  if (problem.id.startsWith("auth.") || problem.id === "host.domain") {
    return "Can't sign you in to the marketplace. Are you connected to the corporate network or VPN?";
  }
  if (problem.id === "server.health" || problem.id === "host.dns" || problem.id === "host.serverUrl") {
    return "Can't reach the marketplace. Check your connection or VPN.";
  }
  if (problem.id === "host.tls") {
    return "This computer doesn't trust the marketplace's certificate. Ask IT.";
  }
  if (problem.id === "server.clientVersion") {
    return "This version of Agent Plugins is too old for the marketplace. Download the new one.";
  }
  return `${problem.title}: ${problem.detail}`;
}

export function SyncMeta({
  checked,
  checking,
  problems,
  blocked,
  newVersion,
  onGetNewVersion
}: Readonly<{ checked: string; checking: boolean; problems: readonly PreflightCheck[]; blocked: boolean; newVersion: boolean; onGetNewVersion: () => void }>): JSX.Element {
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
        {newVersion && problem === null ? (
          <Button size="1" variant="ghost" onClick={onGetNewVersion}>
            A new version of Agent Plugins is available. Get it
          </Button>
        ) : null}
      </div>
      {problem === null ? null : (
        <Text as="p" color="red" size="2">
          {plainProblem(problem)}
          {problems.length > 1 ? ` (+${String(problems.length - 1)} more)` : ""}
          {blocked ? " Agent Plugins can't install or update skills until this is fixed." : ""} Open Status for details.
        </Text>
      )}
    </>
  );
}
