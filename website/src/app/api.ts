import { HttpClient, HttpErrorResponse } from "@angular/common/http";
import { Injectable, inject } from "@angular/core";
import { firstValueFrom } from "rxjs";
import { z } from "zod";

// Every response is parsed from `unknown` here; the portal and the API ship in one image, so an
// unexpected field is a bug worth failing on (strict objects).

const timestamp = z.iso.datetime({ offset: true });
const stringList = z.array(z.string()).readonly();
const counts = z.record(z.string(), z.number().int()).readonly();

export const laneSchema = z.enum(["official", "team", "personal"]);
export type Lane = z.infer<typeof laneSchema>;

export const reviewStateSchema = z.enum(["pending", "approved", "rejected"]);
export type ReviewState = z.infer<typeof reviewStateSchema>;

export const healthSchema = z
  .strictObject({ serverVersion: z.string(), minimumClientVersion: z.string(), latestClientVersion: z.string(), environment: z.string(), authSchemes: stringList })
  .readonly();
export type Health = z.infer<typeof healthSchema>;

export const meSchema = z.strictObject({ account: z.string(), namespace: z.string(), displayName: z.string(), namespaces: stringList, admin: z.boolean(), groups: stringList }).readonly();
export type Me = z.infer<typeof meSchema>;

export const indexPackageSchema = z
  .strictObject({
    id: z.string(),
    namespace: z.string(),
    packageId: z.string(),
    name: z.string(),
    description: z.string(),
    version: z.string(),
    publisher: z.strictObject({ account: z.string(), displayName: z.string() }).readonly(),
    lane: laneSchema,
    tags: stringList,
    componentKinds: stringList,
    publishedAt: timestamp,
    installs: z.number().int(),
    installedBase: z.number().int(),
    restricted: z.boolean()
  })
  .readonly();
export type IndexPackage = z.infer<typeof indexPackageSchema>;

export const indexSchema = z.strictObject({ generatedAt: timestamp, packages: z.array(indexPackageSchema).readonly() }).readonly();

export const versionSchema = z
  .strictObject({
    version: z.string(),
    archiveDigest: z.string(),
    sizeBytes: z.number().int(),
    publishedBy: z.string(),
    publishedAt: timestamp,
    changelog: z.string().nullable(),
    yanked: z.boolean(),
    componentKinds: stringList,
    reviewState: reviewStateSchema,
    reviewNote: z.string().nullable()
  })
  .readonly();
export type PackageVersion = z.infer<typeof versionSchema>;

export const packageSchema = z
  .strictObject({
    id: z.string(),
    namespace: z.string(),
    packageId: z.string(),
    name: z.string(),
    description: z.string(),
    tags: stringList,
    liveVersion: z.string().nullable(),
    versions: z.array(versionSchema).readonly()
  })
  .readonly();
export type PackageDetail = z.infer<typeof packageSchema>;

export const spaceSchema = z.strictObject({ namespace: z.string(), displayName: z.string(), lane: laneSchema }).readonly();
export type Space = z.infer<typeof spaceSchema>;

export const mineSchema = z.strictObject({ spaces: z.array(spaceSchema).readonly(), packages: z.array(packageSchema).readonly() }).readonly();
export type Mine = z.infer<typeof mineSchema>;

export const packageFileSchema = z.strictObject({ path: z.string(), size: z.number().int() }).readonly();
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
    reviewState: reviewStateSchema
  })
  .readonly();
export type Published = z.infer<typeof publishedSchema>;

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

export const accessSchema = z.strictObject({ target: z.string(), users: stringList, groups: stringList }).readonly();
export type Access = z.infer<typeof accessSchema>;

export const pendingReviewSchema = z
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
    firstVersion: z.boolean(),
    liveVersion: z.string().nullable()
  })
  .readonly();
export type PendingReview = z.infer<typeof pendingReviewSchema>;
const reviewListSchema = z.array(pendingReviewSchema).readonly();

export const reportSchema = z
  .strictObject({ id: z.number().int(), account: z.string(), packageId: z.string(), reason: z.string(), createdAt: timestamp, resolvedAt: timestamp.nullable(), resolvedBy: z.string().nullable() })
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

export function fileUrl(ns: string, packageId: string, version: string, path: string): string {
  return `${packageUrl(ns, packageId)}/versions/${segment(version)}/files/${filePath(path)}`;
}

/** What a publish sends: an archive, or files with their paths, plus the version fields. */
export interface PublishForm {
  readonly version: string;
  readonly changelog: string;
  readonly tags: readonly string[];
  readonly files: readonly { readonly path: string; readonly file: Blob }[];
  readonly archive?: Blob;
  readonly name?: string;
  readonly description?: string;
}

@Injectable({ providedIn: "root" })
export class Api {
  private readonly http = inject(HttpClient);

