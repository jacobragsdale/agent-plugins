/**
 * Starts work Angular does not await (a constructor, a template event) and reports a failure instead
 * of dropping it. Pages turn expected failures into messages themselves; this catches the rest.
 */
export function runTask(task: Promise<unknown>): void {
  task.catch((error: unknown) => {
    console.error(error);
  });
}
