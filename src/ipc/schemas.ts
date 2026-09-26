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
export const targetIdSchema = z.enum(["cursor", "claude-code", "codex", "opencode", "grok-build", "github-copilot", "claude-desktop", "chatgpt", "m365-copilot"]);
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
    requiresApproval: z.boolean().default(false)
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
    restricted: z.boolean()
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
      requiresApproval: component.requiresApproval
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
export const itemReferenceSchema = z.object({ id: z.string().min(1), sourceId: z.string().min(2), localId: z.string().min(1) }).readonly();
export const itemFailureSchema = z.object({ id: z.string().min(1), message: z.string().min(1) }).readonly();
export const autoUpdateReportSchema = z
  .object({
    updatedItems: z.array(itemReferenceSchema).readonly(),
    failedItems: z.array(itemFailureSchema).readonly(),
    /** Display names of installed packages whose missing files the sync put back. */
    repairedItems: z.array(z.string().min(1)).readonly().default([]),
    /** Display names of installed packages the sync added to newly found AI apps. */
    extendedItems: z.array(z.string().min(1)).readonly().default([])
  })
  .readonly();
export const identitySchema = z.object({ account: z.string().min(1), namespace: z.string().min(1), displayName: z.string().min(1), admin: z.boolean(), authMode: z.string().min(1) }).readonly();
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
    agentProfiles: z.array(agentProfileSchema).readonly(),
    /** The detected app the skill tutorial can demonstrate, until it has run. */
    tutorial: targetIdSchema.nullable().default(null),
    marketplaceUrl: z.string().min(1).nullable().default(null),
    downloadUrl: z.string().min(1).nullable().default(null),
    identity: identitySchema.nullable().default(null),
    preflight: preflightReportSchema.nullable().default(null)
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

export type AppState = z.infer<typeof appStateSchema>;
export type CatalogItem = z.infer<typeof itemSchema>;
export type CatalogComponent = CatalogItem["components"][number];
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
export type AgentProfile = z.infer<typeof agentProfileSchema>;
export type TargetId = z.infer<typeof targetIdSchema>;
export type AppIdentity = z.infer<typeof identitySchema>;
export type SourceRemovalPlan = z.infer<typeof sourceRemovalPlanSchema>;
export type CheckStatus = z.infer<typeof checkStatusSchema>;
export type PreflightCheck = z.infer<typeof preflightCheckSchema>;
export type PreflightReport = z.infer<typeof preflightReportSchema>;
