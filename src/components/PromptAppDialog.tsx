import { useState } from "react";
import type { JSX } from "react";
import { Button, Dialog, RadioGroup } from "@radix-ui/themes";
import type { PromptApp } from "../ipc/schemas";
import { returnFocus } from "../lib/returnFocus";

/** What the picked app is opened for: the skill tutorial, or making a new skill. */
export type PromptPurpose = "tutorial" | "create";

const COPY: Readonly<Record<PromptPurpose, Readonly<{ title: string; description: string }>>> = {
  tutorial: {
    title: "Try a skill",
    description: "Agent Plugins adds a sample skill, then opens the app you pick with a message that uses it. The skill walks you through making and sharing your own. Nothing is closed."
  },
  create: {
    title: "Create a skill",
    description: "Pick the AI app that will help you write the skill. It asks what the skill should do, writes it, and shares it on the marketplace when you're ready."
  }
};

type PickerProps = Readonly<{ apps: readonly PromptApp[]; running: boolean; onOpen: (purpose: PromptPurpose, app: PromptApp) => void }>;

/** Asks which detected app to open with a prompt, even when there is only one. */
export function PromptAppDialog({ purpose, onClose, ...picker }: PickerProps & Readonly<{ purpose: PromptPurpose | null; onClose: () => void }>): JSX.Element {
  return (
    <Dialog.Root
      open={purpose !== null}
      onOpenChange={(open) => {
        // The app is already being opened; closing now would hide how it went.
        if (!open && !picker.running) {
          onClose();
        }
      }}
    >
      <Dialog.Content maxWidth="440px" {...returnFocus}>
        <Picker purpose={purpose ?? "tutorial"} {...picker} />
      </Dialog.Content>
    </Dialog.Root>
  );
}

/** Mounts each time the dialog opens, so the choice starts at the first app and the title holds while the dialog fades out. */
function Picker({ purpose, apps, running, onOpen }: PickerProps & Readonly<{ purpose: PromptPurpose }>): JSX.Element {
  const [shown] = useState(purpose);
  const [chosen, setChosen] = useState<string | null>(null);
  // The first app until the person picks another, and again if the picked one disappears on a refresh.
  const selected = apps.find((app) => app.id === chosen) ?? apps[0] ?? null;
  return (
    <>
      <Dialog.Title>{COPY[shown].title}</Dialog.Title>
      <Dialog.Description size="2">{COPY[shown].description}</Dialog.Description>
      <RadioGroup.Root className="manage-section" value={selected?.id ?? ""} disabled={running} onValueChange={setChosen}>
        {apps.map((app) => (
          <RadioGroup.Item key={app.id} value={app.id}>
            {app.label}
          </RadioGroup.Item>
        ))}
      </RadioGroup.Root>
      <div className="dialog-actions">
        <Dialog.Close>
          <Button variant="soft" color="gray" disabled={running}>
            Cancel
          </Button>
        </Dialog.Close>
        <Button
          loading={running}
          disabled={running || selected === null}
          onClick={() => {
            if (selected !== null) {
              onOpen(shown, selected);
            }
          }}
        >
          Open
        </Button>
      </div>
    </>
  );
}
