import { useEffect, useState } from "react";
import type { JSX } from "react";
import { Badge, Button, Callout, Card, Checkbox, Dialog, Heading, Select, Text, TextArea, TextField } from "@radix-ui/themes";
import { invokeParsed, toAppError } from "../ipc/client";
import type { AppError } from "../ipc/client";
import { savedBundleSchema } from "../ipc/schemas";
import type { AppIdentity, BulkAction, BundleState, CatalogItem } from "../ipc/schemas";
import { bundleSummary, cardDomId, ID_PATTERN, ownsSpace, spaceLabel, suggestId } from "../lib/marketplace";
import type { BundleSummary } from "../lib/marketplace";
import { ItemCard } from "./ItemCard";
import { ErrorMessage } from "./Notice";
import { returnFocus } from "../lib/returnFocus";

const MAX_MEMBERS = 50;

function primaryAction(summary: BundleSummary): Readonly<{ label: string; action: BulkAction; color: "green" | "red" }> {
  switch (summary.action) {
    case "install":
    case "empty":
      return { label: "Install all", action: "install", color: "green" };
    case "installRest":
      return { label: "Install the rest", action: "install", color: "green" };
    case "installed":
      return { label: "Uninstall all", action: "uninstall", color: "red" };
  }
}

/** Bundles: sets of packages people install together. Each member is still an ordinary package below. */
export function BundleGroup({
  bundles,
  items,
  identity,
  busyBundles,
  busyIds,
  allBusy,
  onRun,
  onEdit,
  onDelete,
  onShare,
  onItemChange,
  onManualChange,
  onError
}: Readonly<{
  bundles: readonly BundleState[];
  items: readonly CatalogItem[];
  identity: AppIdentity | null;
  busyBundles: ReadonlyMap<string, BulkAction | "remove">;
  busyIds: ReadonlySet<string>;
  allBusy: boolean;
  onRun: (bundle: BundleState, action: BulkAction) => Promise<void>;
  onEdit: (bundle: BundleState) => void;
  onDelete: (bundle: BundleState) => Promise<void>;
  onShare: (target: string, label: string) => void;
  onItemChange: (item: CatalogItem, componentId?: string) => Promise<void>;
  onManualChange: (item: CatalogItem, manual: boolean, componentId?: string) => Promise<void>;
  onError: (error: AppError) => void;
}>): JSX.Element | null {
  // Member cards mount only while their bundle is open: each is a full card with its own menu.
  const [openBundles, setOpenBundles] = useState<ReadonlySet<string>>(new Set());
  if (bundles.length === 0) {
    return null;
  }
  return (
    <section className="source-group">
      <div className="source-heading">
        <div>
          <Heading as="h2" size="4">
            Bundles
          </Heading>
          <Text as="p" color="gray" size="2">
            Sets of skills to install together. You can still install each one on its own.
          </Text>
        </div>
      </div>
      <div className="skills-list">
        {bundles.map((bundle) => {
          const summary = bundleSummary(bundle, items);
          const primary = primaryAction(summary);
          const running = busyBundles.get(bundle.id) ?? null;
          const busy = allBusy || running !== null;
          const owned = ownsSpace(identity, bundle.namespace);
          const count = bundle.members.length;
          return (
            <Card key={bundle.id} className="skill-card" id={cardDomId(bundle.id)}>
              <div className="skill-card-main">
                <div className="skill-copy">
                  <div className="skill-title-row">
                    <Heading as="h3" size="3">
                      {bundle.name}
                    </Heading>
                    <Badge color="gray" variant="soft">
                      Bundle
                    </Badge>
                    {bundle.lane === "official" ? <Badge color="violet">Official</Badge> : null}
                    {bundle.lane === "team" ? <Badge color="teal">Team</Badge> : null}
                    {bundle.sharedWithYou ? <Badge color="blue">Shared with you</Badge> : bundle.restricted ? <Badge color="orange">Private</Badge> : null}
                  </div>
                  {bundle.description.length === 0 ? null : (
                    <Text as="p" color="gray" size="2">
                      {bundle.description}
                    </Text>
                  )}
                  <div className="marketplace-meta">
                    <Text color="gray" size="1">
                      {bundle.publisher} · {String(count)} skill{count === 1 ? "" : "s"} · {String(summary.installed)} of {String(summary.members.length)} installed
                    </Text>
                  </div>
                </div>
                <div className="item-actions">
                  {owned ? (
                    <>
                      <Button
                        variant="soft"
                        color="gray"
                        disabled={busy}
                        onClick={() => {
                          onShare(bundle.id, bundle.name);
                        }}
                      >
                        Share…
                      </Button>
                      <Button
                        variant="soft"
                        color="gray"
                        disabled={busy}
                        onClick={() => {
                          onEdit(bundle);
                        }}
                      >
                        Edit
                      </Button>
                      <Button
                        variant="soft"
                        color="red"
                        disabled={busy}
                        loading={running === "remove"}
                        onClick={() => {
                          onDelete(bundle).catch((reason: unknown) => {
                            onError(toAppError(reason));
                          });
                        }}
                      >
                        Delete
                      </Button>
                    </>
                  ) : null}
                  <Button
                    className="skill-action skill-action-primary"
                    color={primary.color}
                    disabled={busy || summary.action === "empty"}
                    loading={running === primary.action}
                    onClick={() => {
                      onRun(bundle, primary.action).catch((reason: unknown) => {
                        onError(toAppError(reason));
                      });
                    }}
                  >
                    {primary.label}
                  </Button>
                </div>
              </div>
              <details
                className="component-list"
                open={openBundles.has(bundle.id)}
                onToggle={(event) => {
                  const open = event.currentTarget.open;
                  setOpenBundles((current) => {
                    const next = new Set(current);
                    if (open) {
                      next.add(bundle.id);
                    } else {
                      next.delete(bundle.id);
                    }
                    return next;
                  });
                }}
              >
                <summary>Show its skills</summary>
                {openBundles.has(bundle.id) ? (
                  <div className="bundle-members">
                    {summary.members.map((item) => (
                      <ItemCard key={item.id} item={item} busy={busyIds.has(item.id)} allBusy={busy} anchor={false} onChange={onItemChange} onManualChange={onManualChange} onError={onError} />
                    ))}
                    {summary.missing > 0 ? (
                      <Text as="p" color="gray" size="1">
                        {String(summary.missing)} more {summary.missing === 1 ? "isn't" : "aren't"} available to you yet.
                      </Text>
                    ) : null}
                  </div>
                ) : null}
              </details>
            </Card>
          );
        })}
      </div>
    </section>
  );
}

