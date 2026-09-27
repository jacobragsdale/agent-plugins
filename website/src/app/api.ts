import { HttpClient, HttpErrorResponse } from "@angular/common/http";
import { Injectable, inject } from "@angular/core";
import type { Observable } from "rxjs";
import { firstValueFrom } from "rxjs";
import { z } from "zod";

// Every response is parsed from `unknown` here; the portal and the API ship in one image, so an
// unexpected field is a bug worth failing on (strict objects).

const timestamp = z.iso.datetime({ offset: true });
const stringList = z.array(z.string()).readonly();
const counts = z.record(z.string(), z.number().int()).readonly();

export const laneSchema = z.enum(["official", "team", "personal"]);
export type Lane = z.infer<typeof laneSchema>;

/** A space is public or private; a package or bundle may also follow its space ("inherit"). */
export const visibilitySchema = z.enum(["inherit", "public", "private"]);
export type Visibility = z.infer<typeof visibilitySchema>;
export const effectiveSchema = z.enum(["public", "private"]);

const publisherSchema = z.strictObject({ account: z.string(), displayName: z.string() }).readonly();

// Health is the first call every page makes, so a field a newer server adds must not break sign-in.
export const healthSchema = z
  .object({ serverVersion: z.string(), minimumClientVersion: z.string(), latestClientVersion: z.string(), environment: z.string(), authSchemes: stringList, adGroups: z.boolean() })
  .readonly();
export type Health = z.infer<typeof healthSchema>;

/** The desktop app's latest heartbeat: its version and the packages installed on that PC. */
export const appSchema = z.strictObject({ version: z.string(), os: z.string(), lastSeenAt: timestamp, installed: stringList }).readonly();
export type AppInfo = z.infer<typeof appSchema>;

export const meSchema = z
  .strictObject({
    account: z.string(),
    namespace: z.string(),
    displayName: z.string(),
    namespaces: stringList,
    admin: z.boolean(),
    groups: stringList,
    teams: z.array(z.strictObject({ namespace: z.string(), displayName: z.string(), owner: z.boolean() }).readonly()).readonly(),
    suggestionsWaiting: z.number().int(),
    reportsWaiting: z.number().int(),
    unreadNotifications: z.number().int(),
    app: appSchema.nullable()
  })
  .readonly();
export type Me = z.infer<typeof meSchema>;

export const indexPackageSchema = z
  .strictObject({
    id: z.string(),
    namespace: z.string(),
    packageId: z.string(),
    name: z.string(),
    description: z.string(),
    version: z.string(),
    publisher: publisherSchema,
    lane: laneSchema,
    tags: stringList,
    componentKinds: stringList,
    publishedAt: timestamp,
    createdAt: timestamp,
    changelog: z.string().nullable(),
    /** Null unless the live version has an MCP server; then whether an admin checked it for everyone. */
    mcpApproved: z.boolean().nullable(),
    /** `stdio`, `streamable-http`, `sse`; empty when there is no MCP server or it isn't known. */
    mcpTransports: stringList,
    installs: z.number().int(),
    installedBase: z.number().int(),
    restricted: z.boolean(),
    sharedWithYou: z.boolean()
  })
  .readonly();
export type IndexPackage = z.infer<typeof indexPackageSchema>;

export const indexBundleSchema = z
  .strictObject({
    id: z.string(),
    namespace: z.string(),
    bundleId: z.string(),
    name: z.string(),
    description: z.string(),
    publisher: publisherSchema,
    lane: laneSchema,
    members: stringList,
    updatedAt: timestamp,
    restricted: z.boolean(),
    sharedWithYou: z.boolean()
  })
  .readonly();
export type IndexBundle = z.infer<typeof indexBundleSchema>;

export const indexSchema = z.strictObject({ generatedAt: timestamp, packages: z.array(indexPackageSchema).readonly(), bundles: z.array(indexBundleSchema).readonly(), revoked: stringList }).readonly();
export type Index = z.infer<typeof indexSchema>;

