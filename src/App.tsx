import { startTransition, useCallback, useEffect, useMemo, useState } from "react";
import type { JSX } from "react";
import { Button, Callout, Heading, Spinner, Text } from "@radix-ui/themes";
import { listen } from "@tauri-apps/api/event";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { AgentSetupNotice, TutorialNotice } from "./components/AgentSetupNotice";
import { ManageSourcesDialog } from "./components/ManageSourcesDialog";
import { ErrorMessage, Notices, OfflineBanner } from "./components/Notice";
import type { InfoNotice } from "./components/Notice";
import { SourceGroup } from "./components/SourceGroup";
import type { SourceAction } from "./components/SourceGroup";
import { CatalogToolbar, CreateSkillButton, StatusButton, SyncMeta } from "./components/CatalogToolbar";
import { diagnosticsFailure, diagnosticsResult, seriousProblems, SystemStatusDialog } from "./components/SystemStatusDialog";
import { errorResponse, explainAfterRetry, invokeParsed, SCHEDULED_SYNC_EVENT, toAppError, withRetry } from "./ipc/client";
import type { AppError } from "./ipc/client";
import {
  appStateSchema,
  bulkPlanSchema,
  bulkResultSchema,
  cachedStateSchema,
  operationOutcomeSchema,
  preflightReportSchema,
  preparedSourceSchema,
  scheduledSyncSchema,
  sourceRemovalPlanSchema,
  unitSchema
} from "./ipc/schemas";
import type { DiagnosticsResult } from "./components/SystemStatusDialog";
import type { AgentProfile, AppIdentity, AppState, BulkAction, CatalogItem, ListedSource, PreflightCheck, PreflightReport, RepositoryState, SourceState } from "./ipc/schemas";
import { catalogBody, headerProblems, isChecking, isOffline, lastCheckedLabel, noMatchesText, offlineBanner } from "./lib/connectivity";
import {
  bulkLabels,
  failuresError,
  hasDetectedAgent,
  itemCommand,
  itemCommandArgs,
  outcomeNotice,
  reportNotice,
  reviewApproval,
  reviewBulk,
  reviewBulkApproval,
  reviewReset,
  reviewSourceRemoval,
  reviewTutorial
} from "./lib/status";
import type { ReportNotice } from "./lib/status";
import "./App.css";

/** Errors remember whether syncing raised them: a later sync may clear its own errors, never an action error the person has not read. */
type ShownError = AppError & Readonly<{ fromSync: boolean }>;

const SYNC_FAILED = "Couldn't check your sources for updates.";
const LOAD_FAILED = "Couldn't load your packages.";

function fromAction(error: AppError): ShownError {
  return { ...error, fromSync: false };
}

function fromSync(error: AppError): ShownError {
  return { ...error, fromSync: true };
}

/** What a sync leaves on screen: its own result, unless an action error is still waiting to be read. */
function afterSync(current: ShownError | null, next: ShownError | null): ShownError | null {
  return current === null || current.fromSync ? next : current;
}

function toggled(current: ReadonlySet<string>, id: string, busy: boolean): ReadonlySet<string> {
  const next = new Set(current);
  if (busy) {
    next.add(id);
  } else {
    next.delete(id);
  }
  return next;
}

/** A source's running action, or none: `busy` null clears it. */
function marked(current: ReadonlyMap<string, SourceAction>, id: string, busy: SourceAction | null): ReadonlyMap<string, SourceAction> {
  const next = new Map(current);
  if (busy === null) {
    next.delete(id);
  } else {
    next.set(id, busy);
  }
  return next;
}

function backupNotice(lead: string, paths: readonly string[]): InfoNotice {
  return { text: `${lead} ${paths.join(", ")}.`, folder: paths[0] ?? null, caution: false };
}

function infoText(text: string): InfoNotice {
  return { text, folder: null, caution: false };
}

/** Shown while a locked-file or dropped-connection failure gets its one automatic retry. */
const TRYING_AGAIN = infoText("Trying again…");

