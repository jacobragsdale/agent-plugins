import type { MatSnackBar } from "@angular/material/snack-bar";

/**
 * Starts work Angular does not await (a constructor, a template event) and reports a failure instead
 * of dropping it. Pages turn expected failures into messages themselves; this catches the rest.
 */
export function runTask(task: Promise<unknown>): void {
  task.catch((error: unknown) => {
    console.error(error);
  });
}

/** Copies text, saying so, or explains how to copy it by hand when the browser refuses. */
export async function copyText(snackBar: MatSnackBar, text: string, done = "Copied."): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    snackBar.open(done, undefined, { duration: 3000 });
  } catch {
    snackBar.open("The browser blocked the clipboard; select the text and copy it instead.", "Dismiss");
  }
}