export const versionSchema = z
  .strictObject({
    version: z.string(),
    archiveDigest: z.string(),
    sizeBytes: z.number().int(),
    publishedBy: z.string(),
    publishedAt: timestamp,
    changelog: z.string().nullable(),
    yanked: z.boolean(),
    /** An admin deleted its files for good; it stays listed so its number isn't reused. */
    purged: z.boolean(),
    componentKinds: stringList
  })
  .readonly();
export type PackageVersion = z.infer<typeof versionSchema>;

/** What an MCP server in a package runs or connects to, read from its files when it was published. */
export const mcpServerSchema = z
  .strictObject({ name: z.string(), transport: z.string(), command: z.string().nullish(), args: stringList, envNames: stringList, url: z.string().nullish(), headerNames: stringList })
  .readonly();
export type McpServer = z.infer<typeof mcpServerSchema>;

export const packageSchema = z
  .strictObject({
    id: z.string(),
    namespace: z.string(),
    packageId: z.string(),
    name: z.string(),
    description: z.string(),
    tags: stringList,
    liveVersion: z.string().nullable(),
    versions: z.array(versionSchema).readonly(),
    owned: z.boolean(),
    visibility: visibilitySchema,
    effective: effectiveSchema,
    sharedWithYou: z.boolean(),
    revoked: z.boolean(),
    /** An admin pulled it, so only an admin can restore it. */
    revokedByAdmin: z.boolean(),
    createdAt: timestamp,
    publisher: publisherSchema,
    lane: laneSchema,
    installs: z.number().int(),
    installedBase: z.number().int(),
    mcpServers: z.array(mcpServerSchema).readonly(),
    /** Only for a package with an MCP server: whether an admin let everyone see it. */
    publicReview: z
      .strictObject({ state: z.enum(["approved", "declined", "waiting"]), note: z.string().nullable() })
      .readonly()
      .nullable()
  })
  .readonly();
export type PackageDetail = z.infer<typeof packageSchema>;

export const spaceSchema = z.strictObject({ namespace: z.string(), displayName: z.string(), lane: laneSchema, visibility: effectiveSchema, role: z.enum(["owner", "member"]) }).readonly();
export type Space = z.infer<typeof spaceSchema>;

export const bundleSchema = z
  .strictObject({
    id: z.string(),
    namespace: z.string(),
    bundleId: z.string(),
    name: z.string(),
    description: z.string(),
    members: stringList,
    hiddenMembers: z.number().int(),
    publisher: publisherSchema,
    lane: laneSchema,
    updatedAt: timestamp,
    updatedBy: z.string(),
    owned: z.boolean(),
    visibility: visibilitySchema,
    effective: effectiveSchema
  })
  .readonly();
export type Bundle = z.infer<typeof bundleSchema>;

export const suggestionStateSchema = z.enum(["pending", "accepted", "declined", "withdrawn"]);
export type SuggestionState = z.infer<typeof suggestionStateSchema>;

export const suggestionSchema = z
  .strictObject({
    id: z.number().int(),
    target: z.string(),
    namespace: z.string(),
    packageId: z.string(),
    name: z.string(),
    baseVersion: z.string().nullable(),
    liveVersion: z.string().nullable(),
    message: z.string(),
    suggestedBy: z.string(),
    suggestedByName: z.string(),
    suggestedAt: timestamp,
    state: suggestionStateSchema,
    decidedBy: z.string().nullable(),
    decidedAt: timestamp.nullable(),
    note: z.string().nullable(),
    acceptedVersion: z.string().nullable(),
    sizeBytes: z.number().int(),
    componentKinds: stringList
  })
  .readonly();
export type Suggestion = z.infer<typeof suggestionSchema>;
const suggestionListSchema = z.array(suggestionSchema).readonly();

export const mineSchema = z
  .strictObject({
    spaces: z.array(spaceSchema).readonly(),
    packages: z.array(packageSchema).readonly(),
    bundles: z.array(bundleSchema).readonly(),
    suggestions: z.strictObject({ waiting: suggestionListSchema, yours: suggestionListSchema }).readonly()
  })
  .readonly();
export type Mine = z.infer<typeof mineSchema>;

export const fileStatusSchema = z.enum(["added", "changed", "removed", "unchanged"]);
export type FileStatus = z.infer<typeof fileStatusSchema>;

