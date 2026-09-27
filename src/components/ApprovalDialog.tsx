import { useEffect, useState } from "react";
import type { JSX } from "react";
import { Badge, Button, Code, Dialog, Text, TextField } from "@radix-ui/themes";
import type { CatalogItem, Connector } from "../ipc/schemas";
import { listPhrase } from "../lib/marketplace";
import { returnFocus } from "../lib/returnFocus";

/** A package (or one part of it) whose connectors the person is asked about. */
export type ApprovalEntry = Readonly<{ item: CatalogItem; componentId: string | null }>;

/**
 * `install` asks before connectors are installed; `settings` only fills in
 * what installed ones need, such as an API key.
 */
export type ApprovalRequest = Readonly<{ entries: readonly ApprovalEntry[]; mode: "install" | "settings" }>;

/** Values typed for environment variables, by name. Empty ones are left out. */
export type ConnectorSettings = Readonly<Record<string, string>>;

function connectorsOf({ item, componentId }: ApprovalEntry): readonly Connector[] {
  return item.connectors.filter((connector) => componentId === null || connector.componentId === componentId);
}

/** Every variable the request's connectors read, each once, missing ones first. */
function variables(request: ApprovalRequest): readonly Readonly<{ name: string; missing: boolean }>[] {
  const connectors = request.entries.flatMap(connectorsOf);
  const missing = new Set(connectors.flatMap((connector) => connector.missingEnvironment));
  const names = [...new Set(connectors.flatMap((connector) => connector.environment))];
  return names.map((name) => ({ name, missing: missing.has(name) })).sort((left, right) => Number(right.missing) - Number(left.missing));
}

export function ApprovalDialog({ request, onResolve }: Readonly<{ request: ApprovalRequest | null; onResolve: (settings: ConnectorSettings | null) => void }>): JSX.Element {
  const [values, setValues] = useState<Record<string, string>>({});
  useEffect(() => {
    setValues({});
  }, [request]);
  const install = request?.mode !== "settings";
  const vars = request === null ? [] : variables(request);
  const typed = Object.fromEntries(Object.entries(values).filter(([, value]) => value.trim().length > 0));
  return (
    <Dialog.Root
      open={request !== null}
      onOpenChange={(open) => {
        if (!open) {
          onResolve(null);
        }
      }}
    >
      <Dialog.Content maxWidth="600px" {...returnFocus}>
        <Dialog.Title>{install ? "Allow connector?" : "Connector settings"}</Dialog.Title>
        <Dialog.Description size="2">
          {install
            ? "A connector is a program on this computer or an online service that your AI apps use. Allow it only if you trust who published it."
            : "Values are saved for your Windows account, where your AI apps read them. Restart the apps afterwards."}
        </Dialog.Description>
        {(request?.entries ?? []).map((entry) => (
          <EntryFacts key={`${entry.item.id}:${entry.componentId ?? ""}`} entry={entry} />
        ))}
        {vars.length === 0 ? null : (
          <div className="manage-section">
            <Text as="p" size="2" weight="medium">
              Settings it needs
            </Text>
            {vars.map(({ name, missing }) => (
              <label key={name} className="approval-variable">
                <Text size="2">
                  <Code>{name}</Code> {missing ? "isn't set yet. Paste the value from the publisher's instructions, or add it later." : "is already set. Type a new value only to change it."}
                </Text>
                <TextField.Root
                  type="password"
                  autoComplete="off"
                  aria-label={name}
                  value={values[name] ?? ""}
                  onChange={(event) => {
                    const value = event.currentTarget.value;
                    setValues((current) => ({ ...current, [name]: value }));
                  }}
                />
              </label>
            ))}
          </div>
        )}
        <div className="dialog-actions">
          <Dialog.Close>
            <Button variant="soft" color="gray">
              Cancel
            </Button>
          </Dialog.Close>
          <Button
            onClick={() => {
              onResolve(typed);
            }}
          >
            {install ? "Allow and install" : "Save"}
          </Button>
        </div>
      </Dialog.Content>
    </Dialog.Root>
  );
}

function EntryFacts({ entry }: Readonly<{ entry: ApprovalEntry }>): JSX.Element {
  const meta = entry.item.marketplace;
  return (
    <section className="manage-section approval-entry">
      <div className="skill-title-row">
        <Text size="3" weight="bold">
          {entry.item.name}
        </Text>
        {meta?.mcpApproved === true ? <Badge color="green">Checked by IT</Badge> : null}
        {meta?.mcpApproved === false ? <Badge color="amber">Not checked by IT</Badge> : null}
      </div>
      {meta === null ? null : (
        <Text as="p" color="gray" size="1">
          From {meta.publisher}
        </Text>
      )}
      {connectorsOf(entry).map((connector) => (
        <div key={connector.componentId} className="approval-connector">
          <Text as="p" size="2">
            <strong>{connector.name}</strong>
            {connector.changed ? " (changed in this update)" : ""}: {connector.summary}
          </Text>
          <Text as="p" color="gray" size="1">
            {connector.apps.length === 0 ? "None of the AI apps on this computer can use it." : `Goes to ${listPhrase(connector.apps)}.`}
          </Text>
          {connector.missingProgram === null ? null : (
            <Text as="p" color="amber" size="1">
              It needs {connector.missingProgram}, which isn't on this computer. Install it, or ask IT to, before the connector will work.
            </Text>
          )}
          <Code size="1" variant="ghost" className="approval-detail">
            {connector.detail}
          </Code>
        </div>
      ))}
    </section>
  );
}
