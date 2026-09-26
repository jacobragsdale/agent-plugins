import { describe, expect, it } from "vitest";
import fixture from "../ipc/fixtures/app-state.json";
import { appStateSchema } from "../ipc/schemas";
import type { AppState, PreflightCheck } from "../ipc/schemas";
import { catalogBody, formatEpoch, headerProblems, lastCheckedLabel, noMatchesText, NOTHING_PUBLISHED, OFFLINE_EMPTY, offlineBanner, savedCopyLabel } from "./connectivity";

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

  it("stays hidden while some servers answer; the unreachable sources carry their own badge", () => {
    const [first, ...rest] = online.sources;
    if (first === undefined) {
      throw new Error("The fixture needs a source.");
    }
    expect(offlineBanner({ ...online, connectivity: "degraded", sources: [{ ...first, status: "stale" }, ...rest] }, false)).toBeNull();
  });

  it("leaves an empty page to say it is offline itself", () => {
    expect(offlineBanner({ ...empty, connectivity: "offline" }, false)).toBeNull();
    expect(offlineBanner(null, true)).toBeNull();
  });
});

describe("headerProblems", () => {
  const check = (id: string): PreflightCheck => ({ id, group: id.split(".")[0] ?? "", title: id, status: "fail", detail: "", remediation: null, blocking: false, durationMillis: 0 });

  it("leaves being offline to the offline banner", () => {
    const problems = [check("server.health"), check("host.dns"), check("agents.ledger")];
    expect(headerProblems(problems, true).map((problem) => problem.id)).toEqual(["agents.ledger"]);
    expect(headerProblems(problems, false)).toHaveLength(3);
  });

  it("counts an untrusted certificate once", () => {
    expect(headerProblems([check("server.health"), check("host.tls")], false).map((problem) => problem.id)).toEqual(["host.tls"]);
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

describe("noMatchesText", () => {
  it("names the search, the local-changes filter, or both", () => {
    expect(noMatchesText(" sql ", false, 0)).toBe("No packages match “sql”.");
    expect(noMatchesText("", true, 0)).toBe("No packages have local changes.");
    expect(noMatchesText("sql", true, 0)).toBe("No packages with local changes match “sql”.");
  });

  it("says nothing while something matches or no filter is on", () => {
    expect(noMatchesText("sql", false, 1)).toBeNull();
    expect(noMatchesText(" ", false, 0)).toBeNull();
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
