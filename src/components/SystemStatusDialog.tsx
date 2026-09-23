import type { JSX, ReactNode } from "react";
import { Button, Dialog, Text } from "@radix-ui/themes";
import type { AgentProfile, AppIdentity, CheckStatus, PreflightCheck, PreflightReport } from "../ipc/schemas";
import { detectionUnsure } from "../lib/status";

const GROUP_TITLES: Readonly<Record<string, string>> = { host: "Windows host", auth: "Authentication", server: "Marketplace server", agents: "Agents", dependencies: "Dependencies" };

const GROUP_ORDER = ["auth", "server", "host", "agents", "dependencies"];

/** Ordered so the first check that ran supplies the summary line when nothing failed. */
const AUTH_CHECKS = ["auth.identity", "auth.ticket", "host.domain"];
const SERVER_CHECKS = ["server.health", "host.serverUrl", "host.dns", "host.tls", "server.clientVersion", "host.clock", "host.proxy"];

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

export type DiagnosticsResult = Readonly<{ tone: "green" | "amber" | "red"; message: string }>;

/** What to tell someone after they press Run diagnostics: the run happened, and what it found. */
export function diagnosticsResult(report: PreflightReport): DiagnosticsResult {
  const failed = seriousProblems(report).length;
  if (failed > 0) {
    return { tone: "red", message: `Finished: ${String(failed)} check${failed === 1 ? "" : "s"} failed.` };
  }
  const warnings = report.checks.filter((check) => check.status === "warn").length;
  if (warnings > 0) {
    return { tone: "amber", message: `Finished: nothing failed, ${String(warnings)} warning${warnings === 1 ? "" : "s"}.` };
  }
  return { tone: "green", message: `Finished: all ${String(report.checks.length)} checks passed.` };
}

export function diagnosticsFailure(text: string): DiagnosticsResult {
  return { tone: "red", message: `Diagnostics could not run. ${text}` };
}

/** Only failures are worth interrupting someone over. Warnings stay inside the check list. */
export function seriousProblems(report: PreflightReport | null): readonly PreflightCheck[] {
  return (report?.checks ?? []).filter((check) => check.status === "fail");
}

/** The failure to show for one summary row, and the line to show when nothing failed. */
function summarize(report: PreflightReport | null, ids: readonly string[], fallback: string): Readonly<{ problem: PreflightCheck | null; detail: string }> {
  const checks = ids.flatMap((id) => (report?.checks ?? []).filter((check) => check.id === id));
  const problem = checks.find((check) => check.status === "fail") ?? null;
  if (problem !== null) {
    return { problem, detail: problem.detail };
  }
  const primary = checks.at(0);
  return { problem: null, detail: primary === undefined || primary.detail.length === 0 ? fallback : primary.detail };
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
    case "showDrift":
      return "Show modified packages";
    case "installPublishSkill":
      return "Install the publish skill";
    default:
      return action;
  }
}

function agentSummary(profile: AgentProfile): string {
  return profile.detectedVersion === null ? profile.displayName : `${profile.displayName} ${profile.detectedVersion}`;
}

function Remedy({ check, onAction }: Readonly<{ check: PreflightCheck; onAction: (action: string) => void }>): JSX.Element | null {
  const text = remediationText(check);
  const action = check.remediation?.kind === "action" ? check.remediation.action : null;
  if (text === null && action === null) {
    return null;
  }
  return (
    <>
      {text === null ? null : (
        <Text as="p" color="gray" size="1">
          {text}
        </Text>
      )}
      {action === null ? null : (
        <Button
          size="1"
          variant="soft"
          onClick={() => {
            onAction(action);
          }}
        >
          {actionLabel(action)}
        </Button>
      )}
    </>
  );
}

function SummaryRow({
  label,
  detail,
  problem,
  onAction,
  children
}: Readonly<{ label: string; detail: string; problem: PreflightCheck | null; onAction: (action: string) => void; children?: ReactNode }>): JSX.Element {
  return (
    <div className="status-summary-row">
      <Text as="p" className="status-summary-label" size="2" weight="medium">
        {label}
      </Text>
      <div className="status-summary-copy">
        <Text as="p" {...(problem === null ? {} : { color: "red" as const })} size="2">
          {detail}
        </Text>
        {problem === null ? null : <Remedy check={problem} onAction={onAction} />}
        {children}
      </div>
    </div>
  );
}

function StatusSummary({
  report,
  identity,
  marketplaceUrl,
  profiles,
  onAction
}: Readonly<{ report: PreflightReport | null; identity: AppIdentity | null; marketplaceUrl: string | null; profiles: readonly AgentProfile[]; onAction: (action: string) => void }>): JSX.Element {
  const auth = summarize(report, AUTH_CHECKS, identity === null ? "Not signed in." : `${identity.account} (namespace ${identity.namespace})`);
  const server = summarize(report, SERVER_CHECKS, marketplaceUrl ?? "No marketplace is configured.");
  const elsewhere = seriousProblems(report).filter((check) => !AUTH_CHECKS.includes(check.id) && !SERVER_CHECKS.includes(check.id));
  const detected = profiles.filter((profile) => profile.detected);
  const unsure = profiles.filter(detectionUnsure);
  return (
    <div className="status-summary">
      <SummaryRow label="Windows sign-in" detail={auth.detail} problem={auth.problem} onAction={onAction}>
        {report === null || auth.problem !== null ? null : (
          <Text as="p" color="gray" size="1">
            {report.authMode}
          </Text>
        )}
      </SummaryRow>
      <SummaryRow label="Marketplace server" detail={server.detail} problem={server.problem} onAction={onAction} />
      <SummaryRow
        label="Agents"
        detail={
          detected.length === 0 && unsure.length > 0
            ? `Couldn't check ${unsure.map((profile) => profile.displayName).join(", ")} just now. Agent Plugins will check again.`
            : detected.length === 0
              ? "No supported AI app was found. Install Claude Desktop, ChatGPT, or Microsoft 365 Copilot, or a coding tool such as Cursor, Claude Code, Codex, OpenCode, Grok Build, or GitHub Copilot, then refresh."
              : detected.map(agentSummary).join(", ")
        }
        problem={null}
        onAction={onAction}
      >
        {detected.length === 0 ? null : (
          <Text as="p" color="gray" size="1">
            Skills go to {[...new Set(detected.map((profile) => profile.skillDirectory))].join(" and ")}.
          </Text>
        )}
      </SummaryRow>
      {elsewhere.map((check) => (
        <SummaryRow key={check.id} label={check.title} detail={check.detail} problem={check} onAction={onAction} />
      ))}
    </div>
  );
}

