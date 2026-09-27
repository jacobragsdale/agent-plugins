import { useEffect, useState } from "react";
import type { JSX } from "react";
import { Button, Checkbox, Dialog, Text } from "@radix-ui/themes";
import type { AgentProfile, CatalogItem } from "../ipc/schemas";
import { returnFocus } from "../lib/returnFocus";

/** One connector whose apps the person is choosing. */
export type AppsRequest = Readonly<{ item: CatalogItem; componentId: string }>;

/** The apps here that could take the connector: those it goes to now, and those it was kept out of. */
function candidates(request: AppsRequest, profiles: readonly AgentProfile[]): readonly AgentProfile[] {
  const component = request.item.components.find((entry) => entry.id === request.componentId);
  const excluded = component?.excludedApps ?? [];
  const usable = new Set(
    request.item.compatibility
      .filter((report) => report.componentId === request.componentId && report.capability.level !== "unsupported" && report.capability.level !== "blocked")
      .map((report) => report.targetId)
  );
  return profiles.filter((profile) => profile.detected && (usable.has(profile.targetId) || excluded.includes(profile.targetId)));
}

export function AppsDialog({
  request,
  profiles,
  onSave,
  onClose
}: Readonly<{
  request: AppsRequest | null;
  profiles: readonly AgentProfile[];
  /** `excluded` are target ids; `added` says an app it was kept out of gets it again, which needs approval. */
  onSave: (request: AppsRequest, excluded: readonly string[], added: boolean) => void;
  onClose: () => void;
}>): JSX.Element {
  const apps = request === null ? [] : candidates(request, profiles);
  const excludedNow = request?.item.components.find((entry) => entry.id === request.componentId)?.excludedApps ?? [];
  const [chosen, setChosen] = useState<ReadonlySet<string>>(new Set());
  useEffect(() => {
    setChosen(new Set(apps.map((profile) => profile.targetId).filter((target) => !excludedNow.includes(target))));
    // Reset only when a new request opens, not on every render of the same one.
  }, [request]);
  // An app that isn't here right now stays excluded; only the ones shown can change.
  const shown = new Set<string>(apps.map((profile) => profile.targetId));
  const excluded = [...excludedNow.filter((target) => !shown.has(target)), ...apps.map((profile) => profile.targetId).filter((target) => !chosen.has(target))];
  return (
    <Dialog.Root
      open={request !== null}
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
    >
      <Dialog.Content maxWidth="440px" {...returnFocus}>
        <Dialog.Title>Which apps use it?</Dialog.Title>
        <Dialog.Description size="2">Agent Plugins keeps this connector out of the apps you clear, and leaves it there on later updates.</Dialog.Description>
        <div className="manage-section">
          {apps.map((profile) => (
            <Text as="label" size="2" key={profile.targetId} className="apps-choice">
              <Checkbox
                checked={chosen.has(profile.targetId)}
                onCheckedChange={(checked) => {
                  setChosen((current) => {
                    const next = new Set(current);
                    if (checked === true) {
                      next.add(profile.targetId);
                    } else {
                      next.delete(profile.targetId);
                    }
                    return next;
                  });
                }}
              />{" "}
              {profile.displayName}
            </Text>
          ))}
        </div>
        {chosen.size === 0 ? (
          <Text as="p" color="amber" size="1">
            Keep at least one app, or uninstall the connector instead.
          </Text>
        ) : null}
        <div className="dialog-actions">
          <Dialog.Close>
            <Button variant="soft" color="gray">
              Cancel
            </Button>
          </Dialog.Close>
          <Button
            disabled={request === null || chosen.size === 0}
            onClick={() => {
              // Nothing changed means nothing to write, and nothing to restart.
              const unchanged = excluded.length === excludedNow.length && excluded.every((target) => excludedNow.includes(target));
              if (unchanged) {
                onClose();
              } else if (request !== null) {
                onSave(
                  request,
                  excluded,
                  excludedNow.some((target) => chosen.has(target))
                );
              }
            }}
          >
            Save
          </Button>
        </div>
      </Dialog.Content>
    </Dialog.Root>
  );
}