/** Whether a bundle has what saving needs: a name, a usable id, a space, and 1–50 members. */
function complete(name: string, bundleId: string, space: string, members: readonly string[]): boolean {
  return name.trim().length > 0 && ID_PATTERN.test(bundleId) && space.length > 0 && members.length > 0 && members.length <= MAX_MEMBERS;
}

/** Marketplace packages matching the search: only those can go in a bundle. */
function bundleChoices(items: readonly CatalogItem[], search: string): readonly CatalogItem[] {
  const needle = search.trim().toLowerCase();
  return items.filter((item) => item.marketplace !== null && `${item.id} ${item.name} ${item.marketplace.publisher}`.toLowerCase().includes(needle));
}

/** A new bundle (`bundle` null) or an edit to one this person owns. */
export type BundleEdit = Readonly<{ bundle: BundleState | null }>;

/** Name a bundle, pick its space, and tick the packages that go in it. */
export function BundleDialog({
  request,
  identity,
  items,
  onClose,
  onSaved
}: Readonly<{ request: BundleEdit | null; identity: AppIdentity | null; items: readonly CatalogItem[]; onClose: () => void; onSaved: () => void }>): JSX.Element {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [space, setSpace] = useState("");
  const [members, setMembers] = useState<readonly string[]>([]);
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const editing = request?.bundle ?? null;

  useEffect(() => {
    if (request === null) {
      return;
    }
    const { bundle } = request;
    setName(bundle?.name ?? "");
    setDescription(bundle?.description ?? "");
    setSpace(bundle?.namespace ?? identity?.namespace ?? "");
    setMembers(bundle?.members ?? []);
    setSearch("");
    setError(null);
  }, [request, identity]);

  const bundleId = editing?.bundleId ?? suggestId(name);
  const ready = !busy && complete(name, bundleId, space, members);
  const choices = bundleChoices(items, search);

  async function save(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      await invokeParsed("save_bundle", savedBundleSchema, { namespace: space, bundleId, name: name.trim(), description: description.trim(), members });
      onSaved();
      onClose();
    } catch (reason) {
      setError(toAppError(reason, "Couldn't save the bundle."));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog.Root
      open={request !== null}
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
    >
      <Dialog.Content maxWidth="640px" {...returnFocus}>
        <Dialog.Title>{editing === null ? "New bundle" : `Edit ${editing.name}`}</Dialog.Title>
        <Dialog.Description size="2">A bundle lets people install several skills at once. Anyone can still install each one on its own.</Dialog.Description>
        {error === null ? null : (
          <Callout.Root className="app-callout" color="red" role="alert">
            <ErrorMessage summary={error.summary} detail={error.detail} />
          </Callout.Root>
        )}
        <div className="manage-section form-grid">
          <label className="form-field">
            <Text size="2">Name</Text>
            <TextField.Root
              placeholder="New starter kit"
              value={name}
              maxLength={120}
              onChange={(event) => {
                setName(event.currentTarget.value);
              }}
            />
          </label>
          <label className="form-field">
            <Text size="2">What it's for</Text>
            <TextArea
              value={description}
              maxLength={1024}
              onChange={(event) => {
                setDescription(event.currentTarget.value);
              }}
            />
          </label>
          {editing === null ? (
            <div className="form-field">
              <Text size="2">Whose bundle is it?</Text>
              <Select.Root value={space} onValueChange={setSpace}>
                <Select.Trigger aria-label="Whose bundle is it?" />
                <Select.Content>
                  {(identity?.namespaces ?? []).map((namespace) => (
                    <Select.Item key={namespace} value={namespace}>
                      {spaceLabel(identity, namespace)}
                    </Select.Item>
                  ))}
                </Select.Content>
              </Select.Root>
            </div>
          ) : null}
          <div className="form-field">
            <Text size="2">
              Skills in it ({String(members.length)} chosen{members.length > MAX_MEMBERS ? `, at most ${String(MAX_MEMBERS)}` : ""})
            </Text>
            <TextField.Root
              type="search"
              placeholder="Search packages…"
              aria-label="Search packages"
              value={search}
              onChange={(event) => {
                setSearch(event.currentTarget.value);
              }}
            />
            <ul className="bundle-choices">
              {choices.map((item) => (
                <li key={item.id}>
                  <Text as="label" size="2" className="bundle-choice">
                    <Checkbox
                      checked={members.includes(item.id)}
                      onCheckedChange={(checked) => {
                        setMembers((current) => (checked === true ? [...current, item.id] : current.filter((id) => id !== item.id)));
                      }}
                    />
                    {item.name}
                    <Text color="gray" size="1">
                      {item.marketplace?.publisher ?? item.sourceName}
                    </Text>
                  </Text>
                </li>
              ))}
            </ul>
          </div>
        </div>
        <div className="dialog-actions">
          <Dialog.Close>
            <Button variant="soft" color="gray">
              Cancel
            </Button>
          </Dialog.Close>
          <Button
            loading={busy}
            disabled={!ready}
            onClick={() => {
              save().catch((reason: unknown) => {
                setError(toAppError(reason));
              });
            }}
          >
            Save bundle
          </Button>
        </div>
      </Dialog.Content>
    </Dialog.Root>
  );
}
