import { Component } from "react";
import type { ErrorInfo, JSX, ReactNode } from "react";
import { Button, Callout, Heading, Text } from "@radix-ui/themes";
import { ErrorMessage } from "./Notice";

type BoundaryState = Readonly<{ failure: string | null }>;

/** A rendering bug should cost the person a reload, not a blank window. */
export class ErrorBoundary extends Component<Readonly<{ children: ReactNode }>, BoundaryState> {
  public override state: BoundaryState = { failure: null };

  public static getDerivedStateFromError(error: unknown): BoundaryState {
    return { failure: error instanceof Error ? (error.stack ?? error.message) : String(error) };
  }

  public override componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error("Agent Plugins window crashed", error, info.componentStack);
  }

  public override render(): ReactNode {
    if (this.state.failure === null) {
      return this.props.children;
    }
    return <CrashPanel detail={this.state.failure} />;
  }
}

function CrashPanel({ detail }: Readonly<{ detail: string }>): JSX.Element {
  return (
    <main className="app-shell">
      <div className="load-failed">
        <Heading as="h1" size="5">
          Something went wrong
        </Heading>
        <Text as="p" color="gray">
          This window hit a problem it could not recover from. Your installed packages are not affected. Reload to continue.
        </Text>
        <Callout.Root className="app-callout" color="red">
          <ErrorMessage summary="The window stopped working." detail={detail} />
        </Callout.Root>
        <Button
          onClick={() => {
            window.location.reload();
          }}
        >
          Reload
        </Button>
      </div>
    </main>
  );
}