/** A file of a version, or of a suggestion with how it differs from the live version. */
export const packageFileSchema = z.strictObject({ path: z.string(), size: z.number().int(), status: fileStatusSchema.optional() }).readonly();
export type PackageFile = z.infer<typeof packageFileSchema>;
const fileListSchema = z.array(packageFileSchema).readonly();

export const publishedSchema = z
  .strictObject({
    id: z.string(),
    namespace: z.string(),
    packageId: z.string(),
    version: z.string(),
    archiveDigest: z.string(),
    sizeBytes: z.number().int(),
    publishedBy: z.string(),
    publishedAt: timestamp,
    componentKinds: stringList,
    tags: stringList,
    waitingForPublicReview: z.boolean(),
    warnings: stringList
  })
  .readonly();
export type Published = z.infer<typeof publishedSchema>;

/** A publish checked but not committed: each file against the live version, and anything worth knowing. */
export const dryRunSchema = z
  .strictObject({
    version: z.string(),
    componentKinds: stringList,
    sizeBytes: z.number().int(),
    warnings: stringList,
    files: z.array(z.strictObject({ path: z.string(), status: z.enum(["new", "changed", "removed", "same"]) }).readonly()).readonly()
  })
  .readonly();
export type DryRun = z.infer<typeof dryRunSchema>;

export const statsSchema = z
  .strictObject({
    id: z.string(),
    installs: z.number().int(),
    installedBase: z.number().int(),
    installsByDay: z.array(z.strictObject({ day: z.iso.date(), count: z.number().int() }).readonly()).readonly(),
    agentMix: counts
  })
  .readonly();
export type PackageStats = z.infer<typeof statsSchema>;

const personSchema = z.strictObject({ account: z.string(), displayName: z.string() }).readonly();
export type Person = z.infer<typeof personSchema>;
const teamNameSchema = z.strictObject({ namespace: z.string(), displayName: z.string() }).readonly();
export type TeamName = z.infer<typeof teamNameSchema>;

export const shareSchema = z
  .strictObject({
    target: z.string(),
    visibility: visibilitySchema,
    effective: effectiveSchema,
    users: z.array(personSchema).readonly(),
    teams: z.array(teamNameSchema).readonly(),
    groups: stringList,
    link: z.string().nullable()
  })
  .readonly();
export type Share = z.infer<typeof shareSchema>;

/** What a share saves: the lists replace the stored ones. */
export interface ShareUpdate {
  readonly visibility: Visibility;
  readonly users: readonly string[];
  readonly teams: readonly string[];
  readonly groups: readonly string[];
}

const linkSchema = z.strictObject({ link: z.string() }).readonly();

export const teamRoleSchema = z.enum(["owner", "member", "admin"]);
export const teamSummarySchema = z.strictObject({ namespace: z.string(), displayName: z.string(), visibility: effectiveSchema, role: teamRoleSchema, memberCount: z.number().int() }).readonly();
export type TeamSummary = z.infer<typeof teamSummarySchema>;

export const memberSchema = z.strictObject({ account: z.string(), displayName: z.string(), owner: z.boolean(), joinedAt: timestamp }).readonly();
export type Member = z.infer<typeof memberSchema>;

export const teamSchema = z
  .strictObject({ namespace: z.string(), displayName: z.string(), visibility: effectiveSchema, role: teamRoleSchema, members: z.array(memberSchema).readonly(), invite: z.string().nullable() })
  .readonly();
export type Team = z.infer<typeof teamSchema>;

export const directorySchema = z.strictObject({ people: z.array(personSchema).readonly(), teams: z.array(teamNameSchema).readonly() }).readonly();
export type Directory = z.infer<typeof directorySchema>;

export const linkPreviewSchema = z
  .strictObject({ kind: z.enum(["invite", "share"]), targetKind: z.enum(["team", "space", "package", "bundle"]), target: z.string(), name: z.string(), by: z.string() })
  .readonly();
export type LinkPreview = z.infer<typeof linkPreviewSchema>;
export const linkResultSchema = z
  .strictObject({ kind: z.enum(["invite", "share"]), targetKind: z.enum(["team", "space", "package", "bundle"]), target: z.string(), name: z.string(), by: z.string(), changed: z.boolean() })
  .readonly();
