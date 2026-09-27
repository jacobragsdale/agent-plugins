import { z } from "zod";

/**
 * One malformed element must not hide the whole window, so these arrays keep
 * every element that parses and log the rest. Everything else at the IPC
 * boundary uses `z.object`, which ignores fields a newer backend adds.
 */
function tolerantArray<T extends z.ZodType>(element: T, label: string): z.ZodType<readonly z.output<T>[]> {
  return z.array(z.unknown()).transform((values) =>
    values.flatMap((value) => {
      const parsed = element.safeParse(value);
      if (parsed.success) {
        return [parsed.data];
      }
      console.warn(`Agent Plugins skipped a ${label} it could not read.`, z.prettifyError(parsed.error), value);
      return [];
    })
  );
}

export const itemStatusSchema = z.enum(["available", "installed", "updateAvailable", "removed", "modified", "conflict", "sourceConflict", "partiallyInstalled", "missing"]);
export const sourceStatusSchema = z.enum(["fresh", "cached", "stale", "error"]);
export const connectivitySchema = z.enum(["online", "offline", "degraded"]);
/** When a fetch last succeeded; null when it never has. */
const lastSuccessSchema = z.number().int().nonnegative().nullable().default(null);
export const catalogErrorSchema = z.object({ path: z.string().min(1), message: z.string().min(1) }).readonly();
export const targetIdSchema = z.enum(["github-copilot", "cursor", "claude-code", "claude-desktop", "opencode", "pi", "codex", "chatgpt", "grok-build"]);
export const agentProfileSchema = z
  .object({
    targetId: targetIdSchema,
    displayName: z.string().min(1),
    enabled: z.boolean(),
    scopes: z.array(z.string().min(1)).readonly(),
    dialectId: z.string().min(1),
    detected: z.boolean(),
    detectedVersion: z.string().min(1).nullable(),
    detectionMessage: z.string().min(1).nullable(),
    verificationGuidance: z.string().min(1),
    reloadGuidance: z.string().min(1),
    skillDirectory: z.string().min(1),
    skillDirectoryShared: z.boolean()
  })
  .readonly();
export const componentSchema = z
  .object({
    id: z.string().min(1),
    kind: z.string().min(1),
    description: z.string().min(1),
    manualInvocation: z.boolean(),
    status: itemStatusSchema.optional(),
    requiresApproval: z.boolean().default(false),
    /** Target ids the person kept this component out of. */
    excludedApps: z.array(z.string().min(1)).readonly().default([])
  })
  .readonly();
/** One MCP server as the approval prompt shows it. */
export const connectorSchema = z
  .object({
    componentId: z.string().min(1),
    name: z.string().min(1),
    summary: z.string().min(1),
    detail: z.string().min(1),
    environment: z.array(z.string().min(1)).readonly(),
    missingEnvironment: z.array(z.string().min(1)).readonly(),
    missingProgram: z.string().min(1).nullable(),
    apps: z.array(z.string().min(1)).readonly(),
    /** The apps it is in now; empty until it is installed. */
    installedApps: z.array(z.string().min(1)).readonly().default([]),
    changed: z.boolean()
  })
  .readonly();
export const capabilitySchema = z.discriminatedUnion("level", [
  z.object({ level: z.literal("native") }).readonly(),
  z.object({ level: z.literal("losslessTranslation") }).readonly(),
  z.object({ level: z.literal("lossyTranslation"), losses: z.array(z.string().min(1)).readonly() }).readonly(),
  z.object({ level: z.literal("unsupported"), reason: z.string().min(1) }).readonly(),
  z.object({ level: z.literal("blocked"), reason: z.string().min(1), requiredAction: z.string().min(1) }).readonly()
]);
export const compatibilitySchema = z.object({ componentId: z.string().min(1), targetId: z.string().min(1), capability: capabilitySchema }).readonly();
export const marketplaceMetaSchema = z
  .object({
    publisher: z.string().min(1),
    publisherAccount: z.string().min(1),
    version: z.string().min(1),
    lane: z.string().min(1),
    tags: z.array(z.string().min(1)).readonly(),
    publishedAt: z.string().min(1),
    installs: z.number().int().nonnegative(),
    installedBase: z.number().int().nonnegative(),
    restricted: z.boolean(),
    /** Visible only because someone shared it with this person. */
    sharedWithYou: z.boolean().default(false),
    changelog: z.string().min(1).nullable().default(null),
    /** Whether an admin let everyone see its MCP server; null without one. */
    mcpApproved: z.boolean().nullable().default(null)
  })
  .readonly();
