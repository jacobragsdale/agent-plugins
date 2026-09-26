import { invoke } from "@tauri-apps/api/core";
import { z } from "zod";
import { ipcErrorSchema } from "./schemas";
import type { IpcErrorKind } from "./schemas";

export const SCHEDULED_SYNC_EVENT = "scheduled-sync";
/** Carries an `agent-plugins://` link the running app was opened with. */
export const DEEP_LINK_EVENT = "deep-link";

/** The commands `lib.rs` registers with `generate_handler!`; keep the two lists the same. */
export type IpcCommand =
  | "load_cached_manifest_state"
  | "run_preflight"
  | "sync_manifest_state"
  | "prepare_source"
  | "confirm_source"
  | "cancel_prepared_source"
  | "install_item"
  | "replace_item"
  | "set_manual_invocation"
  | "uninstall_item"
  | "plan_bulk_items"
  | "run_bulk_items"
  | "plan_source_removal"
  | "remove_manifest_source"
  | "reset_app"
  | "run_tutorial"
  | "dismiss_tutorial"
  | "create_skill"
  | "take_pending_link"
  | "list_teams"
  | "get_team"
  | "create_team"
  | "rename_team"
  | "add_team_member"
  | "remove_team_member"
  | "team_invite"
  | "delete_team"
  | "search_directory"
  | "preview_link"
  | "redeem_link"
  | "get_share"
  | "set_share"
  | "share_link"
  | "save_bundle"
  | "delete_bundle"
  | "plan_items"
  | "run_items";

export async function invokeParsed<T>(command: IpcCommand, schema: z.ZodType<T>, args?: Record<string, unknown>): Promise<T> {
  const payload = args === undefined ? await invoke<unknown>(command) : await invoke<unknown>(command, args);
  return schema.parse(payload);
}

/** What the window shows: a plain-language summary, with the raw text behind "Details". `kind` is null for an untyped (string) error. */
export type AppError = Readonly<{ kind: IpcErrorKind | null; summary: string; detail: string | null }>;

const SOMETHING_WENT_WRONG = "Something went wrong.";

/**
 * Turns a rejection into something a person can read. `context` names what
 * failed in plain words ("Couldn't install Python standards."); the raw
 * backend text then goes to `detail` instead of the summary.
 */
export function toAppError(reason: unknown, context?: string): AppError {
  const typed = ipcErrorSchema.safeParse(reason);
  if (typed.success) {
    const { kind, message, detail } = typed.data;
    if (kind === "bug") {
      return { kind, summary: joined(context, SOMETHING_WENT_WRONG), detail: detail === undefined ? message : `${message}\n${detail}` };
    }
    return { kind, summary: joined(context, message), detail: detail ?? null };
  }
  if (reason instanceof z.ZodError) {
    return { kind: "bug", summary: joined(context, SOMETHING_WENT_WRONG), detail: `Agent Plugins received data it could not read.\n${z.prettifyError(reason)}` };
  }
  const raw = reason instanceof Error ? reason.message : String(reason);
  return context === undefined ? { kind: null, summary: raw, detail: null } : { kind: null, summary: context, detail: raw };
}

function joined(context: string | undefined, message: string): string {
  return context === undefined ? message : `${context} ${message}`;
}

/**
 * What the window does with a failure: `offline` feeds the offline banner
 * instead of a red error, `retry` tries a person's action once more on its
 * own, and `show` puts the message on screen.
 */
export type ErrorResponse = "offline" | "retry" | "show";

export function errorResponse(kind: IpcErrorKind | null): ErrorResponse {
  switch (kind) {
    case "offline":
      return "offline";
    case "locked":
    case "retryable":
      return "retry";
    case "needsUser":
    case "bug":
    case null:
      return "show";
  }
}

export const RETRY_DELAY_MILLIS = 3000;

/** Runs a person's action, and once more after a short wait when the failure is one that usually clears up (a locked file, a dropped connection). */
export async function withRetry<T>(task: () => Promise<T>, onRetrying: () => void, delayMillis: number = RETRY_DELAY_MILLIS): Promise<T> {
  try {
    return await task();
  } catch (reason) {
    if (errorResponse(toAppError(reason).kind) !== "retry") {
      throw reason;
    }
    onRetrying();
    await new Promise<void>((resolve) => {
      setTimeout(resolve, delayMillis);
    });
    return task();
  }
}

/** After the retry also failed: the message plus what the person can do, such as closing the app that holds the file. */
export function explainAfterRetry(error: AppError, appNames: readonly string[]): AppError {
  if (errorResponse(error.kind) !== "retry") {
    return error;
  }
  const app = appNames.find((name) => error.summary.includes(name) || (error.detail?.includes(name) ?? false));
  const advice = app === undefined ? (error.kind === "locked" ? "Another program is using these files. Close it, then try again." : "Try again in a moment.") : `Close ${app} and try again.`;
  return { ...error, summary: `${error.summary} ${advice}` };
}