export type LinkResult = z.infer<typeof linkResultSchema>;

/** A package with an MCP server that everyone could see, waiting for an admin first. */
export const publicReviewSchema = z
  .strictObject({
    id: z.string(),
    namespace: z.string(),
    packageId: z.string(),
    name: z.string(),
    version: z.string(),
    publishedBy: z.string(),
    publishedAt: timestamp,
    changelog: z.string().nullable(),
    componentKinds: stringList,
    mcpServers: z.array(mcpServerSchema).readonly(),
    /** Who can see it now, in words: "Everyone", or "Private: 3 people, 1 team". */
    audience: z.string(),
    installedBase: z.number().int()
  })
  .readonly();
export type PublicReview = z.infer<typeof publicReviewSchema>;
const reviewListSchema = z.array(publicReviewSchema).readonly();

export const reportKindSchema = z.enum(["problem", "feedback"]);
export type ReportKind = z.infer<typeof reportKindSchema>;

/** A problem or a piece of feedback about a package; owners and admins answer it with an optional note. */
export const reportSchema = z
  .strictObject({
    id: z.number().int(),
    account: z.string(),
    packageId: z.string(),
    kind: reportKindSchema,
    reason: z.string(),
    createdAt: timestamp,
    resolvedAt: timestamp.nullable(),
    resolvedBy: z.string().nullable(),
    note: z.string().nullable()
  })
  .readonly();
export type Report = z.infer<typeof reportSchema>;
const reportListSchema = z.array(reportSchema).readonly();

export const summarySchema = z
  .strictObject({
    activeUsers: z.strictObject({ day: z.number().int(), week: z.number().int(), month: z.number().int() }).readonly(),
    publishers: z.number().int(),
    packages: z.number().int(),
    clientVersions: counts,
    agentMix: counts,
    topPackages: z.array(z.strictObject({ id: z.string(), installedBase: z.number().int() }).readonly()).readonly(),
    preflightFailures: counts,
    openReports: z.number().int()
  })
  .readonly();
export type Summary = z.infer<typeof summarySchema>;

export const notificationSchema = z.strictObject({ id: z.number().int(), kind: z.string(), text: z.string(), link: z.string().nullable(), createdAt: timestamp, read: z.boolean() }).readonly();
export type Notification = z.infer<typeof notificationSchema>;
const notificationsSchema = z.strictObject({ items: z.array(notificationSchema).readonly() }).readonly();

/** Who has a package installed, per PC, from the desktop apps' last check-ins. */
export const installRowSchema = z
  .strictObject({ account: z.string(), device: z.string().nullable(), version: z.string().nullable(), clientVersion: z.string().nullable(), lastSeenAt: timestamp })
  .readonly();
export type InstallRow = z.infer<typeof installRowSchema>;

export const auditEntrySchema = z.strictObject({ id: z.number().int(), at: timestamp, actor: z.string(), action: z.string(), target: z.string(), detail: z.string().nullable() }).readonly();
export type AuditEntry = z.infer<typeof auditEntrySchema>;

export const blockSchema = z.strictObject({ account: z.string(), blockedBy: z.string(), blockedAt: timestamp, reason: z.string().nullable() }).readonly();
export type Block = z.infer<typeof blockSchema>;

/** RFC 9457 lets a problem carry extension members, so only the fields the portal reads are checked. */
const problemSchema = z
  .object({
    title: z.string(),
    detail: z.string().optional(),
    errors: z
      .array(z.strictObject({ path: z.string(), message: z.string() }).readonly())
      .readonly()
      .optional()
  })
  .readonly();

