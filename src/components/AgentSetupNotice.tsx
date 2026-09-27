import type { JSX } from "react";
import { Button, Callout } from "@radix-ui/themes";
import type { AgentProfile } from "../ipc/schemas";

export function AgentSetupNotice({ visible, onChoose }: Readonly<{ visible: boolean; onChoose: () => void }>): JSX.Element | null {
  if (!visible) {
    return null;
  }
  return (
    <Callout.Root className="app-callout" color="blue" role="status">
      <div className="callout-content">
        <Callout.Text>
          No supported AI app was found. Skills can't be installed until GitHub Copilot, Cursor, or Claude (Claude Code or Claude Desktop) is on this computer, or another supported app such as
          OpenCode, pi, Codex, ChatGPT, or Grok Build.
        </Callout.Text>
        <Button className="callout-action" size="1" onClick={onChoose}>
          View AI apps
        </Button>
      </div>
    </Callout.Root>
  );
}

/** Offers the skill tutorial for one detected app until it has run once or the person says not now. */
export function TutorialNotice({
  profile,
  running,
  onStart,
  onDismiss
}: Readonly<{ profile: AgentProfile | null; running: boolean; onStart: (profile: AgentProfile) => void; onDismiss: () => void }>): JSX.Element | null {
  if (profile === null) {
    return null;
  }
  const app = profile.displayName;
  return (
    <Callout.Root className="app-callout" color="blue" role="status">
      <div className="callout-content">
        <Callout.Text>
          New to skills? See one work in {app}: Agent Plugins adds a sample skill and opens {app} with a prompt that uses it.
        </Callout.Text>
        <div className="callout-actions">
          <Button
            className="callout-action"
            size="1"
            loading={running}
            disabled={running}
            onClick={() => {
              onStart(profile);
            }}
          >
            Show me
          </Button>
          <Button className="callout-action" size="1" variant="soft" disabled={running} onClick={onDismiss}>
            Not now
          </Button>
        </div>
      </div>
    </Callout.Root>
  );
}