  public health(): Promise<Health> {
    return this.get("/api/health", healthSchema);
  }

  public me(): Promise<Me> {
    return this.get("/api/me", meSchema);
  }

  public async index(): Promise<readonly IndexPackage[]> {
    return (await this.get("/api/index", indexSchema)).packages;
  }

  public package(ns: string, packageId: string): Promise<PackageDetail> {
    return this.get(packageUrl(ns, packageId), packageSchema);
  }

  public mine(): Promise<Mine> {
    return this.get("/api/mine", mineSchema);
  }

  public files(ns: string, packageId: string, version: string): Promise<readonly PackageFile[]> {
    return this.get(`${packageUrl(ns, packageId)}/versions/${segment(version)}/files`, fileListSchema);
  }

  /** A text file from a version; binary files and files over 1 MB come back as downloads, not previews. */
  public async fileText(ns: string, packageId: string, version: string, path: string): Promise<string> {
    const response = await this.request(() => firstValueFrom(this.http.get(fileUrl(ns, packageId, version, path), { responseType: "text", observe: "response" })));
    if (response.headers.get("Content-Type")?.startsWith("text/") !== true || response.body === null) {
      throw new ApiError(415, `${path} is not a text file.`);
    }

    return response.body;
  }

  public stats(ns: string, packageId: string): Promise<PackageStats> {
    return this.get(`/api/stats/packages/${segment(ns)}/${segment(packageId)}`, statsSchema);
  }

  public access(ns: string, packageId?: string): Promise<Access> {
    return this.get(this.accessUrl(ns, packageId), accessSchema);
  }

  public setAccess(ns: string, packageId: string | undefined, users: readonly string[], groups: readonly string[]): Promise<Access> {
    return this.send(() => firstValueFrom(this.http.put<unknown>(this.accessUrl(ns, packageId), { users, groups })), accessSchema);
  }

  public publish(ns: string, packageId: string, form: PublishForm): Promise<Published> {
    const body = new FormData();
    body.set("version", form.version);
    body.set("changelog", form.changelog);
    body.set("tags", form.tags.join(","));
    for (const [key, value] of [
      ["name", form.name],
      ["description", form.description]
    ] as const) {
      if (value !== undefined && value.length > 0) {
        body.set(key, value);
      }
    }

    if (form.archive !== undefined) {
      body.set("archive", form.archive, "upload.zip");
    }

    for (const { path, file } of form.files) {
      body.append("files", file, path.split("/").at(-1) ?? path);
      body.append("paths", path);
    }

    return this.send(() => firstValueFrom(this.http.post<unknown>(`${packageUrl(ns, packageId)}/versions`, body)), publishedSchema);
  }

  public withdraw(ns: string, packageId: string, version: string): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.put(this.yankUrl(ns, packageId, version), null));
    });
  }

  /** Undoes a withdrawal. */
  public restore(ns: string, packageId: string, version: string): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.delete(this.yankUrl(ns, packageId, version)));
    });
  }

  public report(ns: string, packageId: string, reason: string): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.post(`${packageUrl(ns, packageId)}/reports`, { reason }));
    });
  }

  public reviews(): Promise<readonly PendingReview[]> {
    return this.get("/api/admin/reviews", reviewListSchema);
  }

  public review(ns: string, packageId: string, version: string, decision: "approve" | "reject", note: string): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.post(`/api/admin/reviews/${segment(ns)}/${segment(packageId)}/${segment(version)}`, { decision, note }));
    });
  }

  public reports(): Promise<readonly Report[]> {
    return this.get("/api/admin/reports", reportListSchema);
  }

  public resolveReport(id: number): Promise<void> {
    return this.request(async () => {
      await firstValueFrom(this.http.post(`/api/admin/reports/${String(id)}/resolve`, null));
    });
  }

  public summary(): Promise<Summary> {
    return this.get("/api/admin/summary", summarySchema);
  }

  private yankUrl(ns: string, packageId: string, version: string): string {
    return `${packageUrl(ns, packageId)}/versions/${segment(version)}/yank`;
  }

  private accessUrl(ns: string, packageId: string | undefined): string {
    return packageId === undefined ? `/api/access/${segment(ns)}` : `/api/access/${segment(ns)}/${segment(packageId)}`;
  }

  private get<T>(url: string, schema: z.ZodType<T>): Promise<T> {
    return this.send(() => firstValueFrom(this.http.get<unknown>(url)), schema);
  }

  private async send<T>(call: () => Promise<unknown>, schema: z.ZodType<T>): Promise<T> {
    const payload = await this.request(call);
    return schema.parse(payload);
  }

  private async request<T>(call: () => Promise<T>): Promise<T> {
    try {
      return await call();
    } catch (error) {
      throw ApiError.from(error);
    }
  }
}
