import { useState } from "react";
import type { JSX } from "react";
import { Badge, Button, Callout, Dialog, Text, TextField } from "@radix-ui/themes";
import { invokeParsed, toAppError } from "../ipc/client";
import type { AppError } from "../ipc/client";
import { linkPreviewSchema, linkResultSchema } from "../ipc/schemas";
import type { AgentProfile, CatalogItem, LinkPreview, LinkResult } from "../ipc/schemas";
import { appsFor, appsPhrase, bundleSummary, linkAction, listPhrase, partCounts, unusableReason } from "../lib/marketplace";
import type { LinkTarget } from "../lib/marketplace";
import { statusColor, statusLabel } from "../lib/status";
import { ErrorMessage } from "./Notice";

/** Pasting a team invite or a share link someone sent: see what it is, then join or get it. */
export function OpenLinkDialog({ open, onOpenChange, onRedeemed }: Readonly<{ open: boolean; onOpenChange: (open: boolean) => void; onRedeemed: (result: LinkResult) => void }>): JSX.Element {
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<LinkPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  function reset(): void {
    setText("");
    setPreview(null);
    setError(null);
  }

  async function check(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      setPreview(await invokeParsed("preview_link", linkPreviewSchema, { link: text.trim() }));
    } catch (reason) {
      setError(toAppError(reason, "Couldn't open that link."));
    } finally {
      setBusy(false);
    }
  }

  async function redeem(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      const result = await invokeParsed("redeem_link", linkResultSchema, { link: text.trim() });
      reset();
      onOpenChange(false);
      onRedeemed(result);
    } catch (reason) {
      setError(toAppError(reason, "Couldn't open that link."));
    } finally {
      setBusy(false);
    }
  }

  const run = (task: Promise<void>): void => {
    task.catch((reason: unknown) => {
      setError(toAppError(reason));
    });
  };

  return (
    <Dialog.Root
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          reset();
        }
        onOpenChange(next);
      }}
    >
      <Dialog.Content maxWidth="520px">
        <Dialog.Title>Open a link</Dialog.Title>
        <Dialog.Description size="2">Paste a team invite or a share link someone sent you.</Dialog.Description>
        {error === null ? null : (
          <Callout.Root className="app-callout" color="red" role="alert">
            <ErrorMessage summary={error.summary} detail={error.detail} />
          </Callout.Root>
        )}
        <div className="link-row manage-section">
          <TextField.Root
            className="link-field"
            placeholder="https://…/l/…"
            aria-label="Link"
            value={text}
            onChange={(event) => {
              setText(event.currentTarget.value);
              setPreview(null);
            }}
          />
          <Button
            variant="soft"
            loading={busy && preview === null}
            disabled={busy || text.trim().length === 0}
            onClick={() => {
              run(check());
            }}
          >
            Check
          </Button>
        </div>
        {preview === null ? null : (
          <Text as="p" size="2" className="manage-section">
            {preview.kind === "invite"
              ? `${preview.by} invited you to join ${preview.name}. Members can publish skills to it and see its private skills.`
              : `${preview.by} shared ${preview.name} with you.`}
          </Text>
        )}
        <div className="dialog-actions">
          <Dialog.Close>
            <Button variant="soft" color="gray">
              Cancel
            </Button>
          </Dialog.Close>
          {preview === null ? null : (
            <Button
              loading={busy}
              disabled={busy}
              onClick={() => {
                run(redeem());
              }}
            >
              {preview.kind === "invite" ? "Join" : "Get it"}
            </Button>
          )}
        </div>
      </Dialog.Content>
    </Dialog.Root>
  );
}

/** A request to install from a link, and what the window found for it. */
export type LinkRequest = Readonly<{ target: LinkTarget }>;

function itemPart(item: CatalogItem, componentId: string | null): Readonly<{ name: string; status: CatalogItem["status"]; parts: string; approval: boolean }> {
  const component = componentId === null ? undefined : item.components.find((entry) => entry.id === componentId);
  if (component === undefined) {
    return { name: item.name, status: item.status, parts: listPhrase(partCounts(item.components)), approval: item.requiresApproval };
  }
  return { name: `${item.name}: ${component.id}`, status: component.status, parts: listPhrase(partCounts([component])), approval: component.requiresApproval };
}

/**
 * What a link asks for, before anything changes: a link from any web page
 * opens this, so the person always decides here. It never uninstalls; a
 * package already here offers to show it instead.
 */