export const itemSchema = z
  .object({
    id: z.string().min(3),
    localId: z.string().min(1),
    sourceId: z.string().min(2),
    sourceKey: z.string().min(1),
    sourceName: z.string().min(1),
    sourceUrl: z.string().min(1),
    name: z.string().min(1),
    description: z.string().min(1),
    manualInvocation: z.boolean(),
    source: z.string().min(1),
    sourceIsDirectory: z.boolean(),
    manifestVersion: z.number().int().min(1).max(2),
    components: z.array(componentSchema).readonly(),
    compatibility: z.array(compatibilitySchema).readonly(),
    destination: z.string().min(1).nullable(),
    status: itemStatusSchema,
    requiresApproval: z.boolean().default(false),
    riskDetails: z.array(z.string().min(1)).readonly().default([]),
    connectors: z.array(connectorSchema).readonly().default([]),
    /** The person holds this package's background updates. */
    /** The package's content digest; an approval sends back the one its dialog showed. */
    digest: z.string().default(""),
    held: z.boolean().default(false),
    marketplace: marketplaceMetaSchema.nullable().default(null)
  })
  .transform((item) => ({
    ...item,
    components: item.components.map((component) => ({
      id: component.id,
      kind: component.kind,
      description: component.description,
      manualInvocation: component.manualInvocation,
      status: component.status ?? item.status,
      requiresApproval: component.requiresApproval,
      excludedApps: component.excludedApps
    }))
  }))
  .readonly();
export const sourceSchema = z
  .object({
    sourceId: z.string().min(2),
    sourceKey: z.string().min(1),
    name: z.string().min(1),
    description: z.string().min(1),
    url: z.string().min(1),
    repositoryKey: z.string().min(1).nullable().default(null),
    status: sourceStatusSchema,
    refreshFailed: z.boolean(),
    message: z.string().min(1).nullable(),
    commit: z.string().min(1).nullable(),
    checkedAtEpochSeconds: z.number().int().nonnegative(),
    lastSuccessAtEpochSeconds: lastSuccessSchema,
    catalogErrors: z.array(catalogErrorSchema).readonly()
  })
  .readonly();
export const listedSourceSchema = z
  .object({ name: z.string().min(1), description: z.string().min(1), url: z.string().min(1), sourceId: z.string().min(2).nullable(), alreadyAdded: z.boolean() })
  .readonly();
export const repositorySchema = z
  .object({
    repositoryId: z.string().min(2),
    repositoryKey: z.string().min(1),
    name: z.string().min(1),
    description: z.string().min(1),
    url: z.string().min(1),
    status: sourceStatusSchema,
    refreshFailed: z.boolean(),
    message: z.string().min(1).nullable(),
    revision: z.string().min(1).nullable(),
    checkedAtEpochSeconds: z.number().int().nonnegative(),
    lastSuccessAtEpochSeconds: lastSuccessSchema,
    sources: z.array(listedSourceSchema).readonly()
  })
  .readonly();
export const itemReferenceSchema = z
  .object({ id: z.string().min(1), sourceId: z.string().min(2), localId: z.string().min(1), fromVersion: z.string().min(1).optional(), toVersion: z.string().min(1).optional() })
  .readonly();