function AgentDetails({ profiles }: Readonly<{ profiles: readonly AgentProfile[] }>): JSX.Element {
  return (
    <div className="status-agents">
      {profiles.map((profile) => (
        <div key={profile.targetId} className="status-agent">
          <Text as="p" size="2">
            {agentSummary(profile)}
            {profile.detected ? "" : ` — ${detectionUnsure(profile) ? "couldn't check just now" : "not installed"}`}
          </Text>
          {profile.detected ? (
            <Text as="p" color="gray" size="1">
              Skills {profile.skillDirectoryShared ? "shared at" : "at"} {profile.skillDirectory}. Verify: {profile.verificationGuidance} Reload: {profile.reloadGuidance}
            </Text>
          ) : null}
          {profile.detectionMessage === null ? null : (
            <Text as="p" color="amber" size="1">
              {profile.detectionMessage}
            </Text>
          )}
        </div>
      ))}
    </div>
  );
}

function headline(report: PreflightReport | null, problems: readonly PreflightCheck[]): string {
  if (report === null) {
    return "Diagnostics have not run on this machine yet.";
  }
  if (problems.length === 0) {
    return "Windows sign-in, the marketplace server, and the agents on this machine are working.";
  }
  const blocked = report.blocked ? " Agent Plugins can't install or update packages until this is fixed." : "";
  return `${String(problems.length)} check${problems.length === 1 ? "" : "s"} failed.${blocked}`;
}

function checksSummary(report: PreflightReport | null): string {
  const warnings = (report?.checks ?? []).filter((check) => check.status === "warn").length;
  const total = report?.checks.length ?? 0;
  return warnings === 0 ? `All checks (${String(total)})` : `All checks (${String(total)}, ${String(warnings)} warning${warnings === 1 ? "" : "s"})`;
}

export function SystemStatusDialog({
  open,
  report,
  identity,
  marketplaceUrl,
  profiles,
  running,
  diagnostics,
  onOpenChange,
  onRerun,
  onAction
}: Readonly<{
  open: boolean;
  report: PreflightReport | null;
  identity: AppIdentity | null;
  marketplaceUrl: string | null;
  profiles: readonly AgentProfile[];
  running: boolean;
  diagnostics: DiagnosticsResult | null;
  onOpenChange: (open: boolean) => void;
  onRerun: () => void;
  onAction: (action: string) => void;
}>): JSX.Element {
  const ran = report === null ? "not run yet" : new Date(report.startedAtEpochSeconds * 1000).toLocaleString();
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content maxWidth="640px">
        <Dialog.Title>System status</Dialog.Title>
        <Dialog.Description size="2">{headline(report, seriousProblems(report))}</Dialog.Description>
        <StatusSummary report={report} identity={identity} marketplaceUrl={marketplaceUrl} profiles={profiles} onAction={onAction} />
        <details className="status-details">
          <summary>Agent details</summary>
          <AgentDetails profiles={profiles} />
        </details>
        <details className="status-details">
          <summary>{checksSummary(report)}</summary>
          <CheckList report={report} onAction={onAction} />
        </details>
        <div className="dialog-actions status-footer">
          <div className="status-footer-meta">
            <Text color="gray" size="1">
              Last checked {ran}
            </Text>
            {running ? (
              <Text color="gray" size="1">
                Running checks…
              </Text>
            ) : null}
            {running || diagnostics === null ? null : (
              <Text color={diagnostics.tone} size="1">
                {diagnostics.message}
              </Text>
            )}
          </div>
          <div className="status-footer-actions">
            <Button variant="soft" loading={running} disabled={running} onClick={onRerun}>
              Run diagnostics
            </Button>
            <Dialog.Close>
              <Button>Close</Button>
            </Dialog.Close>
          </div>
        </div>
      </Dialog.Content>
    </Dialog.Root>
  );
}

function CheckList({ report, onAction }: Readonly<{ report: PreflightReport | null; onAction: (action: string) => void }>): JSX.Element {
  const groups = new Map<string, PreflightCheck[]>();
  for (const check of report?.checks ?? []) {
    const list = groups.get(check.group) ?? [];
    list.push(check);
    groups.set(check.group, list);
  }
  const orderedGroups = [...groups.keys()].sort((left, right) => GROUP_ORDER.indexOf(left) - GROUP_ORDER.indexOf(right));
  return (
    <div className="status-groups">
      {orderedGroups.map((group) => (
        <section key={group} className="status-group">
          <Text as="p" weight="bold" size="2">
            {GROUP_TITLES[group] ?? group}
          </Text>
          <ul className="status-list">
            {(groups.get(group) ?? []).map((check) => (
              <li key={check.id} className="status-row">
                <Text as="span" className="status-mark" color={statusColor(check.status)} size="1" weight="medium">
                  {statusLabel(check.status)}
                </Text>
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
                  <Remedy check={check} onAction={onAction} />
                </div>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
