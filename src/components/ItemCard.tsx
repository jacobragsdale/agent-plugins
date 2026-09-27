import { createContext, useContext, useState, type JSX } from "react";
import { Badge, Button, Card, DropdownMenu, Heading, Text } from "@radix-ui/themes";
import { toAppError } from "../ipc/client";
import type { AppError } from "../ipc/client";
import type { CatalogComponent, CatalogItem } from "../ipc/schemas";
import { cardDomId, listPhrase, partCounts, unusableReason } from "../lib/marketplace";
import { componentLabel, primaryActionColor, primaryActionLabel, statusColor, statusLabel } from "../lib/status";

/** What a card offers beyond its main button, supplied once by the window rather than through every group. */
export type CardExtras = Readonly<{
  /** Opens the package's page in the marketplace portal; null without a marketplace. */
  onDetails: ((item: CatalogItem) => void) | null;
  onKeepMine: (item: CatalogItem) => Promise<void>;
  onForceRemove: (item: CatalogItem, componentId?: string) => Promise<void>;
  onHold: (item: CatalogItem, held: boolean) => Promise<void>;
  /** Chooses which apps a connector goes to. */
  onApps: (item: CatalogItem, componentId: string) => void;
  /** Fills in settings such as an API key an installed connector needs. */
  onSettings: (item: CatalogItem) => void;
}>;

export const CardExtrasContext = createContext<CardExtras | null>(null);

function isInstalled(status: CatalogItem["status"]): boolean {
  return status === "installed" || status === "updateAvailable" || status === "partiallyInstalled" || status === "modified" || status === "missing";
}

export function ItemCard({
  item,
  busy,
  allBusy,
  onChange,
  onManualChange,
  onShare,
  anchor,
  onError
}: Readonly<{
  item: CatalogItem;
  busy: boolean;
  allBusy: boolean;
  onChange: (item: CatalogItem, componentId?: string) => Promise<void>;
  onManualChange: (item: CatalogItem, manual: boolean, componentId?: string) => Promise<void>;
  /** Set when this person may change who sees the package. */
  onShare?: ((item: CatalogItem) => void) | undefined;
  /** The one copy of a card a link scrolls to; a bundle's copy of it has none. */
  anchor: boolean;
  onError: (error: AppError) => void;
}>): JSX.Element {
  // A package another source installed is not this one's to touch.
  const protectedItem = item.status === "sourceConflict";
  // A package its source no longer publishes has no skill to reinstall.
  const manualLocked = busy || allBusy || protectedItem || item.status === "removed";
  const changeManual = (manual: boolean, componentId?: string): void => {
    onManualChange(item, manual, componentId).catch((reason: unknown) => {
      onError(toAppError(reason));
    });
  };
  const expandable = item.components.length > 1;
  const [componentsOpen, setComponentsOpen] = useState(true);
  const extras = useContext(CardExtrasContext);
  const unusable = availableButUnusable(item);
  const run = (task: Promise<void>): void => {
    task.catch((reason: unknown) => {
      onError(toAppError(reason));
    });
  };
  return (
    <Card className="skill-card" id={anchor ? cardDomId(item.id) : undefined}>
      <div className="skill-card-main">
        <div className="skill-copy">
          <div className="skill-title-row">
            <Heading as="h3" size="3">
              {item.name}
            </Heading>
            {uniqueKinds(item.components).map((kind) => (
              <KindBadge key={kind} kind={kind} />
            ))}
            <StatusBadges item={item} />
            {item.components.some((component) => component.kind === "skill") ? (
              <ManualInvocationToggle
                manual={item.manualInvocation}
                disabled={manualLocked}
                onToggle={() => {
                  changeManual(!item.manualInvocation);
                }}
              />
            ) : null}
            <MarketplaceBadges meta={item.marketplace} />
          </div>
          <Text as="p" color="gray" size="2">
            {item.description}
          </Text>
          <CardNotes item={item} unusable={unusable} extras={extras} />
          {item.marketplace === null ? null : (
            <div className="marketplace-meta">
              <Text color="gray" size="1">
                {item.marketplace.publisher} · v{item.marketplace.version} · {String(item.marketplace.installs)} install{item.marketplace.installs === 1 ? "" : "s"} ·{" "}
                {String(item.marketplace.installedBase)} active user{item.marketplace.installedBase === 1 ? "" : "s"}
              </Text>
              {item.marketplace.tags.map((tag) => (
                <Badge key={tag} color="gray" variant="outline" size="1">
                  {tag}
                </Badge>
              ))}
            </div>
          )}
        </div>
        <div className="item-actions">
          {onShare === undefined ? null : (
            <Button
              variant="soft"
              color="gray"
              onClick={() => {
                onShare(item);
              }}
            >
              Share…
            </Button>
          )}
          <Button
            className="skill-action skill-action-primary"
            color={primaryActionColor(item.status)}
            disabled={busy || allBusy || protectedItem || unusable !== null}
            loading={busy}
            onClick={() => {
              onChange(item).catch((reason: unknown) => {
                onError(toAppError(reason));
              });
            }}
          >
            {packageActionLabel(item)}
          </Button>
          <MoreMenu item={item} extras={extras} disabled={busy || allBusy} run={run} />
        </div>
      </div>
      {expandable ? (
        <details
          className="component-list"
          open={componentsOpen}
          onToggle={(event) => {
            setComponentsOpen(event.currentTarget.open);
          }}
        >
          <summary>
            {String(item.components.length)} parts · {componentSummary(item.components)}
          </summary>
          <ul>
            {item.components.map((component) => (
              <li key={`${component.kind}:${component.id}`}>
                <ComponentRow
                  component={component}
                  busy={busy}
                  allBusy={allBusy}
                  protectedItem={protectedItem}
                  manualLocked={manualLocked}
                  onApps={
                    extras === null || component.kind !== "mcpServer" || !isInstalled(component.status)
                      ? null
                      : () => {
                          extras.onApps(item, component.id);
                        }
                  }
                  onChange={() => onChange(item, component.id)}
                  onManualToggle={() => {
                    changeManual(!component.manualInvocation, component.id);
                  }}
                  onError={onError}
                />
              </li>
            ))}
          </ul>
        </details>
      ) : null}
    </Card>
  );
}

