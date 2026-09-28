import { startTransition, useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import type { JSX } from "react";
import { Badge, Button, Callout, Heading, Spinner, Text } from "@radix-ui/themes";
import { listen } from "@tauri-apps/api/event";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { AgentSetupNotice, TutorialNotice } from "./components/AgentSetupNotice";
import { ApprovalDialog } from "./components/ApprovalDialog";
import type { ApprovalEntry, ApprovalRequest, ConnectorSettings } from "./components/ApprovalDialog";
import { AppsDialog } from "./components/AppsDialog";
import type { AppsRequest } from "./components/AppsDialog";
import { CardExtrasContext } from "./components/ItemCard";
import type { CardExtras } from "./components/ItemCard";
import { BundleDialog, BundleGroup } from "./components/Bundles";
import type { BundleEdit } from "./components/Bundles";
import { LinkInstallDialog, OpenLinkDialog } from "./components/LinkDialogs";
import type { LinkRequest } from "./components/LinkDialogs";
import { ShareDialog } from "./components/ShareDialog";
import type { ShareTarget } from "./components/ShareDialog";
import { TeamsDialog } from "./components/TeamsDialog";
import { ManageSourcesDialog } from "./components/ManageSourcesDialog";
import { PromptAppDialog } from "./components/PromptAppDialog";
import type { PromptPurpose } from "./components/PromptAppDialog";
import { ErrorMessage, Notices, OfflineBanner } from "./components/Notice";
import type { InfoNotice } from "./components/Notice";
import { SourceGroup } from "./components/SourceGroup";
import type { SourceAction } from "./components/SourceGroup";
import { CatalogFilters, CatalogToolbar, StatusButton, SyncMeta } from "./components/CatalogToolbar";
import { diagnosticsFailure, diagnosticsResult, seriousProblems, SystemStatusDialog } from "./components/SystemStatusDialog";
import { DEEP_LINK_EVENT, errorResponse, explainAfterRetry, invokeParsed, SCHEDULED_SYNC_EVENT, toAppError, withRetry } from "./ipc/client";
import type { AppError } from "./ipc/client";
import {
  appStateSchema,
  bulkPlanSchema,
  bulkResultSchema,
  cachedStateSchema,
  itemsPlanSchema,
  operationOutcomeSchema,
  pendingLinkSchema,
  preflightReportSchema,
  preparedSourceSchema,
  scheduledSyncSchema,
  sourceRemovalPlanSchema,
  textSchema,
  unitSchema
} from "./ipc/schemas";
import type { DiagnosticsResult } from "./components/SystemStatusDialog";
import type {
  AgentProfile,
  AppIdentity,
  AppState,
  BulkAction,
  BulkPlanEntry,
  BulkResult,
  BundleState,
  CatalogItem,
  DeepLink,
  LinkResult,
  ListedSource,
  PreflightCheck,
  PreflightReport,
  PromptApp,
  RepositoryState,
  SourceState
} from "./ipc/schemas";
import { useStableCallback } from "./lib/stableCallback";
import { catalogBody, headerProblems, isChecking, isOffline, lastCheckedLabel, missingLinkText, noMatchesText, offlineBanner } from "./lib/connectivity";
import { bundleMembers, cardDomId, matchesAllWords, ownsSpace, portalUrl, resolveLink } from "./lib/marketplace";
import type { LinkTarget } from "./lib/marketplace";
import {
  bulkLabels,
  failuresError,
  hasDetectedAgent,
  installedNotice,
  itemCommand,
  itemCommandArgs,
  outcomeNotice,
  packageName,
  reportNotice,
  reviewBulk,
  reviewBundleDelete,
  reviewForceRemove,
  reviewKeepMine,
  reviewBundleUninstall,
  reviewReset,
  reviewSourceRemoval,
  shownDigests,
  uninstalledNotice
} from "./lib/status";
import type { ReportNotice } from "./lib/status";
import "./App.css";

/** Errors remember whether syncing raised them: a later sync may clear its own errors, never an action error the person has not read. */
type ShownError = AppError & Readonly<{ fromSync: boolean }>;

const SYNC_FAILED = "Couldn't check for updates.";
const LOAD_FAILED = "Couldn't load your skills.";

/** What the catalog shows: everything, or one kind of card. */
type CatalogFilter = "all" | "installed" | "updates" | "official" | "team" | "shared";
type CatalogSort = "name" | "used";

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

/** `next`, with every item equal to one in `current` kept as that object, so a sync or reload that didn't touch a card doesn't re-render it. */
function withUnchangedItems(current: AppState | null, next: AppState): AppState {
  if (current === null) {
    return next;
  }
  const before = new Map(current.items.map((item) => [item.id, item]));
  const items = next.items.map((item) => {
    const old = before.get(item.id);
    return old !== undefined && JSON.stringify(old) === JSON.stringify(item) ? old : item;
  });
  return { ...next, items };
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
  /** The newest background report, kept until dismissed: states from later actions don't carry it. */
  const [lastReport, setLastReport] = useState<ReportNotice | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [adding, setAdding] = useState<string | null>(null);
  const [sourceDialogOpen, setSourceDialogOpen] = useState(false);
  const [busyItems, setBusyItems] = useState<ReadonlySet<string>>(new Set());
  const [busySources, setBusySources] = useState<ReadonlyMap<string, SourceAction>>(new Map());
  const [resetting, setResetting] = useState(false);
  /** What the app picker is open for; null while it is closed. */
  const [promptPurpose, setPromptPurpose] = useState<PromptPurpose | null>(null);
  const [promptRunning, setPromptRunning] = useState(false);
  const [statusDialogOpen, setStatusDialogOpen] = useState(false);
  const [preflightRunning, setPreflightRunning] = useState(false);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsResult | null>(null);
  const [query, setQuery] = useState("");
  const [driftOnly, setDriftOnly] = useState(false);
  // An action or sync that failed for lack of a connection shows the offline banner until the next sync answers.
  const [offlineHint, setOfflineHint] = useState(false);
  const [teamsOpen, setTeamsOpen] = useState(false);
  const [openLinkOpen, setOpenLinkOpen] = useState(false);
  const [shareTarget, setShareTarget] = useState<ShareTarget | null>(null);
  const [bundleEdit, setBundleEdit] = useState<BundleEdit | null>(null);
  const [linkRequest, setLinkRequest] = useState<LinkRequest | null>(null);
  const [busyBundles, setBusyBundles] = useState<ReadonlyMap<string, SourceAction>>(new Map());
  const [approvalRequest, setApprovalRequest] = useState<ApprovalRequest | null>(null);
  const approvalAnswer = useRef<((settings: ConnectorSettings | null) => void) | null>(null);
  /** A link that arrived while a connector was being asked about, shown once that is answered. */
  const heldLink = useRef<LinkTarget | null>(null);
  const [appsRequest, setAppsRequest] = useState<AppsRequest | null>(null);
  const [catalogFilter, setCatalogFilter] = useState<CatalogFilter>("all");
  const [catalogSort, setCatalogSort] = useState<CatalogSort>("name");
  /** A card a link asked for, scrolled to as soon as it is on screen. */
  const [revealing, setRevealing] = useState<string | null>(null);
  // Typing stays instant: the field shows `query`, and the catalog catches up with it in the background.
  const deferredQuery = useDeferredValue(query);
  // Links resolve against the newest state, which an async handler can't read from its closure.
  const stateRef = useRef<AppState | null>(null);
  const linkSeq = useRef(0);
  useEffect(() => {
    stateRef.current = state;
  }, [state]);

  const applyState = useCallback((next: AppState): void => {
    const fresh = reportNotice(next);
    startTransition(() => {
      setState((current) => withUnchangedItems(current, next));
      if (fresh !== null) {
        setLastReport(fresh);
      }
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

  /** Clears any filter hiding `id`'s card; the effect below scrolls to it once it has rendered. */
  const reveal = useCallback((id: string): void => {
    setQuery("");
    setDriftOnly(false);
    setLinkRequest(null);
    setRevealing(id);
  }, []);

  /**
   * An `agent-plugins://` link. Something just published or shared may not be
   * in this window's catalog until the next check, so a miss checks first;
   * `fresh` always checks, for access that was granted a moment ago. The
   * newest link wins: a later one replaces a confirmation still waiting.
   */
  const openLink = useCallback(
    async (link: DeepLink, fresh = false): Promise<void> => {
      linkSeq.current += 1;
      const seq = linkSeq.current;
      const current = stateRef.current;
      let latest = current;
      let target: LinkTarget | null = current === null || fresh ? null : resolveLink(link, current);
      if (target === null) {
        setSyncing(true);
        try {
          latest = await invokeParsed("sync_manifest_state", appStateSchema);
        } finally {
          setSyncing(false);
        }
        applySynced(latest);
        target = resolveLink(link, latest);
      }
      if (seq !== linkSeq.current) {
        return;
      }
      if (target === null) {
        setInfo(infoText(missingLinkText(latest, false)));
      } else if (link.kind === "open") {
        reveal(`${link.namespace}/${link.id}`);
      } else if (approvalAnswer.current !== null) {
        // Asking about a connector now: the link waits until that is answered.
        heldLink.current = target;
      } else {
        setLinkRequest({ target });
      }
    },
    [applySynced, reveal]
  );

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // The backend keeps the newest link until it is taken, and the event only says one is
    // waiting. Taking it on every event and on mount handles a link that started the app
    // (it arrived before this window could listen) and never handles the same link twice.
    const take = (): void => {
      invokeParsed("take_pending_link", pendingLinkSchema)
        .then(async (link) => {
          if (link !== null && !disposed) {
            await openLink(link);
          }
        })
        .catch((reason: unknown) => {
          if (!disposed) {
            setError(fromAction(toAppError(reason, "Couldn't open the link.")));
          }
        });
    };
    listen<unknown>(DEEP_LINK_EVENT, take)
      .then((stop) => {
        if (disposed) {
          stop();
        } else {
          unlisten = stop;
        }
      })
      .catch((reason: unknown) => {
        if (!disposed) {
          setError(fromAction(toAppError(reason, "Couldn't start listening for links.")));
        }
      });
    take();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [openLink]);

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
    for (const item of state?.items ?? []) {
      if (!matchesQuery(item, deferredQuery) || !matchesFilter(item, catalogFilter)) {
        continue;
      }
      if (driftOnly && item.status !== "modified") {
        continue;
      }
      const items = grouped.get(item.sourceKey) ?? [];
      items.push(item);
      grouped.set(item.sourceKey, items);
    }
    if (catalogSort === "used") {
      for (const items of grouped.values()) {
        items.sort((left, right) => (usersOf(right) === usersOf(left) ? left.name.localeCompare(right.name) : usersOf(right) - usersOf(left)));
      }
    }
    return grouped;
  }, [catalogFilter, catalogSort, deferredQuery, driftOnly, state]);

  const filtering = deferredQuery.trim().length > 0 || driftOnly || catalogFilter !== "all";

  const visibleBundles = useMemo(() => {
    if (driftOnly || catalogFilter !== "all") {
      return [];
    }
    return (state?.bundles ?? []).filter((bundle) => matchesAllWords(`${bundle.id} ${bundle.name} ${bundle.description} ${bundle.publisher}`, deferredQuery));
  }, [catalogFilter, deferredQuery, driftOnly, state]);

  useEffect(() => {
    if (revealing === null) {
      return;
    }
    const card = document.getElementById(cardDomId(revealing));
    if (card === null) {
      // Not rendered yet: the cleared filter or a just-synced catalog brings it, and this runs again.
      return;
    }
    setRevealing(null);
    // Instant: a smooth scroll repaints every frame of a long list on a software-rendered VM.
    card.scrollIntoView({ block: "center" });
    card.classList.add("card-highlight");
    window.setTimeout(() => {
      card.classList.remove("card-highlight");
    }, 2400);
  }, [revealing, itemsBySource, visibleBundles]);

  const visibleSources = useMemo(() => {
    const sources = state?.sources ?? [];
    const shown = filtering ? sources.filter((source) => (itemsBySource.get(source.sourceKey) ?? []).length > 0) : sources;
    if (catalogSort !== "used") {
      return shown;
    }
    const most = (source: SourceState): number => Math.max(0, ...(itemsBySource.get(source.sourceKey) ?? []).map((item) => item.marketplace?.installedBase ?? 0));
    return [...shown].sort((left, right) => most(right) - most(left));
  }, [catalogSort, filtering, itemsBySource, state]);

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
      if (plan.trustApproved && !(await approveConnectors([{ item, componentId: componentId ?? null }]))) {
        return;
      }
      // What the dialog showed: the backend asks again if the package changed since.
      const shown = plan.trustApproved ? shownDigests([item]) : null;
      const outcome = await retrying(() => invokeParsed(plan.command, operationOutcomeSchema, itemCommandArgs(item, componentId, { trustApproved: plan.trustApproved, shown })));
      const notice = plan.command === "uninstall_item" ? uninstalledNotice(item, componentId, plan.backupLead, outcome) : installedNotice(item, componentId, state?.agentProfiles ?? [], plan, outcome);
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

  /** Opens the connector dialog and waits for the person's answer; null means they cancelled. */
  async function askConnectors(request: ApprovalRequest): Promise<ConnectorSettings | null> {
    approvalAnswer.current?.(null);
    return new Promise((resolve) => {
      approvalAnswer.current = resolve;
      setApprovalRequest(request);
    });
  }

  function answerConnectors(settings: ConnectorSettings | null): void {
    approvalAnswer.current?.(settings);
    approvalAnswer.current = null;
    setApprovalRequest(null);
    if (heldLink.current !== null) {
      setLinkRequest({ target: heldLink.current });
      heldLink.current = null;
    }
  }

  /** Asks before connectors are installed, saving any API keys typed in. False means the person said no. */
  async function approveConnectors(entries: readonly ApprovalEntry[]): Promise<boolean> {
    const settings = await askConnectors({ entries, mode: "install" });
    if (settings === null) {
      return false;
    }
    if (Object.keys(settings).length > 0) {
      await invokeParsed("save_connector_settings", unitSchema, { values: settings });
    }
    return true;
  }

  async function editConnectorSettings(item: CatalogItem): Promise<void> {
    const settings = await askConnectors({ entries: [{ item, componentId: null }], mode: "settings" });
    if (settings === null || Object.keys(settings).length === 0) {
      return;
    }
    try {
      await invokeParsed("save_connector_settings", unitSchema, { values: settings });
      setInfo(infoText(`Saved. Quit and reopen your AI apps so ${item.name} picks up the change.`));
    } catch (reason) {
      showActionError(toAppError(reason, "Couldn't save the connector settings."));
    } finally {
      await refreshAfterOperation();
    }
  }

  /** Runs one card action with the card busy, then shows the machine as it is now. */
  async function onCard(item: CatalogItem, failure: string, task: () => Promise<void>): Promise<void> {
    setBusyItems((current) => toggled(current, item.id, true));
    setError(null);
    setInfo(null);
    try {
      await task();
    } catch (reason) {
      showActionError(toAppError(reason, failure));
    } finally {
      setBusyItems((current) => toggled(current, item.id, false));
      await refreshAfterOperation();
    }
  }

  function cardExtrasFor(marketplaceUrl: string | null): CardExtras {
    return {
      onDetails:
        marketplaceUrl === null
          ? null
          : (item) => {
              openUrl(portalUrl(marketplaceUrl, `/p/${item.sourceId}/${item.localId}`)).catch((reason: unknown) => {
                showActionError(toAppError(reason, "Couldn't open the marketplace."));
              });
            },
      onKeepMine: async (item) => {
        if (!(await reviewKeepMine(item.name))) {
          return;
        }
        await onCard(item, `Couldn't keep your copy of ${item.name}.`, async () => {
          await invokeParsed("keep_my_version", unitSchema, { sourceId: item.sourceId, localId: item.localId });
          setInfo(infoText(`Agent Plugins no longer manages ${item.name}. Your copy stays as it is.`));
        });
      },
      onForceRemove: async (item, componentId) => {
        if (!(await reviewForceRemove(item.name))) {
          return;
        }
        await onCard(item, `Couldn't remove ${item.name}.`, async () => {
          const outcome = await retrying(() => invokeParsed("uninstall_item", operationOutcomeSchema, itemCommandArgs(item, componentId, { force: true })));
          const notice = outcomeNotice("Your changed copy was backed up to", outcome);
          if (notice !== null) {
            setInfo(notice);
          }
        });
      },
      onHold: async (item, held) =>
        onCard(item, `Couldn't change updates for ${item.name}.`, async () => {
          await invokeParsed("set_held", unitSchema, { sourceId: item.sourceId, localId: item.localId, held });
          setInfo(infoText(held ? `${item.name} stays as it is until you update it yourself.` : `${item.name} updates on its own again.`));
        }),
      onApps: (item, componentId) => {
        setAppsRequest({ item, componentId });
      },
      onSettings: (item) => {
        settle(editConnectorSettings(item));
      }
    };
  }

  async function saveApps(request: AppsRequest, excluded: readonly string[], added: boolean): Promise<void> {
    setAppsRequest(null);
    const { item, componentId } = request;
    // Adding an app back installs the connector there, which the person approves like any install.
    // Every app the connector goes to after the change that it isn't in now: the ones ticked again,
    // and any it was never in (an app found again after it went away).
    const excludedBefore = item.components.find((component) => component.id === componentId)?.excludedApps ?? [];
    const connector = item.connectors.find((entry) => entry.componentId === componentId);
    const readded = (state?.agentProfiles ?? []).filter((profile) => excludedBefore.includes(profile.targetId) && !excluded.includes(profile.targetId)).map((profile) => profile.displayName);
    const addingApps = [...new Set([...(connector?.apps ?? []), ...readded])].filter((app) => !(connector?.installedApps ?? []).includes(app));
    if (added && !(await approveConnectors([{ item, componentId, addingApps }]))) {
      return;
    }
    await onCard(item, `Couldn't change which apps use ${item.name}.`, async () => {
      const outcome = await retrying(() =>
        invokeParsed("set_excluded_apps", operationOutcomeSchema, {
          sourceId: item.sourceId,
          localId: item.localId,
          componentId,
          excluded,
          trustApproved: added,
          shown: added ? shownDigests([item]) : null
        })
      );
      const notice = outcomeNotice("The files that were there before were backed up to", outcome);
      setInfo(notice ?? infoText(`Saved. Restart the AI apps you changed so they notice.`));
    });
  }

  /**
   * Asks before a batch installs any connector among `eligible`. Null means
   * the person said no; otherwise whether the batch carries that approval.
   */
  /** The approval a batch sends: none needed, the packages the dialog showed, or null when the person declined. */
  async function connectorApproval(action: BulkAction, eligible: readonly BulkPlanEntry[]): Promise<{ trustApproved: boolean; shown: Record<string, string> | null } | null> {
    const needApproval = action === "uninstall" ? [] : (state?.items ?? []).filter((item) => item.requiresApproval && eligible.some((entry) => entry.id === item.id));
    if (needApproval.length === 0) {
      return { trustApproved: false, shown: null };
    }
    return (await approveConnectors(needApproval.map((item) => ({ item, componentId: null })))) ? { trustApproved: true, shown: shownDigests(needApproval) } : null;
  }

  function showBatchResult(verb: string, result: BulkResult, from?: string): void {
    if (result.failures.length > 0) {
      showActionError(failuresError(verb, result.failures, state?.items ?? [], from));
    }
    if (result.backupPaths.length > 0) {
      setInfo(backupNotice("The files that were there before were backed up to", result.backupPaths));
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
      if (!(await reviewBulk(source, action, plan))) {
        return;
      }
      const approval = await connectorApproval(action, eligible);
      if (approval === null) {
        return;
      }
      showBatchResult(verb, await retrying(() => invokeParsed("run_bulk_items", bulkResultSchema, { sourceId: source.sourceId, action, ...approval })), source.name);
    } catch (reason) {
      showActionError(toAppError(reason, `Couldn't ${verb} packages from ${source.name}.`));
    } finally {
      setBusySources((current) => marked(current, source.sourceId, null));
      await refreshAfterOperation();
    }
  }

  /** Installs or uninstalls a bundle's members that this window lists, as one batch. */
  async function runBundle(bundle: BundleState, action: BulkAction): Promise<void> {
    const verb = bulkLabels(action).action.toLowerCase();
    const ids = bundleMembers(bundle, state?.items ?? []).map((item) => item.id);
    setError(null);
    setInfo(null);
    setBusyBundles((current) => marked(current, bundle.id, action));
    try {
      const plan = await retrying(() => invokeParsed("plan_items", itemsPlanSchema, { ids, action }));
      const eligible = plan.entries.filter((entry) => entry.willRun);
      if (eligible.length === 0) {
        setInfo(infoText(`Nothing to ${verb} in ${bundle.name} right now.`));
        return;
      }
      if (
        action === "uninstall" &&
        !(await reviewBundleUninstall(
          bundle.name,
          eligible.map((entry) => packageName(state?.items ?? [], entry.id))
        ))
      ) {
        return;
      }
      const approval = await connectorApproval(action, eligible);
      if (approval === null) {
        return;
      }
      showBatchResult(verb, await retrying(() => invokeParsed("run_items", bulkResultSchema, { ids, action, ...approval })));
    } catch (reason) {
      showActionError(toAppError(reason, `Couldn't ${verb} ${bundle.name}.`));
    } finally {
      setBusyBundles((current) => marked(current, bundle.id, null));
      await refreshAfterOperation();
    }
  }

  async function deleteBundle(bundle: BundleState): Promise<void> {
    if (!(await reviewBundleDelete(bundle.name))) {
      return;
    }
    setBusyBundles((current) => marked(current, bundle.id, "remove"));
    try {
      await invokeParsed("delete_bundle", unitSchema, { namespace: bundle.namespace, bundleId: bundle.bundleId });
      settle(synchronize());
    } catch (reason) {
      showActionError(toAppError(reason, `Couldn't delete ${bundle.name}.`));
    } finally {
      setBusyBundles((current) => marked(current, bundle.id, null));
    }
  }

  /** The person confirmed a link: install through the same path a click on the card takes. */
  function installFromLink(target: LinkTarget): void {
    setLinkRequest(null);
    if (target.kind === "item") {
      settle(changeItem(target.item, target.componentId ?? undefined));
    } else {
      settle(runBundle(target.bundle, "install"));
    }
  }

  /** After a pasted link: a team invite just needs a refresh; a share opens what was shared. */
  function linkRedeemed(result: LinkResult): void {
    const slash = result.target.indexOf("/");
    if (result.kind === "invite") {
      setInfo(infoText(`You joined ${result.name}. You can publish skills to it now.`));
      settle(synchronize());
    } else if (slash < 0) {
      setInfo(infoText(`${result.name} is in your list now.`));
      settle(
        synchronize().then(() => {
          reveal(result.target);
        })
      );
    } else {
      // A saved copy may already list it; checking again replaces that copy with the one now shared.
      settle(openLink({ kind: "install", namespace: result.target.slice(0, slash), id: result.target.slice(slash + 1), component: null }, true));
    }
  }

  function openShare(target: string, label: string): void {
    setTeamsOpen(false);
    setShareTarget({ target, label });
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

  /** Opens the app picked in the picker with the tutorial or create-a-skill prompt; the backend says what to do next. */
  async function openWithPrompt(purpose: PromptPurpose, app: PromptApp): Promise<void> {
    const tutorial = purpose === "tutorial";
    setPromptRunning(true);
    setError(null);
    setInfo(null);
    try {
      setInfo(infoText(await invokeParsed(tutorial ? "run_tutorial" : "create_skill", textSchema, { app: app.id })));
    } catch (reason) {
      showActionError(toAppError(reason, tutorial ? `Couldn't start the tutorial in ${app.label}.` : `Couldn't open ${app.label}.`));
    } finally {
      setPromptRunning(false);
      setPromptPurpose(null);
      if (tutorial) {
        await refreshAfterOperation();
      }
    }
  }

  async function dismissTutorial(): Promise<void> {
    setState((current) => (current === null ? current : { ...current, tutorialOffered: false }));
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
  const noMatches = noMatchesText(deferredQuery, driftOnly, visibleSources.length, catalogFilter !== "all");
  const matchCount = [...itemsBySource.values()].reduce((total, items) => total + items.length, 0);
  // With nothing loaded yet, a failure replaces the spinner instead of sitting above it forever.
  const loadFailed = state === null && error !== null && !syncing;
  const report = visibleReport(lastReport, dismissedReport);
  const { items, profiles, promptApps } = catalogLists(state);
  // Cards are memoized, so everything they receive keeps its identity across renders.
  const onItemChange = useStableCallback(changeItem);
  const onManualChange = useStableCallback(changeManualInvocation);
  const onBulk = useStableCallback(runBulk);
  const onRunBundle = useStableCallback(runBundle);
  const onDeleteBundle = useStableCallback(deleteBundle);
  const onEditBundle = useStableCallback((bundle: BundleState): void => {
    setBundleEdit({ bundle });
  });
  const onShare = useStableCallback(openShare);
  const onActionError = useStableCallback(showActionError);
  const latestExtras = useStableCallback((): CardExtras => cardExtrasFor(view.marketplaceUrl));
  const hasPortal = view.marketplaceUrl !== null;
  const cardExtras = useMemo<CardExtras>(
    () => ({
      onDetails: hasPortal
        ? (item) => {
            latestExtras().onDetails?.(item);
          }
        : null,
      onKeepMine: async (item) => latestExtras().onKeepMine(item),
      onForceRemove: async (item, componentId) => latestExtras().onForceRemove(item, componentId),
      onHold: async (item, held) => latestExtras().onHold(item, held),
      onApps: (item, componentId) => {
        latestExtras().onApps(item, componentId);
      },
      onSettings: (item) => {
        latestExtras().onSettings(item);
      }
    }),
    [hasPortal, latestExtras]
  );
  return (
    <main className="app-shell">
      <header className="app-header">
        <div>
          <Heading as="h1" size="7">
            Agent Plugins
          </Heading>
        </div>
        <HeaderActions
          view={view}
          canPromptApp={promptApps.length > 0}
          syncing={syncing}
          resetting={resetting}
          onCreateSkill={() => {
            setPromptPurpose("create");
          }}
          onTeams={() => {
            setTeamsOpen(true);
          }}
          onStatus={() => {
            setStatusDialogOpen(true);
          }}
          onRefresh={() => {
            settle(synchronize());
          }}
          onError={showActionError}
        />
      </header>
      <SyncMeta
        checked={checked}
        checking={isChecking(state, syncing)}
        problems={view.problems}
        blocked={view.blocked}
        newVersion={newVersionAvailable(view.preflight)}
        onGetNewVersion={() => {
          handleStatusAction("update");
        }}
      />
      <CatalogToolbar
        query={query}
        matches={matchCount}
        driftOnly={driftOnly}
        onQueryChange={setQuery}
        onClearDrift={() => {
          setDriftOnly(false);
        }}
      >
        <CatalogFilters
          filter={catalogFilter}
          sort={catalogSort}
          onFilterChange={(filter) => {
            setCatalogFilter(asFilter(filter));
          }}
          onSortChange={(sort) => {
            setCatalogSort(asSort(sort));
          }}
        />
        <CatalogActions
          canCreate={view.identity !== null}
          onOpenLink={() => {
            setOpenLinkOpen(true);
          }}
          onNewBundle={() => {
            setBundleEdit({ bundle: null });
          }}
        />
      </CatalogToolbar>
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
        visible={state?.tutorialOffered === true && promptApps.length > 0}
        onStart={() => {
          setPromptPurpose("tutorial");
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
              setCatalogFilter("all");
            }}
          >
            Show all
          </Button>
        </div>
      ) : (
        <CardExtrasContext.Provider value={cardExtras}>
          <div className="sources-list">
            <BundleGroup
              bundles={visibleBundles}
              items={items}
              identity={view.identity}
              busyBundles={busyBundles}
              busyIds={busyItems}
              allBusy={resetting}
              onRun={onRunBundle}
              onEdit={onEditBundle}
              onDelete={onDeleteBundle}
              onShare={onShare}
              onItemChange={onItemChange}
              onManualChange={onManualChange}
              onError={onActionError}
            />
            {visibleSources.map((source) => (
              <SourceGroup
                key={source.sourceKey}
                source={source}
                items={itemsBySource.get(source.sourceKey) ?? []}
                busyIds={busyItems}
                allBusy={resetting || busySources.has(source.sourceId)}
                running={busySources.get(source.sourceId) ?? null}
                filtering={filtering}
                onItemChange={onItemChange}
                onManualChange={onManualChange}
                onBulk={onBulk}
                onShare={ownsSpace(view.identity, source.sourceId) ? onShare : undefined}
                onError={onActionError}
              />
            ))}
          </div>
        </CardExtrasContext.Provider>
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
      <TeamsDialog
        open={teamsOpen}
        identity={view.identity}
        onOpenChange={setTeamsOpen}
        onChanged={() => {
          settle(synchronize());
        }}
        onShare={openShare}
        onOpenLink={() => {
          setTeamsOpen(false);
          setOpenLinkOpen(true);
        }}
      />
      <OpenLinkDialog open={openLinkOpen} onOpenChange={setOpenLinkOpen} onRedeemed={linkRedeemed} />
      <ShareDialog
        request={shareTarget}
        onClose={() => {
          setShareTarget(null);
        }}
        onSaved={() => {
          settle(synchronize());
        }}
      />
      <BundleDialog
        request={bundleEdit}
        identity={view.identity}
        items={items}
        onClose={() => {
          setBundleEdit(null);
        }}
        onSaved={() => {
          settle(synchronize());
        }}
      />
      <LinkInstallDialog
        request={linkRequest}
        profiles={profiles}
        onCancel={() => {
          setLinkRequest(null);
        }}
        onInstall={installFromLink}
        onShow={reveal}
      />
      <ApprovalDialog request={approvalRequest} onResolve={answerConnectors} />
      <PromptAppDialog
        purpose={promptPurpose}
        apps={promptApps}
        running={promptRunning}
        onOpen={(purpose, app) => {
          settle(openWithPrompt(purpose, app));
        }}
        onClose={() => {
          setPromptPurpose(null);
        }}
      />
      <AppsDialog
        request={appsRequest}
        profiles={profiles}
        onSave={(request, excluded, added) => {
          settle(saveApps(request, excluded, added));
        }}
        onClose={() => {
          setAppsRequest(null);
        }}
      />
      <SystemStatusDialog
        open={statusDialogOpen}
        logPath={view.logPath}
        resetting={resetting}
        onManageSources={() => {
          setStatusDialogOpen(false);
          setSourceDialogOpen(true);
        }}
        onReset={() => {
          setStatusDialogOpen(false);
          settle(resetApp());
        }}
        onOpenLog={(path) => {
          revealItemInDir(path).catch((reason: unknown) => {
            showActionError(toAppError(reason, "Couldn't open the log folder."));
          });
        }}
        report={view.preflight}
        identity={view.identity}
        marketplaceUrl={view.marketplaceUrl}
        profiles={profiles}
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

/**
 * Header buttons that need a marketplace account: Teams, and a nudge when
 * suggested changes wait for this person in the portal.
 */
function MarketplaceButtons({
  identity,
  marketplaceUrl,
  disabled,
  onTeams,
  onError
}: Readonly<{ identity: AppIdentity | null; marketplaceUrl: string | null; disabled: boolean; onTeams: () => void; onError: (error: AppError) => void }>): JSX.Element | null {
  if (identity === null) {
    return null;
  }
  // Suggestions and reports for this person's skills, and anything else the marketplace told them.
  const waiting = identity.suggestionsWaiting + identity.reportsWaiting + identity.unreadNotifications;
  return (
    <>
      {waiting === 0 || marketplaceUrl === null ? null : (
        <Button
          variant="soft"
          color="amber"
          onClick={() => {
            openUrl(portalUrl(marketplaceUrl, identity.unreadNotifications > 0 ? "/notifications" : "/mine")).catch((reason: unknown) => {
              onError(toAppError(reason, "Couldn't open the marketplace."));
            });
          }}
        >
          <Badge color="amber" variant="solid">
            {String(waiting)}
          </Badge>
          waiting for you
        </Button>
      )}
      <Button variant="soft" disabled={disabled} onClick={onTeams}>
        Teams
      </Button>
    </>
  );
}

/** The header's buttons: making a skill, marketplace news and teams, status, help, and refresh. */
function HeaderActions({
  view,
  canPromptApp,
  syncing,
  resetting,
  onCreateSkill,
  onTeams,
  onStatus,
  onRefresh,
  onError
}: Readonly<{
  view: MarketplaceView;
  /** Some detected app can be opened with a prompt, so **Create a skill** asks which one. */
  canPromptApp: boolean;
  syncing: boolean;
  resetting: boolean;
  onCreateSkill: () => void;
  onTeams: () => void;
  onStatus: () => void;
  onRefresh: () => void;
  onError: (error: AppError) => void;
}>): JSX.Element {
  const portal = view.marketplaceUrl;
  return (
    <div className="catalog-actions">
      {canPromptApp ? (
        <Button variant="soft" onClick={onCreateSkill}>
          Create a skill
        </Button>
      ) : portal === null ? null : (
        // With no app to open, the skill is written in the portal instead.
        <PortalButton label="Create a skill" url={portalUrl(portal, "/publish")} onError={onError} />
      )}
      <MarketplaceButtons identity={view.identity} marketplaceUrl={portal} disabled={resetting} onTeams={onTeams} onError={onError} />
      <StatusButton problems={view.problems} disabled={resetting} onClick={onStatus} />
      {portal === null ? null : <PortalButton label="Help" url={portalUrl(portal, "/help/getting-started")} onError={onError} />}
      <Button loading={syncing} disabled={syncing || resetting} onClick={onRefresh}>
        Refresh
      </Button>
    </div>
  );
}

/** Whether the marketplace says a newer Agent Plugins is out. */
function newVersionAvailable(report: PreflightReport | null): boolean {
  return report?.checks.some((check) => check.id === "server.clientVersion" && check.status === "warn") === true;
}

/** A header button that opens a page of the marketplace portal. */
function PortalButton({ label, url, onError }: Readonly<{ label: string; url: string; onError: (error: AppError) => void }>): JSX.Element {
  return (
    <Button
      variant="soft"
      onClick={() => {
        openUrl(url).catch((reason: unknown) => {
          onError(toAppError(reason, "Couldn't open the marketplace."));
        });
      }}
    >
      {label}
    </Button>
  );
}

function catalogLists(state: AppState | null): Readonly<{ items: readonly CatalogItem[]; profiles: readonly AgentProfile[]; promptApps: readonly PromptApp[] }> {
  return { items: state?.items ?? [], profiles: state?.agentProfiles ?? [], promptApps: state?.promptApps ?? [] };
}

/** Beside the search box: open a link someone sent, and start a bundle when signed in to the marketplace. */
function CatalogActions({ canCreate, onOpenLink, onNewBundle }: Readonly<{ canCreate: boolean; onOpenLink: () => void; onNewBundle: () => void }>): JSX.Element {
  return (
    <>
      <Button size="1" variant="soft" onClick={onOpenLink}>
        Open a link
      </Button>
      {canCreate ? (
        <Button size="1" variant="soft" onClick={onNewBundle}>
          New bundle
        </Button>
      ) : null}
    </>
  );
}

/** The background-update report, unless the person already dismissed this exact one. */
function visibleReport(report: ReportNotice | null, dismissed: string | null): ReportNotice | null {
  return report === null || report.text === dismissed ? null : report;
}

function LoadingOrFailed({ error, onRetry }: Readonly<{ error: AppError | null; onRetry: () => void }>): JSX.Element {
  if (error === null) {
    return (
      <div className="loading-state">
        <Spinner size="3" />
        <Text color="gray">Loading skills…</Text>
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

function matchesQuery(item: CatalogItem, query: string): boolean {
  return matchesAllWords([item.id, item.name, item.description, item.sourceName, item.marketplace?.publisher ?? "", ...(item.marketplace?.tags ?? [])].join(" "), query);
}

function usersOf(item: CatalogItem): number {
  return item.marketplace?.installedBase ?? 0;
}

const FILTERS: readonly CatalogFilter[] = ["all", "installed", "updates", "official", "team", "shared"];

function asFilter(value: string): CatalogFilter {
  return FILTERS.find((filter) => filter === value) ?? "all";
}

function asSort(value: string): CatalogSort {
  return value === "used" ? "used" : "name";
}

function matchesFilter(item: CatalogItem, filter: CatalogFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "installed":
      return item.status !== "available" && item.status !== "conflict" && item.status !== "sourceConflict";
    case "updates":
      return item.status === "updateAvailable" || item.status === "partiallyInstalled";
    case "official":
      return item.marketplace?.lane === "official";
    case "team":
      return item.marketplace?.lane === "team";
    case "shared":
      return item.marketplace?.sharedWithYou === true;
  }
}

type MarketplaceView = Readonly<{
  identity: AppIdentity | null;
  marketplaceUrl: string | null;
  preflight: PreflightReport | null;
  blocked: boolean;
  problems: readonly PreflightCheck[];
  logPath: string | null;
}>;

function marketplaceView(state: AppState | null, offline: boolean): MarketplaceView {
  const preflight = state?.preflight ?? null;
  return {
    identity: state?.identity ?? null,
    marketplaceUrl: state?.marketplaceUrl ?? null,
    preflight,
    blocked: preflight?.blocked === true,
    problems: headerProblems(seriousProblems(preflight), offline),
    logPath: state?.logPath ?? null
  };
}
