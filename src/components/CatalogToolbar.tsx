import type { JSX } from "react";
import { Badge, Button, Text, TextField } from "@radix-ui/themes";
import type { AppIdentity, CheckStatus } from "../ipc/schemas";
import { statusColor } from "./SystemStatusDialog";

export function StatusButton({ status, identity, disabled, onClick }: Readonly<{ status: CheckStatus; identity: AppIdentity | null; disabled: boolean; onClick: () => void }>): JSX.Element {
  return (
    <Button variant="soft" disabled={disabled} onClick={onClick}>
      <span className="identity-chip">
        <Badge color={statusColor(status)} variant="solid" radius="full">
          {statusGlyph(status)}
        </Badge>
        {identity === null ? "System Status" : identity.namespace}
      </span>
    </Button>
  );
}

function statusGlyph(status: CheckStatus): string {
  switch (status) {
    case "ok":
      return "OK";
    case "warn":
      return "!";
    case "fail":
      return "✕";
    case "skipped":
      return "…";
  }
}

export function CatalogToolbar({ query, matches, onQueryChange }: Readonly<{ query: string; matches: number; onQueryChange: (query: string) => void }>): JSX.Element {
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
    </div>
  );
}

export function SyncMeta({ checked, marketplaceUrl, blocked }: Readonly<{ checked: string; marketplaceUrl: string | null; blocked: boolean }>): JSX.Element {
  return (
    <>
      <div className="sync-meta">
        <Text color="gray" size="1">
          Last checked: {checked}
          {marketplaceUrl === null ? "" : ` · Marketplace ${marketplaceUrl}`}
        </Text>
      </div>
      {blocked ? (
        <Text as="p" color="red" size="2">
          A blocking preflight check failed. Open System Status for details.
        </Text>
      ) : null}
    </>
  );
}