export default function App(): JSX.Element {
  const [state, setState] = useState<AppState | null>(null);
  const [error, setError] = useState<ShownError | null>(null);
  const [info, setInfo] = useState<InfoNotice | null>(null);
  const [dismissedReport, setDismissedReport] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [adding, setAdding] = useState<string | null>(null);
  const [sourceDialogOpen, setSourceDialogOpen] = useState(false);
  const [busyItems, setBusyItems] = useState<ReadonlySet<string>>(new Set());
  const [busySources, setBusySources] = useState<ReadonlyMap<string, SourceAction>>(new Map());
  const [resetting, setResetting] = useState(false);
  const [tutorialRunning, setTutorialRunning] = useState(false);
  const [creatingSkill, setCreatingSkill] = useState(false);
  const [statusDialogOpen, setStatusDialogOpen] = useState(false);
  const [preflightRunning, setPreflightRunning] = useState(false);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsResult | null>(null);
  const [query, setQuery] = useState("");
  const [driftOnly, setDriftOnly] = useState(false);
  // An action or sync that failed for lack of a connection shows the offline banner until the next sync answers.
  const [offlineHint, setOfflineHint] = useState(false);

  const applyState = useCallback((next: AppState): void => {
    startTransition(() => {
      setState(next);
    });
  }, []);

  const loadCached = useCallback(async (): Promise<void> => {
    const cached = await invokeParsed("load_cached_manifest_state", cachedStateSchema);
    if (cached !== null) {
      applyState(cached);
    }
  }, [applyState]);

  /** A sync's own result: its state, and the end of any sync error or offline guess it leaves behind. */
  const applySynced = useCallback(
    (next: AppState): void => {
      applyState(next);
      setOfflineHint(false);
      setError((current) => afterSync(current, null));
    },
    [applyState]
  );

  /** Being offline is never a red error: it feeds the offline banner, and the next sync clears it. */
  const showSyncError = useCallback((shown: AppError): void => {
    if (errorResponse(shown.kind) === "offline") {
      setOfflineHint(true);
      setError((current) => afterSync(current, null));
    } else {
      setError((current) => afterSync(current, fromSync(shown)));
    }
  }, []);

  const synchronize = useCallback(async (): Promise<void> => {
    setSyncing(true);
    try {
      applySynced(await invokeParsed("sync_manifest_state", appStateSchema));
    } catch (reason) {
      showSyncError(toAppError(reason, SYNC_FAILED));
    } finally {
      setSyncing(false);
    }
  }, [applySynced, showSyncError]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<unknown>(SCHEDULED_SYNC_EVENT, (event) => {
      if (disposed) {
        return;
      }
      try {
        const scheduled = scheduledSyncSchema.parse(event.payload);
        if (scheduled.kind === "updated") {
          applySynced(scheduled.state);
        } else {
          showSyncError(toAppError(scheduled.error, SYNC_FAILED));
        }
      } catch (reason: unknown) {
        showSyncError(toAppError(reason, SYNC_FAILED));
      }
    })
      .then((stop) => {
        if (disposed) {
          stop();
        } else {
          unlisten = stop;
        }
      })
      .catch((reason: unknown) => {
        if (!disposed) {
          setError(fromAction(toAppError(reason, "Couldn't start automatic update checks.")));
        }
      });
    loadCached()
      .catch((reason: unknown) => {
        if (!disposed) {
          showSyncError(toAppError(reason, LOAD_FAILED));
        }
      })
      .then(() => {
        if (!disposed) {
          return synchronize();
        }
        return undefined;
      })
      .catch((reason: unknown) => {
        if (!disposed) {
          showSyncError(toAppError(reason, SYNC_FAILED));
        }
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [applySynced, loadCached, showSyncError, synchronize]);

  const itemsBySource = useMemo(() => {
    const grouped = new Map<string, CatalogItem[]>();
    const needle = query.trim().toLowerCase();
    for (const item of state?.items ?? []) {
      if (needle.length > 0 && !matchesQuery(item, needle)) {
        continue;
      }
      if (driftOnly && item.status !== "modified") {
        continue;
      }
      const items = grouped.get(item.sourceKey) ?? [];
      items.push(item);
      grouped.set(item.sourceKey, items);
    }
    return grouped;
  }, [driftOnly, query, state]);

  const filtering = query.trim().length > 0 || driftOnly;

  const visibleSources = useMemo(() => {
    const sources = state?.sources ?? [];
    if (!filtering) {
      return sources;
    }
    return sources.filter((source) => (itemsBySource.get(source.sourceKey) ?? []).length > 0);
  }, [filtering, itemsBySource, state]);

  async function rerunPreflight(): Promise<void> {
    setPreflightRunning(true);
    setDiagnostics(null);
    try {
      const report = await invokeParsed("run_preflight", preflightReportSchema);
      // The run rewrites the identity and preflight caches, so reload before pinning the report:
      // a machine with no marketplace configured loads neither back.
      await loadCached();
      setState((current) => (current === null ? current : { ...current, preflight: report }));
      setDiagnostics(diagnosticsResult(report));
      setError(null);
    } catch (reason) {
      setDiagnostics(diagnosticsFailure(toAppError(reason).summary));
      setError(fromAction(toAppError(reason, "Diagnostics could not run.")));
    } finally {
      setPreflightRunning(false);
    }
  }

  function showActionError(shown: AppError): void {
    if (errorResponse(shown.kind) === "offline") {
      setOfflineHint(true);
      return;
    }
    setError(fromAction(explainAfterRetry(shown, state?.agentProfiles.map((profile) => profile.displayName) ?? [])));
  }

  /** A person's action, retried once on its own when a locked file or dropped connection stopped it. */
  async function retrying<T>(task: () => Promise<T>): Promise<T> {
    try {
      return await withRetry(task, () => {
        setInfo(TRYING_AGAIN);
      });
    } finally {
      setInfo((current) => (current === TRYING_AGAIN ? null : current));
    }
  }

  // The actions below report their own failures; this only catches what slips past them.
  function settle(task: Promise<void>): void {
    task.catch((reason: unknown) => {
      showActionError(toAppError(reason));
    });
  }

  function handleStatusAction(action: string): void {
    setStatusDialogOpen(false);
    const handlers: Readonly<Record<string, () => void>> = {
      sync: () => {
        settle(synchronize());
      },
      update: () => {
        openDownloadSite().catch((reason: unknown) => {
          showActionError(toAppError(reason, "Couldn't open the download site."));
        });
      },
      showDrift: () => {
        setQuery("");
        setDriftOnly(true);
      },
      installPublishSkill: () => {
        setDriftOnly(false);
        setQuery("official/publish");
      }
    };
    handlers[action]?.();
  }

  async function openDownloadSite(): Promise<void> {
    const url = state?.downloadUrl ?? null;
    if (url === null) {
      setInfo(infoText("This build has no download site configured. Ask your administrator where to get the current Agent Plugins, then install it over this one."));
      return;
    }
    await openUrl(url);
  }

  // The view has to match the machine even when an operation failed: a stale
  // card is what makes a failed click look like nothing happened. It never
  // replaces an action error, so a failure raised by the operation survives here.
  async function refreshAfterOperation(): Promise<void> {
    try {
      await loadCached();
    } catch (reason) {
      setError((current) => afterSync(current, fromSync(toAppError(reason, LOAD_FAILED))));
    }
  }

  async function changeItem(item: CatalogItem, componentId?: string): Promise<void> {
    const plan = itemCommand(item, componentId);
    if (plan === null) {
      return;
    }
    // Busy before the first confirmation, so a double-click cannot open a second one.
    setBusyItems((current) => toggled(current, item.id, true));
    try {
      if (plan.review !== null && !(await plan.review())) {
        return;
      }
      setError(null);
      setInfo(null);
      // An MCP server runs a command on this machine, so it is installed only
      // after the person says so. The backend refuses without this approval.
      if (plan.trustApproved && !(await reviewApproval(item.name, item.riskDetails))) {
        return;
      }
      const outcome = await retrying(() => invokeParsed(plan.command, operationOutcomeSchema, itemCommandArgs(item, componentId, { trustApproved: plan.trustApproved })));
      const notice = outcomeNotice(plan.backupLead, outcome);
      if (notice !== null) {
        setInfo(notice);
      }
    } catch (reason) {
      showActionError(toAppError(reason, `Couldn't ${plan.verb} ${item.name}.`));
    } finally {
      setBusyItems((current) => toggled(current, item.id, false));
      await refreshAfterOperation();
    }
  }

  async function changeManualInvocation(item: CatalogItem, manual: boolean, componentId?: string): Promise<void> {
    setBusyItems((current) => toggled(current, item.id, true));
    try {
      setError(null);
      setInfo(null);
      const outcome = await retrying(() => invokeParsed("set_manual_invocation", operationOutcomeSchema, itemCommandArgs(item, componentId, { manual })));
      const notice = outcomeNotice("The files that were there before were backed up to", outcome);
      if (notice !== null) {
        setInfo(notice);
      }
    } catch (reason) {
      showActionError(toAppError(reason, `Couldn't change how ${item.name} is used.`));
    } finally {
      setBusyItems((current) => toggled(current, item.id, false));
      await refreshAfterOperation();
    }
  }

  async function runBulk(source: SourceState, action: BulkAction): Promise<void> {
    const verb = bulkLabels(action).action.toLowerCase();
    setError(null);
    setInfo(null);
    setBusySources((current) => marked(current, source.sourceId, action));
    try {
      const plan = await retrying(() => invokeParsed("plan_bulk_items", bulkPlanSchema, { sourceId: source.sourceId, action }));
      const eligible = plan.entries.filter((entry) => entry.willRun);
      if (eligible.length === 0) {
        setInfo(infoText(`Nothing to ${verb} in ${source.name} right now.`));
        return;
      }
      if (action !== "install" && !(await reviewBulk(source, action, plan))) {
        return;
      }
      const items = state?.items ?? [];
      const needApproval = action === "uninstall" ? [] : items.filter((item) => item.requiresApproval && eligible.some((entry) => entry.id === item.id));
      const approvals = needApproval.map((item) => item.name);
      if (
        approvals.length > 0 &&
        !(await reviewBulkApproval(
          approvals,
          needApproval.flatMap((item) => item.riskDetails)
        ))
      ) {
        return;
      }
      const result = await retrying(() => invokeParsed("run_bulk_items", bulkResultSchema, { sourceId: source.sourceId, action, trustApproved: approvals.length > 0 }));
      if (result.failures.length > 0) {
        showActionError(failuresError(verb, result.failures, items, source.name));
      }
      if (result.backupPaths.length > 0) {
        setInfo(backupNotice("The files that were there before were backed up to", result.backupPaths));
      }
    } catch (reason) {
      showActionError(toAppError(reason, `Couldn't ${verb} packages from ${source.name}.`));
    } finally {
      setBusySources((current) => marked(current, source.sourceId, null));
      await refreshAfterOperation();
    }
  }

  async function resetApp(): Promise<void> {
    if (!(await reviewReset())) {
      return;
    }
    setResetting(true);
    setError(null);
    setInfo(null);
    try {
      const result = await invokeParsed("reset_app", bulkResultSchema);
      const count = result.completed.length;
      if (result.failures.length > 0) {
        showActionError(failuresError("uninstall", result.failures, state?.items ?? []));
      } else {
        const cleared = count === 0 ? "Cleared all Agent Plugins data." : `Uninstalled ${String(count)} package${count === 1 ? "" : "s"} and cleared all Agent Plugins data.`;
        const backups = result.backupPaths.length === 0 ? "" : ` Changed files were backed up to ${result.backupPaths.join(", ")}.`;
        setInfo({ text: `${cleared}${backups}`, folder: result.backupPaths[0] ?? null, caution: false });
      }
      await synchronize();
    } catch (reason) {
      showActionError(toAppError(reason, "Couldn't reset Agent Plugins."));
    } finally {
      setResetting(false);
    }
  }

  async function runTutorial({ targetId, displayName: app }: AgentProfile): Promise<void> {
    // Busy before the confirmation, so a double-click cannot open a second one.
    setTutorialRunning(true);
    try {
      if (!(await reviewTutorial(app))) {
        return;
      }
      setError(null);
      setInfo(null);
      await invokeParsed("run_tutorial", unitSchema, { targetId });
      setInfo(infoText(`Reopening ${app}. Choose Create Chat, then send the message to see the skill work.`));
    } catch (reason) {
      showActionError(toAppError(reason, `Couldn't start the ${app} tutorial.`));
    } finally {
      setTutorialRunning(false);
      await refreshAfterOperation();
    }
  }

  async function createSkill({ targetId, displayName: app }: AgentProfile): Promise<void> {
    setCreatingSkill(true);
    setError(null);
    setInfo(null);
    try {
      await invokeParsed("create_skill", unitSchema, { targetId });
      setInfo(infoText(`Opening ${app}. Choose Create Chat, then send the message to start making your skill. If ${app} asks you to log in, do that first, then choose Create a skill again.`));
    } catch (reason) {
      showActionError(toAppError(reason, `Couldn't open ${app}.`));
    } finally {
      setCreatingSkill(false);
    }
  }

  async function dismissTutorial(): Promise<void> {
    setState((current) => (current === null ? current : { ...current, tutorial: null }));
    try {
      await invokeParsed("dismiss_tutorial", unitSchema);
    } finally {
      await refreshAfterOperation();
    }
  }

  async function addListedSource(repository: RepositoryState, listed: ListedSource): Promise<void> {
    setError(null);
    setInfo(null);
    setAdding(listed.url);
    try {
      const prepared = await retrying(() => invokeParsed("prepare_source", preparedSourceSchema, { url: listed.url, repositoryKey: repository.repositoryKey }));
      try {
        applyState(await invokeParsed("confirm_source", appStateSchema, { token: prepared.token }));
      } catch (reason) {
        // Leave no staged candidate behind when the confirmation fails.
        await invokeParsed("cancel_prepared_source", unitSchema, { token: prepared.token }).catch(() => undefined);
        throw reason;
      }
    } catch (reason) {
      showActionError(toAppError(reason, `Couldn't add ${listed.name}.`));
    } finally {
      setAdding(null);
    }
  }

  async function removeSource(source: SourceState): Promise<void> {
    // Busy before the first await, so a double-click cannot open a second confirmation.
    setBusySources((current) => marked(current, source.sourceId, "remove"));
    try {
      const plan = await invokeParsed("plan_source_removal", sourceRemovalPlanSchema, { sourceId: source.sourceId });
      // Removing a source uninstalls everything it installed, and local edits go
      // with it, so the person acknowledges both before anything is touched.
      if (!(await reviewSourceRemoval(source, plan, state?.repositories ?? []))) {
        return;
      }
      const modified = plan.items.flatMap((item) => item.paths).filter((path) => path.modified);
      setError(null);
      setInfo(null);
      const result = await retrying(() => invokeParsed("remove_manifest_source", bulkResultSchema, { sourceId: source.sourceId, acknowledgeModifiedPaths: modified.length > 0 }));
      if (result.failures.length > 0) {
        showActionError(failuresError("uninstall", result.failures, state?.items ?? [], source.name));
      }
      if (result.backupPaths.length > 0) {
        setInfo(backupNotice("Removed files were backed up to", result.backupPaths));
      }
    } catch (reason) {
      showActionError(toAppError(reason, `Couldn't remove ${source.name}.`));
    } finally {
      setBusySources((current) => marked(current, source.sourceId, null));
      await refreshAfterOperation();
    }
  }

  function retryLoad(): void {
    setError(null);
    loadCached()
      .catch((reason: unknown) => {
        showSyncError(toAppError(reason, LOAD_FAILED));
      })
      .then(synchronize)
      .catch((reason: unknown) => {
        showSyncError(toAppError(reason, SYNC_FAILED));
      });
  }

  function tryNow(): void {
    settle(synchronize());
  }

  const checked = lastCheckedLabel(state);
  const body = catalogBody(state, offlineHint, filtering);
  const view = marketplaceView(state, isOffline(state, offlineHint));
  const noMatches = noMatchesText(query, driftOnly, visibleSources.length);
  const matchCount = [...itemsBySource.values()].reduce((total, items) => total + items.length, 0);
  // With nothing loaded yet, a failure replaces the spinner instead of sitting above it forever.
  const loadFailed = state === null && error !== null && !syncing;
  const report = visibleReport(state, dismissedReport);
  return (
    <main className="app-shell">
      <header className="app-header">
        <div>
          <Heading as="h1" size="7">
            Agent Plugins
          </Heading>
        </div>
        <div className="catalog-actions">
          <CreateSkillButton
            profile={skillCreatorProfile(state)}
            running={creatingSkill}
            onClick={(profile) => {
              settle(createSkill(profile));
            }}
          />
          <StatusButton
            problems={view.problems}
            identity={view.identity}
            disabled={resetting}
            onClick={() => {
              setStatusDialogOpen(true);
            }}
          />
          <Button
            variant="soft"
            disabled={resetting}
            onClick={() => {
              setSourceDialogOpen(true);
            }}
          >
            Manage sources
          </Button>
          <Button loading={syncing} disabled={syncing || resetting} onClick={() => void synchronize()}>
            Refresh
          </Button>
          <Button
            color="red"
            variant="soft"
            loading={resetting}
            disabled={resetting || syncing}
            onClick={() => {
              settle(resetApp());
            }}
          >
            Reset
          </Button>
        </div>
      </header>
      <SyncMeta checked={checked} checking={isChecking(state, syncing)} problems={view.problems} blocked={view.blocked} />
      <CatalogToolbar
        query={query}
        matches={matchCount}
        driftOnly={driftOnly}
        onQueryChange={setQuery}
        onClearDrift={() => {
          setDriftOnly(false);
        }}
      />
      <div className="notices">
        <OfflineBanner text={offlineBanner(state, offlineHint)} checking={syncing} onTryNow={tryNow} />
        <AgentSetupNotice
          visible={state !== null && !hasDetectedAgent(state.agentProfiles)}
          onChoose={() => {
            setStatusDialogOpen(true);
          }}
        />
        <Notices
          error={loadFailed ? null : error}
          info={info}
          report={report}
          onDismissError={() => {
            setError(null);
          }}
          onDismissInfo={() => {
            setInfo(null);
          }}
          onDismissReport={() => {
            setDismissedReport(report?.text ?? null);
          }}
          onOpenFolder={(path) => {
            revealItemInDir(path).catch((reason: unknown) => {
              showActionError(toAppError(reason, "Couldn't open the folder."));
            });
          }}
        />
      </div>
      {/* An invitation, not a notice: it scrolls away with the page instead of staying pinned over it. */}
      <TutorialNotice
        profile={tutorialProfile(state)}
        running={tutorialRunning}
        onStart={(profile) => {
          settle(runTutorial(profile));
        }}
        onDismiss={() => {
          settle(dismissTutorial());
        }}
      />
      {body.kind === "empty" ? (
        <EmptyCatalog text={body.text} offline={body.offline} checking={syncing} onTryNow={tryNow} />
      ) : body.kind === "loading" ? (
        <LoadingOrFailed error={loadFailed ? error : null} onRetry={retryLoad} />
      ) : noMatches !== null ? (
        <div className="load-failed">
          <Text color="gray">{noMatches}</Text>
          <Button
            onClick={() => {
              setQuery("");
              setDriftOnly(false);
            }}
          >
            Show all
          </Button>
        </div>
      ) : (
        <div className="sources-list">
          {visibleSources.map((source) => (
            <SourceGroup
              key={source.sourceKey}
              source={source}
              items={itemsBySource.get(source.sourceKey) ?? []}
              busyIds={busyItems}
              allBusy={resetting || busySources.has(source.sourceId)}
              running={busySources.get(source.sourceId) ?? null}
              filtering={filtering}
              onItemChange={changeItem}
              onManualChange={changeManualInvocation}
              onBulk={runBulk}
              onError={showActionError}
            />
          ))}
        </div>
      )}
      <ManageSourcesDialog
        open={sourceDialogOpen}
        state={state}
        adding={adding}
        error={sourceDialogOpen ? error : null}
        removing={busySources}
        onOpenChange={setSourceDialogOpen}
        onAddListed={addListedSource}
        onRemove={removeSource}
        onError={showActionError}
      />
      <SystemStatusDialog
        open={statusDialogOpen}
        report={view.preflight}
        identity={view.identity}
        marketplaceUrl={view.marketplaceUrl}
        profiles={state?.agentProfiles ?? []}
        running={preflightRunning}
        diagnostics={diagnostics}
        onOpenChange={(open) => {
          setStatusDialogOpen(open);
          if (!open) {
            setDiagnostics(null);
          }
        }}
        onRerun={() => {
          settle(rerunPreflight());
        }}
        onAction={handleStatusAction}
      />
    </main>
  );
}

/** The app the skill tutorial is offered for, until it has run once. */
function tutorialProfile(state: AppState | null): AgentProfile | null {
  return state?.agentProfiles.find((profile) => profile.targetId === state.tutorial) ?? null;
}

/** The detected app **Create a skill** opens: Cursor, the one `tutorial.rs` knows how to launch with a prompt. */
function skillCreatorProfile(state: AppState | null): AgentProfile | null {
  return state?.agentProfiles.find((profile) => profile.detected && profile.targetId === "cursor") ?? null;
}

/** The background-update report, unless the person already dismissed this exact one. */
function visibleReport(state: AppState | null, dismissed: string | null): ReportNotice | null {
  const report = state === null ? null : reportNotice(state);
  return report === null || report.text === dismissed ? null : report;
}

function LoadingOrFailed({ error, onRetry }: Readonly<{ error: AppError | null; onRetry: () => void }>): JSX.Element {
  if (error === null) {
    return (
      <div className="loading-state">
        <Spinner size="3" />
        <Text color="gray">Loading packages…</Text>
      </div>
    );
  }
  return (
    <div className="load-failed">
      <Callout.Root className="app-callout" color="red" role="alert">
        <ErrorMessage summary={error.summary} detail={error.detail} />
      </Callout.Root>
      <Button onClick={onRetry}>Try again</Button>
    </div>
  );
}

function EmptyCatalog({ text, offline, checking, onTryNow }: Readonly<{ text: string; offline: boolean; checking: boolean; onTryNow: () => void }>): JSX.Element {
  return (
    <div className="load-failed">
      <Text color="gray">{text}</Text>
      {offline ? (
        <Button loading={checking} disabled={checking} onClick={onTryNow}>
          Try now
        </Button>
      ) : null}
    </div>
  );
}

function matchesQuery(item: CatalogItem, needle: string): boolean {
  const haystack = [item.id, item.name, item.description, item.sourceName, item.marketplace?.publisher ?? "", ...(item.marketplace?.tags ?? [])].join(" ").toLowerCase();
  return haystack.includes(needle);
}

function marketplaceView(
  state: AppState | null,
  offline: boolean
): Readonly<{ identity: AppIdentity | null; marketplaceUrl: string | null; preflight: PreflightReport | null; blocked: boolean; problems: readonly PreflightCheck[] }> {
  const preflight = state?.preflight ?? null;
  return {
    identity: state?.identity ?? null,
    marketplaceUrl: state?.marketplaceUrl ?? null,
    preflight,
    blocked: preflight?.blocked === true,
    problems: headerProblems(seriousProblems(preflight), offline)
  };
}