export const itemFailureSchema = z.object({ id: z.string().min(1), message: z.string().min(1) }).readonly();
export const autoUpdateReportSchema = z
  .object({
    updatedItems: z.array(itemReferenceSchema).readonly(),
    failedItems: z.array(itemFailureSchema).readonly(),
    /** Display names of installed packages whose missing files the sync put back. */
    repairedItems: z.array(z.string().min(1)).readonly().default([]),
    /** Display names of installed packages the sync added to newly found AI apps. */
    extendedItems: z.array(z.string().min(1)).readonly().default([]),
    /** Display names of packages the sync uninstalled because their publisher or an admin pulled them. */
    removedItems: z.array(z.string().min(1)).readonly().default([]),
    /** Display names of installed packages the sync took out of AI apps no longer found, or kept out. */
    releasedItems: z.array(z.string().min(1)).readonly().default([]),
    /** A pulled package still in an app whose settings file was busy or damaged: why, one line each. */
    stillPulled: z.array(z.string().min(1)).readonly().default([])
  })
  .readonly();
export const teamRefSchema = z.object({ namespace: z.string().min(1), displayName: z.string().min(1), owner: z.boolean() }).readonly();
export const identitySchema = z
  .object({
    account: z.string().min(1),
    namespace: z.string().min(1),
    displayName: z.string().min(1),
    admin: z.boolean(),
    authMode: z.string().min(1),
    /** Every space this person may publish to: their own, teams, and `official` when allowed. */
    namespaces: z.array(z.string().min(1)).readonly().default([]),
    teams: z.array(teamRefSchema).readonly().default([]),
    /** Suggested changes to their packages that wait for them in the portal. */
    suggestionsWaiting: z.number().int().nonnegative().default(0),
    /** Open reports and feedback on their packages. */
    reportsWaiting: z.number().int().nonnegative().default(0),
    unreadNotifications: z.number().int().nonnegative().default(0)
  })
  .readonly();
/** Marketplace news that arrived since the last sync; `link` is a portal path. */
export const notificationSchema = z.object({ id: z.number().int().nonnegative(), kind: z.string(), text: z.string().min(1), link: z.string().min(1).nullable().default(null) }).readonly();
/** A group of existing packages people install together; `members` are canonical ids. */
export const bundleStateSchema = z
  .object({
    id: z.string().min(3),
    namespace: z.string().min(2),
    bundleId: z.string().min(1),
    name: z.string().min(1),
    description: z.string(),
    publisher: z.string().min(1),
    lane: z.string().min(1),
    members: z.array(z.string().min(3)).readonly(),
    updatedAt: z.string().min(1),
    restricted: z.boolean().default(false),
    sharedWithYou: z.boolean().default(false)
  })
  .readonly();
export const checkStatusSchema = z.enum(["ok", "warn", "fail", "skipped"]);
export const remediationSchema = z
  .discriminatedUnion("kind", [
    z.object({ kind: z.literal("autoFixed") }),
    z.object({ kind: z.literal("action"), action: z.string().min(1) }),
    z.object({ kind: z.literal("manual"), text: z.string().min(1) })
  ])
  .nullable();
export const preflightCheckSchema = z
  .object({
    id: z.string().min(1),
    group: z.string().min(1),
    title: z.string().min(1),
    status: checkStatusSchema,
    detail: z.string(),
    remediation: remediationSchema,
    blocking: z.boolean(),
    durationMillis: z.number().int().nonnegative()
  })
  .readonly();
export const preflightReportSchema = z
  .object({
    startedAtEpochSeconds: z.number().int().nonnegative(),
    durationMillis: z.number().int().nonnegative(),
    blocked: z.boolean(),
    authMode: z.string().min(1),
    checks: z.array(preflightCheckSchema).readonly()
  })
  .readonly();