/** A failed request, in words a publisher can act on. */
export class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string,
    public readonly details: readonly { readonly path: string; readonly message: string }[] = []
  ) {
    super(message);
    this.name = "ApiError";
  }

  public static from(error: unknown): ApiError {
    if (error instanceof ApiError) {
      return error;
    }

    if (error instanceof HttpErrorResponse) {
      const problem = problemSchema.safeParse(error.error);
      if (problem.success) {
        const { title, detail } = problem.data;
        const message = detail === undefined || title.includes(detail) ? title : `${title} ${detail}`;
        return new ApiError(error.status, message, problem.data.errors ?? []);
      }

      return new ApiError(error.status, error.status === 0 ? "The marketplace could not be reached." : `The marketplace answered ${String(error.status)}.`);
    }

    return new ApiError(0, error instanceof Error ? error.message : "Something went wrong.");
  }
}

/** One path segment, encoded, so namespaces, IDs, versions, and file paths never change the route. */
function segment(value: string): string {
  return encodeURIComponent(value);
}

function filePath(path: string): string {
  return path.split("/").map(segment).join("/");
}

export function packageUrl(ns: string, packageId: string): string {
  return `/api/packages/${segment(ns)}/${segment(packageId)}`;
}

/** The version's zip; the server names the download. */
export function archiveUrl(ns: string, packageId: string, version: string): string {
  return `${packageUrl(ns, packageId)}/versions/${segment(version)}/archive`;
}

/** Where a version's files are listed; each file is below it. */
export function versionFiles(ns: string, packageId: string, version: string): string {
  return `${packageUrl(ns, packageId)}/versions/${segment(version)}/files`;
}

/** Where a suggestion's files are listed, each marked against the live version. */
export function suggestionFiles(id: number): string {
  return `/api/suggestions/${String(id)}/files`;
}

export function fileUrl(files: string, path: string): string {
  return `${files}/${filePath(path)}`;
}

export function installsUrl(ns: string, packageId: string): string {
  return `/api/admin/packages/${segment(ns)}/${segment(packageId)}/installs`;
}

export const auditCsvUrl = "/api/admin/audit?format=csv&limit=10000";

/** What a publish sends: an archive, or files with their paths, plus the version fields. */
export interface PublishForm {
  readonly version: string;
  readonly changelog: string;
  readonly tags: readonly string[];
  readonly files: readonly { readonly path: string; readonly file: Blob }[];
  readonly archive?: Blob;
  readonly name?: string;
  readonly description?: string;
  /** Who can install it; left out, a new package follows its space and an existing one keeps its setting. */
  readonly visibility?: Visibility;
}

/** The upload part of a publish or a suggestion. */
function uploadBody(form: Pick<PublishForm, "files" | "archive">): FormData {
  const body = new FormData();
  if (form.archive !== undefined) {
    body.set("archive", form.archive, "upload.zip");
  }

  for (const { path, file } of form.files) {
    body.append("files", file, path.split("/").at(-1) ?? path);
    body.append("paths", path);
  }

  return body;
}

function publishBody(form: PublishForm): FormData {
  const body = uploadBody(form);
  body.set("version", form.version);
  body.set("changelog", form.changelog);
  body.set("tags", form.tags.join(","));
  for (const [key, value] of [
    ["name", form.name],
    ["description", form.description],
    ["visibility", form.visibility]
  ] as const) {
    if (value !== undefined && value.length > 0) {
      body.set(key, value);
    }
  }

  return body;
}

/** `ns`, or `ns/id` for a package or bundle, as the access routes spell it. */
function target(ns: string, id?: string): string {
  return id === undefined ? segment(ns) : `${segment(ns)}/${segment(id)}`;
}

@Injectable({ providedIn: "root" })
export class Api {
  private readonly http = inject(HttpClient);
  private indexCache: Promise<Index> | null = null;

  public health(): Promise<Health> {
    return this.get("/api/health", healthSchema);
  }

  public me(): Promise<Me> {
    return this.get("/api/me", meSchema);
  }

  /** One copy per page load, shared by every page; a change made from this portal drops it. */
  public index(): Promise<Index> {
    this.indexCache ??= this.get("/api/index", indexSchema).catch((error: unknown) => {
      this.indexCache = null;
      throw error;
    });
    return this.indexCache;
  }

  public forgetIndex(): void {
    this.indexCache = null;
  }

  public package(ns: string, packageId: string): Promise<PackageDetail> {
    return this.get(packageUrl(ns, packageId), packageSchema);
  }