function availableButUnusable(item: CatalogItem): string | null {
  return item.status === "available" ? unusableReason(item) : null;
}

function StatusBadges({ item }: Readonly<{ item: CatalogItem }>): JSX.Element {
  return (
    <>
      {item.status === "available" || item.status === "installed" ? null : <Badge color={statusColor(item.status)}>{statusLabel(item.status)}</Badge>}
      {item.held ? <Badge color="gray">Updates held</Badge> : null}
    </>
  );
}

/** Which of the less common actions apply to `item`. */
function menuChoices(item: CatalogItem, extras: CardExtras): Readonly<{ details: boolean; soleConnector: string | null; settings: boolean; hold: boolean; modified: boolean }> {
  const installed = isInstalled(item.status);
  const [only] = item.components;
  return {
    details: extras.onDetails !== null && item.marketplace !== null,
    soleConnector: item.components.length === 1 && only?.kind === "mcpServer" && installed ? only.id : null,
    settings: installed && item.connectors.some((connector) => connector.environment.length > 0),
    hold: installed && item.status !== "modified",
    modified: item.status === "modified"
  };
}

/** What stands between the person and using the item: no app here can take it, a missing program, or a setting to fill in. */
function CardNotes({ item, unusable, extras }: Readonly<{ item: CatalogItem; unusable: string | null; extras: CardExtras | null }>): JSX.Element {
  const installed = isInstalled(item.status);
  const missingSettings = installed ? [...new Set(item.connectors.flatMap((connector) => connector.missingEnvironment))] : [];
  const missingPrograms = installed ? [...new Set(item.connectors.flatMap((connector) => (connector.missingProgram === null ? [] : [connector.missingProgram])))] : [];
  return (
    <>
      {unusable === null ? null : (
        <Text as="p" color="amber" size="2">
          Can't be added here: {unusable}
        </Text>
      )}
      {missingPrograms.length === 0 ? null : (
        <Text as="p" color="amber" size="2">
          Needs {listPhrase(missingPrograms)}, which isn't on this computer. Install it, or ask IT to, before the connector will work.
        </Text>
      )}
      {missingSettings.length === 0 || extras === null ? null : (
        <div className="skill-title-row">
          <Text color="amber" size="2">
            Needs {listPhrase(missingSettings)} before it works.
          </Text>
          <Button
            size="1"
            variant="soft"
            color="amber"
            onClick={() => {
              extras.onSettings(item);
            }}
          >
            Set…
          </Button>
        </div>
      )}
    </>
  );
}

/** The card's less common actions, kept behind one button so the main one stays obvious. */
function MoreMenu({ item, extras, disabled, run }: Readonly<{ item: CatalogItem; extras: CardExtras | null; disabled: boolean; run: (task: Promise<void>) => void }>): JSX.Element | null {
  if (extras === null) {
    return null;
  }
  const choices = menuChoices(item, extras);
  const { soleConnector } = choices;
  const nothing = !choices.details && soleConnector === null && !choices.settings && !choices.hold && !choices.modified;
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger disabled={disabled}>
        <Button variant="soft" color="gray" aria-label={`More for ${item.name}`}>
          More
          <DropdownMenu.TriggerIcon />
        </Button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Content>
        {choices.details ? (
          <DropdownMenu.Item
            onSelect={() => {
              extras.onDetails?.(item);
            }}
          >
            Details and what changed
          </DropdownMenu.Item>
        ) : null}
        {soleConnector === null ? null : (
          <DropdownMenu.Item
            onSelect={() => {
              extras.onApps(item, soleConnector);
            }}
          >
            Choose apps…
          </DropdownMenu.Item>
        )}
        {choices.settings ? (
          <DropdownMenu.Item
            onSelect={() => {
              extras.onSettings(item);
            }}
          >
            Connector settings…
          </DropdownMenu.Item>
        ) : null}
        {choices.hold ? (
          <DropdownMenu.Item
            onSelect={() => {
              run(extras.onHold(item, !item.held));
            }}
          >
            {item.held ? "Resume automatic updates" : "Hold updates"}
          </DropdownMenu.Item>
        ) : null}
        {choices.modified ? <ModifiedItems item={item} extras={extras} run={run} /> : null}
        {nothing ? <DropdownMenu.Item disabled>Nothing else to do</DropdownMenu.Item> : null}
      </DropdownMenu.Content>
    </DropdownMenu.Root>
  );
}

