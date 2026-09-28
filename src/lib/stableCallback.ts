import { useCallback, useLayoutEffect, useRef } from "react";

/**
 * A function whose identity never changes but that always runs the newest
 * render's `handler`. Handed to memoized cards, it lets a click, a keystroke,
 * or a notice re-render only what changed instead of every card in the
 * catalog. Call it from events only, never during render.
 */
export function useStableCallback<Args extends unknown[], Result>(handler: (...args: Args) => Result): (...args: Args) => Result {
  const latest = useRef(handler);
  useLayoutEffect(() => {
    latest.current = handler;
  });
  return useCallback((...args: Args) => latest.current(...args), []);
}