  public mine(): Promise<Mine> {
    return this.get("/api/mine", mineSchema);
  }

  /** `files` is `versionFiles(…)` or `suggestionFiles(…)`. */
  public files(files: string): Promise<readonly PackageFile[]> {
    return this.get(files, fileListSchema);
  }

  /** A text file; binary files and files over 1 MB come back as downloads, not previews. */
  public async fileText(files: string, path: string): Promise<string> {
    const response = await this.request(() => firstValueFrom(this.http.get(fileUrl(files, path), { responseType: "text", observe: "response" })));
    if (response.headers.get("Content-Type")?.startsWith("text/") !== true || response.body === null) {
      throw new ApiError(415, `${path} is not a text file.`);
    }

    return response.body;
  }

  public stats(ns: string, packageId: string): Promise<PackageStats> {
    return this.get(`/api/stats/packages/${segment(ns)}/${segment(packageId)}`, statsSchema);
  }

  public share(ns: string, id?: string): Promise<Share> {
    return this.get(`/api/access/${target(ns, id)}`, shareSchema);
  }

  public async setShare(ns: string, id: string | undefined, update: ShareUpdate): Promise<Share> {
    const saved = await this.send(() => firstValueFrom(this.http.put<unknown>(`/api/access/${target(ns, id)}`, update)), shareSchema);
    this.forgetIndex();
    return saved;
  }

  /** The share link, created on first use; `reset` replaces it so the old one stops working. */
  public async shareLink(ns: string, id: string | undefined, reset: boolean): Promise<string> {
    return (await this.send(() => firstValueFrom(this.http.post<unknown>(`/api/access/${target(ns, id)}/link`, { reset })), linkSchema)).link;
  }

  public async publish(ns: string, packageId: string, form: PublishForm): Promise<Published> {
    const published = await this.send(() => firstValueFrom(this.http.post<unknown>(`${packageUrl(ns, packageId)}/versions`, publishBody(form))), publishedSchema);
    this.forgetIndex();
    return published;
  }

  /** Runs every check a publish would, and compares the files with the live version, without publishing. */
  public checkPublish(ns: string, packageId: string, form: PublishForm): Promise<DryRun> {
    const body = publishBody(form);
    body.set("dryRun", "true");
    return this.send(() => firstValueFrom(this.http.post<unknown>(`${packageUrl(ns, packageId)}/versions`, body)), dryRunSchema);
  }

  /** Deletes a package nobody installed (admins: any package); its ID can be used again. */
  public async deletePackage(ns: string, packageId: string): Promise<void> {
    await this.request(async () => {
      await firstValueFrom(this.http.delete(packageUrl(ns, packageId)));
    });
    this.forgetIndex();
  }

  public withdraw(ns: string, packageId: string, version: string): Promise<void> {
    return this.change(() => this.http.put(this.yankUrl(ns, packageId, version), null));
  }

  /** Undoes a withdrawal. */
  public restore(ns: string, packageId: string, version: string): Promise<void> {
    return this.change(() => this.http.delete(this.yankUrl(ns, packageId, version)));
  }

  /** Removes the package from every PC at their next check; `revoked: false` offers it again. */
  public setRevoked(ns: string, packageId: string, revoked: boolean): Promise<void> {
    const url = `${packageUrl(ns, packageId)}/revoke`;
    return this.change(() => (revoked ? this.http.put(url, null) : this.http.delete(url)));
  }

  /** Admins only: deletes a version's files for good; its number stays used. */
  public purgeVersion(ns: string, packageId: string, version: string): Promise<void> {
    return this.change(() => this.http.delete(`/api/admin/packages/${segment(ns)}/${segment(packageId)}/versions/${segment(version)}`));
  }