/** A changed copy can be kept as the person's own, or removed with a backup. */
function ModifiedItems({ item, extras, run }: Readonly<{ item: CatalogItem; extras: CardExtras; run: (task: Promise<void>) => void }>): JSX.Element {
  return (
    <>
      <DropdownMenu.Item
        onSelect={() => {
          run(extras.onKeepMine(item));
        }}
      >
        Keep my version…
      </DropdownMenu.Item>
      <DropdownMenu.Item
        color="red"
        onSelect={() => {
          run(extras.onForceRemove(item));
        }}
      >
        Remove…
      </DropdownMenu.Item>
    </>
  );
}

function uniqueKinds(components: readonly CatalogComponent[]): readonly string[] {
  return [...new Set(components.map((component) => component.kind))];
}

function MarketplaceBadges({ meta }: Readonly<{ meta: CatalogItem["marketplace"] }>): JSX.Element | null {
  if (meta === null) {
    return null;
  }
  return (
    <>
      {meta.lane === "official" ? <Badge color="violet">Official</Badge> : null}
      {meta.lane === "team" ? <Badge color="teal">Team</Badge> : null}
      {meta.sharedWithYou ? <Badge color="blue">Shared with you</Badge> : meta.restricted ? <Badge color="orange">Private</Badge> : null}
    </>
  );
}

/** Whether a skill runs only when asked. Clicking flips it and reinstalls the skill with the new setting. */
function ManualInvocationToggle({ manual, disabled, onToggle }: Readonly<{ manual: boolean; disabled: boolean; onToggle: () => void }>): JSX.Element {
  return (
    <Badge asChild color={manual ? "blue" : "gray"} variant={manual ? "soft" : "outline"}>
      <button
        type="button"
        className="manual-invocation-toggle"
        aria-pressed={manual}
        disabled={disabled}
        title={manual ? "Click to let the AI use this on its own." : "Click to use this only when you ask for it."}
        onClick={onToggle}
      >
        {manual ? "Only when you ask" : "Used automatically"}
      </button>
    </Badge>
  );
}

function KindBadge({ kind }: Readonly<{ kind: string }>): JSX.Element | null {
  const label = componentLabel(kind);
  return label === null ? null : (
    <Badge color="gray" variant="soft">
      {label}
    </Badge>
  );
}

function packageActionLabel(item: CatalogItem): string {
  if (item.status === "partiallyInstalled" && item.components.some((component) => component.status === "available")) {
    return "Install remaining";
  }
  return primaryActionLabel(item.status);
}

function componentSummary(components: readonly CatalogComponent[]): string {
  return partCounts(components).join(" · ");
}

function ComponentRow({
  component,
  busy,
  allBusy,
  protectedItem,
  manualLocked,
  onApps,
  onChange,
  onManualToggle,
  onError
}: Readonly<{
  component: CatalogComponent;
  busy: boolean;
  allBusy: boolean;
  protectedItem: boolean;
  manualLocked: boolean;
  onApps: (() => void) | null;
  onChange: () => Promise<void>;
  onManualToggle: () => void;
  onError: (error: AppError) => void;
}>): JSX.Element {
  const blocked = protectedItem || component.status === "sourceConflict";
  return (
    <div className="component-row">
      <div className="component-copy">
        <div className="skill-title-row">
          <Text size="2">{component.id}</Text>
          <KindBadge kind={component.kind} />
          {component.status === "available" || component.status === "installed" ? null : <Badge color={statusColor(component.status)}>{statusLabel(component.status)}</Badge>}
          {component.kind === "skill" ? (
            <ManualInvocationToggle manual={component.manualInvocation} disabled={manualLocked || component.status === "sourceConflict"} onToggle={onManualToggle} />
          ) : null}
        </div>
        <Text as="p" color="gray" size="2">
          {component.description}
        </Text>
        {component.excludedApps.length === 0 ? null : (
          <Text as="p" color="gray" size="1">
            Kept out of {String(component.excludedApps.length)} app{component.excludedApps.length === 1 ? "" : "s"}.
          </Text>
        )}
      </div>
      {onApps === null ? null : (
        <Button size="1" variant="soft" color="gray" disabled={busy || allBusy} onClick={onApps}>
          Apps…
        </Button>
      )}
      <Button
        className="skill-action"
        size="1"
        color={primaryActionColor(component.status)}
        disabled={busy || allBusy || blocked}
        loading={busy}
        onClick={() => {
          onChange().catch((reason: unknown) => {
            onError(toAppError(reason));
          });
        }}
      >
        {primaryActionLabel(component.status)}
      </Button>
    </div>
  );
}
