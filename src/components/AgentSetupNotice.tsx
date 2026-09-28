import type { JSX } from "react";
import { Button, Callout } from "@radix-ui/themes";

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

/** Offers the skill tutorial until it has run once or the person says not now; **Show me** asks which app. */
export function TutorialNotice({ visible, onStart, onDismiss }: Readonly<{ visible: boolean; onStart: () => void; onDismiss: () => void }>): JSX.Element | null {
  if (!visible) {
    return null;
  }
  return (
    <Callout.Root className="app-callout" color="blue" role="status">
      <div className="callout-content">
        <Callout.Text>New to skills? Agent Plugins adds a sample skill and opens one of your AI apps with it, which walks you through making a skill of your own and sharing it.</Callout.Text>
        <div className="callout-actions">
          <Button className="callout-action" size="1" onClick={onStart}>
            Show me
          </Button>
          <Button className="callout-action" size="1" variant="soft" onClick={onDismiss}>
            Not now
          </Button>
        </div>
      </div>
    </Callout.Root>
  );
}