export const appStateSchema = z
  .object({
    /** The last sync that reached the servers; 0 when none has yet. */
    checkedAtEpochSeconds: z.number().int().nonnegative(),
    connectivity: connectivitySchema.default("online"),
    syncInProgress: z.boolean().default(false),
    autoUpdateReport: autoUpdateReportSchema,
    catalogMessage: z.string().min(1).nullable().default(null),
    repositories: z.array(repositorySchema).readonly().default([]),
    sources: tolerantArray(sourceSchema, "source"),
    items: tolerantArray(itemSchema, "package"),
    bundles: tolerantArray(bundleStateSchema, "bundle").default([]),
    agentProfiles: z.array(agentProfileSchema).readonly(),
    /** The detected app the skill tutorial can demonstrate, until it has run. */
    tutorial: targetIdSchema.nullable().default(null),
    marketplaceUrl: z.string().min(1).nullable().default(null),
    downloadUrl: z.string().min(1).nullable().default(null),
    identity: identitySchema.nullable().default(null),
    preflight: preflightReportSchema.nullable().default(null),
    notifications: tolerantArray(notificationSchema, "notification").default([]),
    /** The app's log file, for "Open logs". */
    logPath: z.string().min(1).nullable().default(null)
  })
  .readonly();
export const preparedSourceSchema = z
  .object({
    token: z.string().min(1),
    sourceId: z.string().min(2),
    sourceKey: z.string().min(1),
    name: z.string().min(1),
    description: z.string().min(1),
    url: z.string().min(1),
    commit: z.string().min(1),
    itemCount: z.number().int().nonnegative()
  })
  .readonly();
/** `warnings` names AI apps a package skipped, such as one whose settings file could not be read. */
export const operationOutcomeSchema = z.object({ backupPaths: z.array(z.string().min(1)).readonly(), warnings: z.array(z.string().min(1)).readonly().default([]) }).readonly();
export const bulkActionSchema = z.enum(["install", "replace", "uninstall"]);
export const bulkPlanEntrySchema = z.object({ id: z.string().min(1), localId: z.string().min(1), status: itemStatusSchema, willRun: z.boolean() }).readonly();
export const bulkPlanSchema = z.object({ sourceId: z.string().min(2), action: bulkActionSchema, entries: z.array(bulkPlanEntrySchema).readonly() }).readonly();
export const itemsPlanSchema = z.object({ action: bulkActionSchema, entries: z.array(bulkPlanEntrySchema).readonly() }).readonly();
export const bulkFailureSchema = z.object({ id: z.string().min(1), message: z.string().min(1) }).readonly();
export const bulkResultSchema = z
  .object({ completed: z.array(z.string().min(1)).readonly(), failures: z.array(bulkFailureSchema).readonly(), backupPaths: z.array(z.string().min(1)).readonly() })
  .readonly();
export const removalPathSchema = z.object({ path: z.string().min(1), modified: z.boolean() }).readonly();
export const removalItemSchema = z.object({ id: z.string().min(1), paths: z.array(removalPathSchema).readonly() }).readonly();
export const sourceRemovalPlanSchema = z.object({ sourceId: z.string().min(2), items: z.array(removalItemSchema).readonly() }).readonly();
/**
 * A command rejects with either a plain string (older backends) or a typed
 * error the backend classifies. The kind says what the app can do about it on its own.
 */
export const ipcErrorKindSchema = z.enum(["offline", "locked", "retryable", "needsUser", "bug"]);
export const ipcErrorSchema = z.object({ kind: ipcErrorKindSchema, message: z.string().min(1), detail: z.string().min(1).optional() }).readonly();
export const scheduledSyncSchema = z.union([z.object({ kind: z.literal("updated"), state: appStateSchema }).readonly(), z.object({ kind: z.literal("failed"), error: ipcErrorSchema }).readonly()]);
export const cachedStateSchema = appStateSchema.nullable();
export const unitSchema = z.null();
export const textSchema = z.string().min(1);

/** An `agent-plugins://` link the app was opened with. */
export const deepLinkSchema = z.object({ kind: z.enum(["install", "open"]), namespace: z.string().min(2), id: z.string().min(1), component: z.string().min(1).nullable() }).readonly();
export const pendingLinkSchema = deepLinkSchema.nullable();
const visibilitySchema = z.enum(["public", "private"]);
const personSchema = z.object({ account: z.string().min(1), displayName: z.string().min(1) }).readonly();
const teamNameSchema = z.object({ namespace: z.string().min(1), displayName: z.string().min(1) }).readonly();
export const teamSummarySchema = z
  .object({ namespace: z.string().min(1), displayName: z.string().min(1), visibility: visibilitySchema, role: z.enum(["owner", "member", "admin"]), memberCount: z.number().int().nonnegative() })
  .readonly();
