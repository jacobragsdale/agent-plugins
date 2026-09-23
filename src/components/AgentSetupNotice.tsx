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
          No supported AI app was found. Packages can't be installed until Claude Desktop, ChatGPT, Microsoft 365 Copilot, or a coding tool such as Cursor, Claude Code, Codex, OpenCode, Grok Build, or
          GitHub Copilot is on this machine.
        </Callout.Text>
        <Button className="callout-action" size="1" onClick={onChoose}>
          View AI apps
        </Button>
      </div>
    </Callout.Root>
  );
}