  /** A problem reaches the owners and the admins; feedback reaches the owners. */
  public report(ns: string, packageId: string, reason: string, kind: ReportKind): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.post(`${packageUrl(ns, packageId)}/reports`, { reason, kind }));
    });
  }

  /** Reports and feedback on a package, for its owners and admins. */
  public packageReports(ns: string, packageId: string): Promise<readonly Report[]> {
    return this.get(`${packageUrl(ns, packageId)}/reports`, reportListSchema);
  }

  /** What the caller reported, and what became of it. */
  public myReports(): Promise<readonly Report[]> {
    return this.get("/api/reports/mine", reportListSchema);
  }

  public resolveReport(id: number, note: string): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.post(`/api/reports/${String(id)}/resolve`, note.length > 0 ? { note } : {}));
    });
  }

  /** Newest first, one page; `before` pages back from an id already shown. */
  public async notifications(before?: number): Promise<readonly Notification[]> {
    return (await this.get(before === undefined ? "/api/notifications" : `/api/notifications?before=${String(before)}`, notificationsSchema)).items;
  }

  /** Marks the notifications with ids from `from` to `upTo` read, the ones a page showed. */
  public markNotificationsRead(from: number, upTo: number): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.post("/api/notifications/read", { from, upTo }));
    });
  }

  public suggest(ns: string, packageId: string, message: string, form: Pick<PublishForm, "files" | "archive">): Promise<Suggestion> {
    const body = uploadBody(form);
    body.set("message", message);
    return this.send(() => firstValueFrom(this.http.post<unknown>(`${packageUrl(ns, packageId)}/suggestions`, body)), suggestionSchema);
  }

  /** Owners get every suggestion for the package; anyone else gets their own. */
  public suggestions(ns: string, packageId: string): Promise<readonly Suggestion[]> {
    return this.get(`${packageUrl(ns, packageId)}/suggestions`, suggestionListSchema);
  }

  public suggestion(id: number): Promise<Suggestion> {
    return this.get(`/api/suggestions/${String(id)}`, suggestionSchema);
  }

  public async decideSuggestion(id: number, decision: { readonly decision: "accept"; readonly version: string } | { readonly decision: "decline"; readonly note: string }): Promise<Suggestion> {
    const suggestion = await this.send(() => firstValueFrom(this.http.post<unknown>(`/api/suggestions/${String(id)}`, decision)), suggestionSchema);
    this.forgetIndex();
    return suggestion;
  }

  public withdrawSuggestion(id: number): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.delete(`/api/suggestions/${String(id)}`));
    });
  }

  public teams(): Promise<readonly TeamSummary[]> {
    return this.get("/api/teams", z.array(teamSummarySchema).readonly());
  }

  public team(ns: string): Promise<Team> {
    return this.get(this.teamUrl(ns), teamSchema);
  }

  public async createTeam(namespace: string, displayName: string, visibility: "public" | "private"): Promise<Team> {
    const result = await this.send(() => firstValueFrom(this.http.post<unknown>("/api/teams", { namespace, displayName, visibility })), teamSchema);
    this.forgetIndex();
    return result;
  }

  public async renameTeam(ns: string, displayName: string): Promise<Team> {
    const result = await this.send(() => firstValueFrom(this.http.put<unknown>(this.teamUrl(ns), { displayName })), teamSchema);
    this.forgetIndex();
    return result;
  }

  /** Adds a member, or changes whether they are an owner. */
  public async setMember(ns: string, account: string, owner: boolean): Promise<Team> {
    const result = await this.send(() => firstValueFrom(this.http.post<unknown>(`${this.teamUrl(ns)}/members`, { account, owner })), teamSchema);
    this.forgetIndex();
    return result;
  }

  /** Removes a member; with your own account, leaves the team. */
  public removeMember(ns: string, account: string): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.delete(`${this.teamUrl(ns)}/members`, { params: { account } }));
      this.forgetIndex();
    });
  }

  public async teamInvite(ns: string, reset: boolean): Promise<string> {
    return (await this.send(() => firstValueFrom(this.http.post<unknown>(`${this.teamUrl(ns)}/invite`, { reset })), z.strictObject({ invite: z.string() }))).invite;
  }

  public deleteTeam(ns: string): Promise<void> {
    return this.change(() => this.http.delete(this.teamUrl(ns)));
  }

  public directory(query: string): Promise<Directory> {
    return this.send(() => firstValueFrom(this.http.get<unknown>("/api/directory", { params: { q: query } })), directorySchema);
  }

  public linkPreview(code: string): Promise<LinkPreview> {
    return this.get(`/api/links/${segment(code)}`, linkPreviewSchema);
  }

  public async redeemLink(code: string): Promise<LinkResult> {
    const result = await this.send(() => firstValueFrom(this.http.post<unknown>(`/api/links/${segment(code)}`, null)), linkResultSchema);
    this.forgetIndex();
    return result;
  }

  public bundle(ns: string, bundleId: string): Promise<Bundle> {
    return this.get(this.bundleUrl(ns, bundleId), bundleSchema);
  }

  public async saveBundle(ns: string, bundleId: string, bundle: { readonly name: string; readonly description: string; readonly members: readonly string[] }): Promise<Bundle> {
    const saved = await this.send(() => firstValueFrom(this.http.put<unknown>(this.bundleUrl(ns, bundleId), bundle)), bundleSchema);
    this.forgetIndex();
    return saved;
  }

  public deleteBundle(ns: string, bundleId: string): Promise<void> {
    return this.change(() => this.http.delete(this.bundleUrl(ns, bundleId)));
  }

  public reviews(): Promise<readonly PublicReview[]> {
    return this.get("/api/admin/reviews", reviewListSchema);
  }

  /** `version` is the one the admin looked at; an approval of anything else answers 409. */
  public review(ns: string, packageId: string, decision: "approve" | "decline", note: string, version: string): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.post(`/api/admin/reviews/${segment(ns)}/${segment(packageId)}`, { decision, note, version }));
      this.forgetIndex();
    });
  }

  public reports(): Promise<readonly Report[]> {
    return this.get("/api/admin/reports", reportListSchema);
  }

  public summary(): Promise<Summary> {
    return this.get("/api/admin/summary", summarySchema);
  }

  /** Who has a package installed; `installsCsv` is the same list as a download. */
  public installs(ns: string, packageId: string): Promise<readonly InstallRow[]> {
    return this.get(installsUrl(ns, packageId), z.array(installRowSchema).readonly());
  }

  /** Newest first; `before` pages back from an entry's id. */
  public audit(before?: number): Promise<readonly AuditEntry[]> {
    return this.send(
      () => firstValueFrom(this.http.get<unknown>("/api/admin/audit", { params: before === undefined ? { limit: 100 } : { before, limit: 100 } })),
      z.array(auditEntrySchema).readonly()
    );
  }

  public blocks(): Promise<readonly Block[]> {
    return this.get("/api/admin/blocks", z.array(blockSchema).readonly());
  }

  /** Stops an account publishing, suggesting, and sharing, and hides its space; `reason: null` lifts it. */
  public setBlocked(account: string, reason: string | null): Promise<void> {
    const url = `/api/admin/blocks/${segment(account)}`;
    return this.change(() => (reason === null ? this.http.delete(url) : this.http.put(url, reason.length > 0 ? { reason } : {})));
  }

  /** Hands a personal space to another account. */
  public transferSpace(ns: string, account: string): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.put(`/api/admin/namespaces/${segment(ns)}/owner`, { account }));
      this.forgetIndex();
    });
  }

  private yankUrl(ns: string, packageId: string, version: string): string {
    return `${packageUrl(ns, packageId)}/versions/${segment(version)}/yank`;
  }

  private teamUrl(ns: string): string {
    return `/api/teams/${segment(ns)}`;
  }

  private bundleUrl(ns: string, bundleId: string): string {
    return `/api/bundles/${segment(ns)}/${segment(bundleId)}`;
  }

  private get<T>(url: string, schema: z.ZodType<T>): Promise<T> {
    return this.send(() => firstValueFrom(this.http.get<unknown>(url)), schema);
  }

  private async send<T>(call: () => Promise<unknown>, schema: z.ZodType<T>): Promise<T> {
    const payload = await this.request(call);
    return schema.parse(payload);
  }

  /** A change that can alter what the index lists. */
  private async change(call: () => Observable<unknown>): Promise<void> {
    await this.request(() => firstValueFrom(call()));
    this.forgetIndex();
  }

  private async request<T>(call: () => Promise<T>): Promise<T> {
    try {
      return await call();
    } catch (error) {
      throw ApiError.from(error);
    }
  }
}
