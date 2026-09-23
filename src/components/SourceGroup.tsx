import type { JSX } from "react";
import { Badge, Button, Callout, Heading, Text } from "@radix-ui/themes";
import { toAppError } from "../ipc/client";
import type { AppError } from "../ipc/client";
import type { BulkAction, CatalogItem, SourceState, SourceStatus } from "../ipc/schemas";
import { savedCopyLabel } from "../lib/connectivity";
import { supportsBulkAction } from "../lib/status";
import { ItemCard } from "./ItemCard";
import { ErrorMessage } from "./Notice";

/**
 * A source or catalog the last check couldn't reach shows its saved copy: a
 * neutral badge, never red. Red stays for a real error.
 */
export function FreshnessBadge({ status, refreshFailed, lastSuccessAt }: Readonly<{ status: SourceStatus; refreshFailed: boolean; lastSuccessAt: number | null }>): JSX.Element | null {
  if (status === "stale") {
    return (
      <Badge color="gray" title="Agent Plugins couldn't reach the server and will retry automatically.">
        {savedCopyLabel(lastSuccessAt)}
      </Badge>
    );
  }
  return refreshFailed ? <Badge color="red">Refresh failed</Badge> : null;
}

export function SourceGroup({
  source,
  items,
  busyIds,
  allBusy,
  filtering,
  onItemChange,
  onBulk,
  onError
}: Readonly<{
  source: SourceState;
  items: readonly CatalogItem[];
  busyIds: ReadonlySet<string>;
  allBusy: boolean;
  filtering: boolean;
  onItemChange: (item: CatalogItem, componentId?: string) => Promise<void>;
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
            <FreshnessBadge status={source.status} refreshFailed={source.refreshFailed} lastSuccessAt={source.lastSuccessAtEpochSeconds} />
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
      {/* A stale source's message is the unreachable server; the offline banner already says that. */}
      {source.message === null || source.status === "stale" ? null : (
        <Callout.Root className="app-callout" color="red">
          <Callout.Text>{source.message}</Callout.Text>
        </Callout.Root>
      )}
      {source.catalogErrors.length === 0 ? null : (
        <Callout.Root className="app-callout" color="amber">
          <ErrorMessage
            summary={`${String(source.catalogErrors.length)} package${source.catalogErrors.length === 1 ? "" : "s"} in this source couldn't be read and ${source.catalogErrors.length === 1 ? "is" : "are"} not shown.`}
            detail={source.catalogErrors.map((catalogError) => `${catalogError.path}: ${catalogError.message}`).join("\n")}
          />
        </Callout.Root>
      )}
      <div className="skills-list">
        {items.map((item) => (
          <ItemCard key={item.id} item={item} busy={busyIds.has(item.id)} allBusy={allBusy} onChange={onItemChange} onError={onError} />
        ))}
        {items.length === 0 ? <Text color="gray">This source has no packages right now.</Text> : null}
      </div>
    </section>
  );
}