export const teamSummariesSchema = z.array(teamSummarySchema).readonly();
export const teamSchema = z
  .object({
    namespace: z.string().min(1),
    displayName: z.string().min(1),
    visibility: visibilitySchema,
    role: z.enum(["owner", "member", "admin"]),
    members: z.array(z.object({ account: z.string().min(1), displayName: z.string().min(1), owner: z.boolean(), joinedAt: z.string().min(1) }).readonly()).readonly(),
    invite: z.string().min(1).nullable()
  })
  .readonly();
export const directorySchema = z.object({ people: z.array(personSchema).readonly(), teams: z.array(teamNameSchema).readonly() }).readonly();
export const linkPreviewSchema = z
  .object({ kind: z.enum(["invite", "share"]), targetKind: z.enum(["team", "space", "package", "bundle"]), target: z.string().min(1), name: z.string().min(1), by: z.string().min(1) })
  .readonly();
export const linkResultSchema = z
  .object({ kind: z.enum(["invite", "share"]), targetKind: z.enum(["team", "space", "package", "bundle"]), target: z.string().min(1), name: z.string().min(1), changed: z.boolean() })
  .readonly();
export const shareSchema = z
  .object({
    target: z.string().min(1),
    visibility: z.enum(["inherit", "public", "private"]),
    effective: visibilitySchema,
    users: z.array(personSchema).readonly(),
    teams: z.array(teamNameSchema).readonly(),
    groups: z.array(z.string().min(1)).readonly(),
    link: z.string().min(1).nullable()
  })
  .readonly();
export const savedBundleSchema = z.object({ id: z.string().min(3), name: z.string().min(1) }).readonly();

export type AppState = z.infer<typeof appStateSchema>;
export type CatalogItem = z.infer<typeof itemSchema>;
export type CatalogComponent = CatalogItem["components"][number];
export type Connector = z.infer<typeof connectorSchema>;
export type MarketplaceNotification = z.infer<typeof notificationSchema>;
export type ItemStatus = z.infer<typeof itemStatusSchema>;
export type IpcErrorKind = z.infer<typeof ipcErrorKindSchema>;
export type SourceStatus = z.infer<typeof sourceStatusSchema>;
export type Connectivity = z.infer<typeof connectivitySchema>;
export type OperationOutcome = z.infer<typeof operationOutcomeSchema>;
export type SourceState = z.infer<typeof sourceSchema>;
export type RepositoryState = z.infer<typeof repositorySchema>;
export type ListedSource = z.infer<typeof listedSourceSchema>;
export type BulkAction = z.infer<typeof bulkActionSchema>;
export type BulkPlan = z.infer<typeof bulkPlanSchema>;
export type BulkPlanEntry = z.infer<typeof bulkPlanEntrySchema>;
export type BulkResult = z.infer<typeof bulkResultSchema>;
export type AgentProfile = z.infer<typeof agentProfileSchema>;
export type TargetId = z.infer<typeof targetIdSchema>;
export type AppIdentity = z.infer<typeof identitySchema>;
export type BundleState = z.infer<typeof bundleStateSchema>;
export type DeepLink = z.infer<typeof deepLinkSchema>;
export type TeamSummary = z.infer<typeof teamSummarySchema>;
export type Team = z.infer<typeof teamSchema>;
export type Directory = z.infer<typeof directorySchema>;
export type LinkPreview = z.infer<typeof linkPreviewSchema>;
export type LinkResult = z.infer<typeof linkResultSchema>;
export type Share = z.infer<typeof shareSchema>;
export type SourceRemovalPlan = z.infer<typeof sourceRemovalPlanSchema>;
export type CheckStatus = z.infer<typeof checkStatusSchema>;
export type PreflightCheck = z.infer<typeof preflightCheckSchema>;
export type PreflightReport = z.infer<typeof preflightReportSchema>;
