import type { JSX, ReactNode } from "react";
import { Button, Callout } from "@radix-ui/themes";
import type { AppError } from "../ipc/client";
import type { ReportNotice } from "../lib/status";

/**
 * Something the person should know that is not a failure: where a backup
 * went, that there was nothing to do. `caution` (amber) marks a partial
 * success, such as a package that skipped one AI app.
 */
export type InfoNotice = Readonly<{ text: string; folder: string | null; caution: boolean }>;

/** A plain summary, with the raw text one click away for whoever has to fix it. */
export function ErrorMessage({ summary, detail }: Readonly<{ summary: string; detail: string | null }>): JSX.Element {
  return (
    <div className="notice-copy">
      <Callout.Text>{summary}</Callout.Text>
      {detail === null ? null : (
        <details className="error-details">
          <summary>Details</summary>
          <pre>{detail}</pre>
        </details>
      )}
    </div>
  );
}

function NoticeCallout({ color, role, children, actions }: Readonly<{ color: "red" | "amber" | "blue" | "green"; role: "alert" | "status"; children: ReactNode; actions: ReactNode }>): JSX.Element {
  return (
    <Callout.Root className="app-callout" color={color} role={role}>
      <div className="callout-content">
        {children}
        <div className="callout-actions">{actions}</div>
      </div>
    </Callout.Root>
  );
}

function DismissButton({ onClick }: Readonly<{ onClick: () => void }>): JSX.Element {
  return (
    <Button className="callout-action" size="1" variant="soft" onClick={onClick}>
      Dismiss
    </Button>
  );
}

export function Notices({
  error,
  info,
  report,
  onDismissError,
  onDismissInfo,
  onDismissReport,
  onOpenFolder
}: Readonly<{
  error: AppError | null;
  info: InfoNotice | null;
  report: ReportNotice | null;
  onDismissError: () => void;
  onDismissInfo: () => void;
  onDismissReport: () => void;
  onOpenFolder: (path: string) => void;
}>): JSX.Element | null {
  if (error === null && info === null && report === null) {
    return null;
  }
  const folder = info?.folder ?? null;
  return (
    <>
      {error === null ? null : (
        <NoticeCallout color="red" role="alert" actions={<DismissButton onClick={onDismissError} />}>
          <ErrorMessage summary={error.summary} detail={error.detail} />
        </NoticeCallout>
      )}
      {info === null ? null : (
        <NoticeCallout
          color={info.caution ? "amber" : "blue"}
          role="status"
          actions={
            <>
              {folder === null ? null : (
                <Button
                  className="callout-action"
                  size="1"
                  onClick={() => {
                    onOpenFolder(folder);
                  }}
                >
                  Open folder
                </Button>
              )}
              <DismissButton onClick={onDismissInfo} />
            </>
          }
        >
          <Callout.Text>{info.text}</Callout.Text>
        </NoticeCallout>
      )}
      {report === null ? null : (
        <NoticeCallout color={report.failed ? "amber" : "green"} role="status" actions={<DismissButton onClick={onDismissReport} />}>
          <ErrorMessage summary={report.text} detail={report.detail} />
        </NoticeCallout>
      )}
    </>
  );
}

/** The one calm offline or degraded banner, with a manual retry beside the automatic one. */
export function OfflineBanner({ text, checking, onTryNow }: Readonly<{ text: string | null; checking: boolean; onTryNow: () => void }>): JSX.Element | null {
  if (text === null) {
    return null;
  }
  return (
    <NoticeCallout
      color="amber"
      role="status"
      actions={
        <Button className="callout-action" size="1" variant="soft" loading={checking} disabled={checking} onClick={onTryNow}>
          Try now
        </Button>
      }
    >
      <Callout.Text>{text}</Callout.Text>
    </NoticeCallout>
  );
}