export function LinkInstallDialog({
  request,
  profiles,
  onCancel,
  onInstall,
  onShow
}: Readonly<{ request: LinkRequest | null; profiles: readonly AgentProfile[]; onCancel: () => void; onInstall: (target: LinkTarget) => void; onShow: (id: string) => void }>): JSX.Element {
  return (
    <Dialog.Root
      open={request !== null}
      onOpenChange={(open) => {
        if (!open) {
          onCancel();
        }
      }}
    >
      <Dialog.Content maxWidth="520px">
        {request === null ? null : request.target.kind === "item" ? (
          <ItemRequest target={request.target} item={request.target.item} apps={appsPhrase(appsFor(request.target.item, profiles, request.target.componentId))} onInstall={onInstall} onShow={onShow} />
        ) : (
          <BundleRequest target={request.target} apps={appsPhrase([...new Set(request.target.members.flatMap((member) => appsFor(member, profiles)))])} onInstall={onInstall} onShow={onShow} />
        )}
      </Dialog.Content>
    </Dialog.Root>
  );
}

function RequestActions({ primary, onPrimary }: Readonly<{ primary: string; onPrimary: () => void }>): JSX.Element {
  return (
    <div className="dialog-actions">
      <Dialog.Close>
        <Button variant="soft" color="gray">
          Cancel
        </Button>
      </Dialog.Close>
      <Button onClick={onPrimary}>{primary}</Button>
    </div>
  );
}

function ItemRequest({
  target,
  item,
  apps,
  onInstall,
  onShow
}: Readonly<{ target: LinkTarget; item: CatalogItem; apps: string; onInstall: (target: LinkTarget) => void; onShow: (id: string) => void }>): JSX.Element {
  const part = itemPart(item, target.kind === "item" ? target.componentId : null);
  const action = linkAction(part.status);
  const byline = item.marketplace === null ? item.sourceName : `${item.marketplace.publisher} · v${item.marketplace.version}`;
  if (action !== "install") {
    return (
      <>
        <Dialog.Title>{action === "installed" ? `${part.name} is already installed` : `${part.name} came from another source`}</Dialog.Title>
        <Dialog.Description size="2">{action === "installed" ? "Nothing to do. It's already in your AI apps." : "Agent Plugins leaves it alone, so there's nothing to install."}</Dialog.Description>
        <RequestActions
          primary="Show it"
          onPrimary={() => {
            onShow(item.id);
          }}
        />
      </>
    );
  }
  const unusable = unusableReason(item);
  if (unusable !== null) {
    return (
      <>
        <Dialog.Title>{part.name} can't be added here</Dialog.Title>
        <Dialog.Description size="2">{unusable}</Dialog.Description>
        <RequestActions
          primary="Show it"
          onPrimary={() => {
            onShow(item.id);
          }}
        />
      </>
    );
  }
  return (
    <>
      <Dialog.Title>Install {part.name}?</Dialog.Title>
      <Dialog.Description size="2">{byline}</Dialog.Description>
      <Text as="p" size="2" className="manage-section">
        Adds {part.parts.length === 0 ? "it" : part.parts} to {apps}.
      </Text>
      {part.approval ? (
        <Text as="p" color="gray" size="2">
          It includes a connector. You'll see what it runs before anything is installed.
        </Text>
      ) : null}
      <RequestActions
        primary="Install"
        onPrimary={() => {
          onInstall(target);
        }}
      />
    </>
  );
}

function BundleRequest({ target, apps, onInstall, onShow }: Readonly<{ target: LinkTarget; apps: string; onInstall: (target: LinkTarget) => void; onShow: (id: string) => void }>): JSX.Element | null {
  if (target.kind !== "bundle") {
    return null;
  }
  const { bundle } = target;
  const summary = bundleSummary(bundle, target.members);
  const count = bundle.members.length;
  return (
    <>
      <Dialog.Title>{summary.action === "installed" ? `${bundle.name} is already installed` : `Install ${bundle.name}?`}</Dialog.Title>
      <Dialog.Description size="2">
        {bundle.publisher} · {String(count)} skill{count === 1 ? "" : "s"}
      </Dialog.Description>
      <ul className="entry-list manage-section">
        {summary.members.map((item) => (
          <li key={item.id} className="skill-title-row">
            <Text size="2">{item.name}</Text>
            {linkAction(item.status) === "install" ? null : <Badge color={statusColor(item.status)}>{statusLabel(item.status)}</Badge>}
          </li>
        ))}
      </ul>
      {summary.missing > 0 ? (
        <Text as="p" color="gray" size="1">
          {String(summary.missing)} more {summary.missing === 1 ? "isn't" : "aren't"} available to you.
        </Text>
      ) : null}
      <Text as="p" size="2">
        {summary.action === "empty" ? "None of its skills are available to you yet." : summary.action === "installed" ? "Everything in it is already in your AI apps." : `Adds them to ${apps}.`}
      </Text>
      {summary.action === "install" || summary.action === "installRest" ? (
        <RequestActions
          primary={summary.action === "install" ? "Install all" : "Install the rest"}
          onPrimary={() => {
            onInstall(target);
          }}
        />
      ) : (
        <RequestActions
          primary="Show it"
          onPrimary={() => {
            onShow(bundle.id);
          }}
        />
      )}
    </>
  );
}
