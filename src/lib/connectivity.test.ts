import { describe, expect, it } from "vitest";
import fixture from "../ipc/fixtures/app-state.json";
import { appStateSchema } from "../ipc/schemas";
import type { AppState } from "../ipc/schemas";
import { catalogBody, formatEpoch, lastCheckedLabel, NOTHING_PUBLISHED, OFFLINE_EMPTY, offlineBanner, savedCopyLabel } from "./connectivity";

const base = appStateSchema.parse(fixture);
const online: AppState = {
  ...base,
  connectivity: "online",
  syncInProgress: false,
  checkedAtEpochSeconds: 1790000000,
  repositories: base.repositories.map((repository) => ({ ...repository, status: "fresh" })),
  sources: base.sources.map((source) => ({ ...source, status: "fresh" }))
};
const offline: AppState = { ...online, connectivity: "offline" };
const empty: AppState = { ...online, sources: [], items: [], repositories: [] };

describe("offlineBanner", () => {
  it("stays hidden while every server answers", () => {
    expect(offlineBanner(online, false)).toBeNull();
  });

  it("shows when the packages on screen are as of the last good check", () => {
    expect(offlineBanner(offline, false)).toBe(`Offline — showing packages as of ${formatEpoch(1790000000)}. Agent Plugins will retry automatically.`);
  });

  it("shows when an action just failed for lack of a connection", () => {
    expect(offlineBanner(online, true)).toContain("Offline — showing packages as of");
  });

  it("names the sources a degraded check couldn't reach", () => {
    const [first, ...rest] = online.sources;
    if (first === undefined) {
      throw new Error("The fixture needs a source.");
    }
    const degraded: AppState = { ...online, connectivity: "degraded", sources: [{ ...first, status: "stale" }, ...rest] };
    expect(offlineBanner(degraded, false)).toBe(`Couldn't reach ${first.name}, so their saved copy is shown. Agent Plugins will retry automatically.`);
    expect(offlineBanner({ ...degraded, sources: online.sources }, false)).toContain("Couldn't reach some sources");
  });

  it("leaves an empty page to say it is offline itself", () => {
    expect(offlineBanner({ ...empty, connectivity: "offline" }, false)).toBeNull();
    expect(offlineBanner(null, true)).toBeNull();
  });
});

describe("catalogBody", () => {
  it("lists packages whenever there are any, or a filter is on", () => {
    expect(catalogBody(offline, false, false)).toEqual({ kind: "list" });
    expect(catalogBody(empty, false, true)).toEqual({ kind: "list" });
  });

  it("explains an empty first launch without a connection", () => {
    expect(catalogBody(null, true, false)).toEqual({ kind: "empty", text: OFFLINE_EMPTY, offline: true });
    expect(catalogBody({ ...empty, connectivity: "offline" }, false, false)).toEqual({ kind: "empty", text: OFFLINE_EMPTY, offline: true });
  });

  it("says nothing is published only once a check finished online", () => {
    expect(catalogBody(empty, false, false)).toEqual({ kind: "empty", text: NOTHING_PUBLISHED, offline: false });
    expect(catalogBody({ ...empty, syncInProgress: true }, false, false)).toEqual({ kind: "loading" });
    expect(catalogBody(null, false, false)).toEqual({ kind: "loading" });
  });
});

describe("time labels", () => {
  it("reads a never-checked state as not yet", () => {
    expect(lastCheckedLabel({ ...online, checkedAtEpochSeconds: 0 })).toBe("Not yet");
    expect(lastCheckedLabel(null)).toBe("Not yet");
    expect(lastCheckedLabel(online)).toBe(formatEpoch(1790000000));
  });

  it("dates a saved copy when it knows when it loaded", () => {
    expect(savedCopyLabel(null)).toBe("Saved copy");
    expect(savedCopyLabel(1790000000)).toBe(`Saved copy from ${formatEpoch(1790000000)}`);
  });
});
