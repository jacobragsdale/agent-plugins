import { useState, type JSX } from "react";
import { Badge, Button, Card, Heading, Text } from "@radix-ui/themes";
import { toAppError } from "../ipc/client";
import type { AppError } from "../ipc/client";
import type { CatalogComponent, CatalogItem } from "../ipc/schemas";
import { componentLabel, primaryActionColor, primaryActionLabel, statusColor, statusLabel } from "../lib/status";

export function ItemCard({
  item,
  busy,
  allBusy,
  onChange,
  onManualChange,
  onError
}: Readonly<{
  item: CatalogItem;
  busy: boolean;
  allBusy: boolean;
  onChange: (item: CatalogItem, componentId?: string) => Promise<void>;
  onManualChange: (item: CatalogItem, manual: boolean, componentId?: string) => Promise<void>;
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
  return (
    <Card className="skill-card">
      <div className="skill-card-main">
        <div className="skill-copy">
          <div className="skill-title-row">
            <Heading as="h3" size="3">
              {item.name}
            </Heading>
            {uniqueKinds(item.components).map((kind) => (
              <KindBadge key={kind} kind={kind} />
            ))}
            {item.status === "available" || item.status === "installed" ? null : <Badge color={statusColor(item.status)}>{statusLabel(item.status)}</Badge>}
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
          <Button
            className="skill-action skill-action-primary"
            color={primaryActionColor(item.status)}
            disabled={busy || allBusy || protectedItem}
            loading={busy}
            onClick={() => {
              onChange(item).catch((reason: unknown) => {
                onError(toAppError(reason));
              });
            }}
          >
            {packageActionLabel(item)}
          </Button>
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
      {meta.restricted ? <Badge color="orange">Restricted</Badge> : null}
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
  const skills = components.filter((component) => component.kind === "skill").length;
  const servers = components.filter((component) => component.kind === "mcpServer").length;
  const parts: string[] = [];
  if (skills > 0) {
    parts.push(`${String(skills)} skill${skills === 1 ? "" : "s"}`);
  }
  if (servers > 0) {
    parts.push(`${String(servers)} connector${servers === 1 ? "" : "s"}`);
  }
  return parts.join(" · ");
}

function ComponentRow({
  component,
  busy,
  allBusy,
  protectedItem,
  manualLocked,
  onChange,
  onManualToggle,
  onError
}: Readonly<{
  component: CatalogComponent;
  busy: boolean;
  allBusy: boolean;
  protectedItem: boolean;
  manualLocked: boolean;
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
      </div>
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
