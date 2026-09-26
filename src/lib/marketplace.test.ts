import { describe, expect, it } from "vitest";
import fixture from "../ipc/fixtures/app-state.json";
import { appStateSchema, itemStatusSchema } from "../ipc/schemas";
import type { BundleState, CatalogItem, ItemStatus } from "../ipc/schemas";
import {
  appsPhrase,
  bundleSummary,
  cardDomId,
  ID_PATTERN,
  linkAction,
  listPhrase,
  NAMESPACE_PATTERN,
  ownsSpace,
  resolveLink,
  spaceLabel,
  suggestId,
  suggestNamespace,
  typedAccount
} from "./marketplace";

const state = appStateSchema.parse(fixture);

function bundle(members: readonly string[]): BundleState {
  return {
    id: "team-data/kit",
    namespace: "team-data",
    bundleId: "kit",
    name: "Kit",
    description: "",
    publisher: "Data team",
    lane: "team",
    members,
    updatedAt: "2026-09-20T12:00:00Z",
    restricted: false,
    sharedWithYou: false
  };
}

function withStatus(id: string, status: ItemStatus): CatalogItem {
  const item = state.items.find((candidate) => candidate.id === id);
  if (item === undefined) {
    throw new Error(`fixture has no ${id}`);
  }
  return { ...item, status };
}

describe("suggestNamespace", () => {
  it("turns a team name into a valid space name", () => {
    expect(suggestNamespace("Data Team")).toBe("data-team");
    expect(suggestNamespace("  Équipe Données  ")).toBe("equipe-donnees");
    expect(suggestNamespace("2026 Finance & Ops")).toBe("finance-ops");
    expect(suggestNamespace("A very long team name indeed")).toBe("a-very-long-team");
    for (const name of ["Data Team", "Équipe Données", "2026 Finance & Ops", "A very long team name indeed"]) {
      expect(NAMESPACE_PATTERN.test(suggestNamespace(name))).toBe(true);
    }
  });

  it("leaves nothing to suggest when the name has no letters", () => {
    expect(suggestNamespace("1234")).toBe("");
    expect(suggestId("Starter kit!")).toBe("starter-kit");
    expect(ID_PATTERN.test(suggestId("Starter kit!"))).toBe(true);
  });
});

describe("links", () => {
  it("never uninstalls or restores from a link", () => {
    for (const status of itemStatusSchema.options) {
      expect(["install", "installed", "blocked"]).toContain(linkAction(status));
    }
    expect(linkAction("installed")).toBe("installed");
    expect(linkAction("modified")).toBe("installed");
    expect(linkAction("updateAvailable")).toBe("install");
    expect(linkAction("sourceConflict")).toBe("blocked");
  });

  it("finds a package, one of its parts, or a bundle", () => {
    expect(resolveLink({ kind: "install", namespace: "official", id: "publish", component: null }, state)).toMatchObject({ kind: "item", componentId: null });
    expect(resolveLink({ kind: "install", namespace: "team-data", id: "sql-helper", component: "sql-helper-db" }, state)).toBeNull();
    const part = state.items.find((item) => item.id === "team-data/sql-helper")?.components[0]?.id ?? "";
    expect(resolveLink({ kind: "install", namespace: "team-data", id: "sql-helper", component: part }, state)).toMatchObject({ kind: "item", componentId: part });
    const withBundle = { ...state, bundles: [bundle(["official/publish", "gone/thing"])] };
    const found = resolveLink({ kind: "install", namespace: "team-data", id: "kit", component: null }, withBundle);
    expect(found?.kind === "bundle" ? found.members.map((item) => item.id) : null).toEqual(["official/publish"]);
    expect(resolveLink({ kind: "install", namespace: "team-data", id: "kit", component: "x" }, withBundle)).toBeNull();
  });

  it("reports a miss so the window can check for it first", () => {
    expect(resolveLink({ kind: "open", namespace: "nobody", id: "nothing", component: null }, state)).toBeNull();
  });

  it("gives every card an id a link can scroll to", () => {
    expect(cardDomId("team-data/sql-helper")).toBe("card-team-data_sql-helper");
    expect(cardDomId("team-data")).not.toBe(cardDomId("team-data/x"));
  });
});

describe("bundleSummary", () => {
  it("offers Install all, then the rest, then Uninstall all", () => {
    const kit = bundle(["official/review", "official/tone"]);
    expect(bundleSummary(kit, [withStatus("official/review", "available"), withStatus("official/tone", "available")]).action).toBe("install");
    const partly = bundleSummary(kit, [withStatus("official/review", "installed"), withStatus("official/tone", "available")]);
    expect([partly.action, partly.installed, partly.members.length]).toEqual(["installRest", 1, 2]);
    expect(bundleSummary(kit, [withStatus("official/review", "updateAvailable"), withStatus("official/tone", "installed")]).action).toBe("installed");
  });

  it("counts members this person can't see without offering them", () => {
    const summary = bundleSummary(bundle(["official/review", "hidden/one", "hidden/two"]), [withStatus("official/review", "available")]);
    expect([summary.members.length, summary.missing]).toEqual([1, 2]);
    expect(bundleSummary(bundle(["hidden/one"]), state.items).action).toBe("empty");
  });
});

describe("spaces and people", () => {
  it("names spaces the way a person thinks of them", () => {
    const identity = state.identity;
    expect(spaceLabel(identity, identity?.namespace ?? "")).toBe("Just me");
    expect(spaceLabel(identity, "official")).toBe("Official");
    expect(spaceLabel(identity, "team-data")).toBe("Data team");
    expect(ownsSpace(identity, "team-data")).toBe(true);
    expect(ownsSpace(identity, "official")).toBe(false);
    expect(ownsSpace(null, "team-data")).toBe(false);
  });

  it("accepts a typed Windows account only in full", () => {
    expect(typedAccount(" CORP\\jane ")).toBe("CORP\\jane");
    expect(typedAccount("jane@corp.example")).toBe("jane@corp.example");
    expect(typedAccount("jane")).toBeNull();
    expect(typedAccount("CORP\\")).toBeNull();
  });

  it("lists names and apps in plain words", () => {
    expect(listPhrase(["Cursor"])).toBe("Cursor");
    expect(listPhrase(["Cursor", "Claude Code", "Codex"])).toBe("Cursor, Claude Code and Codex");
    expect(appsPhrase([])).toBe("the AI apps Agent Plugins finds");
  });
});
