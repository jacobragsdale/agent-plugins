import type { JSX } from "react";
import { Badge, Button, Dialog, Text } from "@radix-ui/themes";
import type { AppIdentity, CheckStatus, PreflightCheck, PreflightReport } from "../ipc/schemas";

const GROUP_TITLES: Readonly<Record<string, string>> = { host: "Windows host", auth: "Authentication", server: "Marketplace server", agents: "Agents", dependencies: "Dependencies" };

const GROUP_ORDER = ["auth", "server", "host", "agents", "dependencies"];

export function statusColor(status: CheckStatus): "green" | "amber" | "red" | "gray" {
  switch (status) {
    case "ok":
      return "green";
    case "warn":
      return "amber";
    case "fail":
      return "red";
    case "skipped":
      return "gray";
  }
}

export function statusLabel(status: CheckStatus): string {
  switch (status) {
    case "ok":
      return "OK";
    case "warn":
      return "Warning";
    case "fail":
      return "Failed";
    case "skipped":
      return "Skipped";
  }
}

export function overallStatus(report: PreflightReport | null): CheckStatus {
  if (report === null) {
    return "skipped";
  }
  if (report.checks.some((check) => check.status === "fail")) {
    return "fail";
  }
  if (report.checks.some((check) => check.status === "warn")) {
    return "warn";
  }
  return "ok";
}

function remediationText(check: PreflightCheck): string | null {
  if (check.remediation === null) {
    return null;
  }
  switch (check.remediation.kind) {
    case "autoFixed":
      return "Fixed automatically.";
    case "manual":
      return check.remediation.text;
    case "action":
      return null;
  }
}

function actionLabel(action: string): string {
  switch (action) {
    case "update":
      return "Update Agent Plugins";
    case "sync":
      return "Refresh";
    case "showAgents":
      return "Detected Agents";
    case "showDrift":
      return "Show modified packages";
    case "installPublishSkill":
      return "Install the publish skill";
    default:
      return action;
  }
}

export function SystemStatusDialog({
  open,
  report,
  identity,
  marketplaceUrl,
  running,
  onOpenChange,
  onRerun,
  onAction
}: Readonly<{
  open: boolean;
  report: PreflightReport | null;
  identity: AppIdentity | null;
  marketplaceUrl: string | null;
  running: boolean;
  onOpenChange: (open: boolean) => void;
  onRerun: () => void;
  onAction: (action: string) => void;
}>): JSX.Element {
  const groups = new Map<string, PreflightCheck[]>();
  for (const check of report?.checks ?? []) {
    const list = groups.get(check.group) ?? [];
    list.push(check);
    groups.set(check.group, list);
  }
  const orderedGroups = [...groups.keys()].sort((left, right) => GROUP_ORDER.indexOf(left) - GROUP_ORDER.indexOf(right));
  const ran = report === null ? "Not run yet" : `${new Date(report.startedAtEpochSeconds * 1000).toLocaleString()} · ${String(report.durationMillis)} ms`;
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content maxWidth="760px">
        <Dialog.Title>System status</Dialog.Title>
        <Dialog.Description>
          {identity === null
            ? `Not signed in to the marketplace${marketplaceUrl === null ? "" : ` at ${marketplaceUrl}`}.`
            : `Signed in as ${identity.account} (namespace ${identity.namespace}) via ${identity.authMode}. Usage is recorded under this identity.`}
        </Dialog.Description>
        <div className="status-meta">
          <Text color="gray" size="1">
            Last preflight: {ran}
          </Text>
          <Button size="1" variant="soft" loading={running} disabled={running} onClick={onRerun}>
            Run diagnostics
          </Button>
        </div>
        {report?.blocked === true ? (
          <Text as="p" color="red" size="2">
            A blocking check failed. Agent Plugins will not install or sync until it is resolved.
          </Text>
        ) : null}
        <div className="status-groups">
          {orderedGroups.map((group) => (
            <section key={group} className="status-group">
              <Text as="p" weight="bold" size="2">
                {GROUP_TITLES[group] ?? group}
              </Text>
              <ul className="status-list">
                {(groups.get(group) ?? []).map((check) => {
                  const remediation = remediationText(check);
                  return (
                    <li key={check.id} className="status-row">
                      <Badge color={statusColor(check.status)} variant={check.status === "skipped" ? "soft" : "solid"}>
                        {statusLabel(check.status)}
                      </Badge>
                      <div className="status-copy">
                        <Text as="p" size="2">
                          {check.title}
                          {check.blocking ? " · blocking" : ""}
                        </Text>
                        {check.detail.length === 0 ? null : (
                          <Text as="p" color="gray" size="1">
                            {check.detail}
                          </Text>
                        )}
                        {remediation === null ? null : (
                          <Text as="p" color="amber" size="1">
                            {remediation}
                          </Text>
                        )}
                        {check.remediation?.kind === "action" ? (
                          <Button
                            size="1"
                            variant="soft"
                            onClick={() => {
                              onAction(check.remediation?.kind === "action" ? check.remediation.action : "");
                            }}
                          >
                            {actionLabel(check.remediation.action)}
                          </Button>
                        ) : null}
                      </div>
                    </li>
                  );
                })}
              </ul>
            </section>
          ))}
        </div>
        <div className="dialog-actions">
          <Dialog.Close>
            <Button variant="soft">Close</Button>
          </Dialog.Close>
        </div>
      </Dialog.Content>
    </Dialog.Root>
  );
}
