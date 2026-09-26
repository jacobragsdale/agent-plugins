import type { JSX } from "react";
import { Badge, Button, Heading, Text } from "@radix-ui/themes";
import { toAppError } from "../ipc/client";
import type { AppError } from "../ipc/client";
import type { BulkAction, CatalogItem, SourceState, SourceStatus } from "../ipc/schemas";
import { savedCopyLabel } from "../lib/connectivity";
import { supportsBulkAction } from "../lib/status";
import { ItemCard } from "./ItemCard";

/**
 * A source or catalog the last check couldn't refresh keeps showing its last
 * good copy. A grey badge says so, never red: the next check retries it on its
 * own and there is nothing for a person to do. The reason stays in the
 * tooltip for whoever is asked to look into it.
 */
export function FreshnessBadge({
  status,
  refreshFailed,
  lastSuccessAt,
  message
}: Readonly<{ status: SourceStatus; refreshFailed: boolean; lastSuccessAt: number | null; message: string | null }>): JSX.Element | null {
  if (status !== "stale" && !refreshFailed) {
    return null;
  }
  return (
    <Badge color="gray" title={message ?? "Agent Plugins will try again automatically."}>
      {savedCopyLabel(lastSuccessAt)}
    </Badge>
  );
}

/** What a source is busy with: a bulk action, or being removed. */
export type SourceAction = BulkAction | "remove";

export function SourceGroup({
  source,
  items,
  busyIds,
  allBusy,
  running,
  filtering,
  onItemChange,
  onManualChange,
  onBulk,
  onError
}: Readonly<{
  source: SourceState;
  items: readonly CatalogItem[];
  busyIds: ReadonlySet<string>;
  allBusy: boolean;
  /** The source-wide action in progress, whose button spins. */
  running: SourceAction | null;
  filtering: boolean;
  onItemChange: (item: CatalogItem, componentId?: string) => Promise<void>;
  onManualChange: (item: CatalogItem, manual: boolean, componentId?: string) => Promise<void>;
  onBulk: (source: SourceState, action: BulkAction) => Promise<void>;
  onError: (error: AppError) => void;
}>): JSX.Element {
  // Bulk actions cover the whole source, so they stay out of sight while a
  // filter is showing only part of it.
  const canInstall = !filtering && items.some((item) => supportsBulkAction(item.status, "install"));
  const canReplace = !filtering && items.some((item) => supportsBulkAction(item.status, "replace"));
  const canUninstall = !filtering && items.some((item) => supportsBulkAction(item.status, "uninstall"));
  return (
    <section className="source-group">
      <div className="source-heading">
        <div>
          <div className="source-title-row">
            {/* The source URL is a package archive, so the name is not a link: opening it would download a zip. */}
            <Heading as="h2" size="4">
              {source.name}
            </Heading>
            {items.length === 0 ? null : <FreshnessBadge status={source.status} refreshFailed={source.refreshFailed} lastSuccessAt={source.lastSuccessAtEpochSeconds} message={source.message} />}
          </div>
          <Text as="p" color="gray" size="2">
            {source.description}
          </Text>
        </div>
        <div className="source-group-actions">
          {canInstall ? (
            <Button
              size="1"
              variant="soft"
              color="green"
              disabled={allBusy}
              loading={running === "install"}
              onClick={() => {
                onBulk(source, "install").catch((reason: unknown) => {
                  onError(toAppError(reason));
                });
              }}
            >
              Install all
            </Button>
          ) : null}
          {canReplace ? (
            <Button
              size="1"
              variant="soft"
              color="amber"
              disabled={allBusy}
              loading={running === "replace"}
              onClick={() => {
                onBulk(source, "replace").catch((reason: unknown) => {
                  onError(toAppError(reason));
                });
              }}
            >
              Replace all
            </Button>
          ) : null}
          {canUninstall ? (
            <Button
              size="1"
              variant="soft"
              color="red"
              disabled={allBusy}
              loading={running === "uninstall"}
              onClick={() => {
                onBulk(source, "uninstall").catch((reason: unknown) => {
                  onError(toAppError(reason));
                });
              }}
            >
              Uninstall all
            </Button>
          ) : null}
        </div>
      </div>
      <div className="skills-list">
        {items.map((item) => (
          <ItemCard key={item.id} item={item} busy={busyIds.has(item.id)} allBusy={allBusy} onChange={onItemChange} onManualChange={onManualChange} onError={onError} />
        ))}
        {items.length === 0 ? <Text color="gray">This source has no packages right now.</Text> : null}
      </div>
    </section>
  );
}
