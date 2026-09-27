import { confirm } from "@tauri-apps/plugin-dialog";
import { describe, expect, it, vi } from "vitest";
import fixture from "../ipc/fixtures/app-state.json";
import { appStateSchema, itemStatusSchema } from "../ipc/schemas";
import {
  commandForStatus,
  componentLabel,
  detectionUnsure,
  failuresError,
  hasDetectedAgent,
  itemCommand,
  outcomeNotice,
  packageName,
  primaryActionColor,
  primaryActionLabel,
  reportNotice,
  reviewReset,
  reviewSourceRemoval,
  statusColor,
  statusLabel,
  supportsBulkAction,
  uninstalledNotice
} from "./status";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(() => Promise.resolve(true)) }));

const state = appStateSchema.parse(fixture);
const statuses = itemStatusSchema.options;

describe("status labels", () => {
  it("uses plain, sentence-case words for every status", () => {
    for (const status of statuses) {
      const label = statusLabel(status);
      expect(label).toMatch(/^[A-Z][a-z ]+$/);
      expect(label).not.toMatch(/conflict|upstream|unmanaged/i);
    }
  });

  it("names MCP servers as connectors", () => {
    expect(componentLabel("mcpServer")).toBe("Connector");
    expect(componentLabel("skill")).toBe("Skill");
  });

  it("offers an action label for every status", () => {
    for (const status of statuses) {
      expect(primaryActionLabel(status).length).toBeGreaterThan(0);
    }
  });
});

describe("commandForStatus", () => {
  it("never touches a package another source installed", () => {
    expect(commandForStatus("sourceConflict")).toBeNull();
  });

  it("restores a changed package by replacing it, which backs up the changed copy", () => {
    expect(commandForStatus("modified")).toBe("replace_item");
    expect(primaryActionLabel("modified")).toBe("Restore original…");
  });

  it("offers only Uninstall for a package whose files are missing", () => {
    expect(statusLabel("missing")).toBe("Restoring on next check");
    expect(statusColor("missing")).not.toBe("red");
    expect(primaryActionLabel("missing")).toBe("Uninstall");
    expect(primaryActionColor("missing")).toBe("red");
    expect(commandForStatus("missing")).toBe("uninstall_item");
    expect(supportsBulkAction("missing", "install")).toBe(false);
    expect(supportsBulkAction("missing", "replace")).toBe(false);
  });

  it("maps each actionable status to its command", () => {
    expect(commandForStatus("conflict")).toBe("replace_item");
    expect(commandForStatus("installed")).toBe("uninstall_item");
    expect(commandForStatus("removed")).toBe("uninstall_item");
    expect(commandForStatus("available")).toBe("install_item");
    expect(commandForStatus("updateAvailable")).toBe("install_item");
    expect(commandForStatus("partiallyInstalled")).toBe("install_item");
  });
});

describe("supportsBulkAction", () => {
  it("replaces only conflicts and never uninstalls what is not installed", () => {
    expect(statuses.filter((status) => supportsBulkAction(status, "replace"))).toEqual(["conflict"]);
    expect(supportsBulkAction("available", "uninstall")).toBe(false);
    expect(supportsBulkAction("modified", "install")).toBe(false);
  });

  it("includes missing packages in Uninstall all", () => {
    expect(supportsBulkAction("missing", "uninstall")).toBe(true);
  });
});

describe("names instead of ids", () => {
  it("falls back to the id for a package the window no longer lists", () => {
    expect(packageName(state.items, "official/publish")).toBe("Publish");
    expect(packageName(state.items, "gone/package")).toBe("gone/package");
  });

  it("reports the packages a background sync updated", () => {
    const autoUpdateReport = {
      updatedItems: [{ id: "official/publish", sourceId: "official", localId: "publish", toVersion: "1.4.0" }],
      failedItems: [{ id: "team-data/sql-helper", message: "The process cannot access the file because it is being used by another process. (os error 32)" }],
      repairedItems: [],
      extendedItems: [],
      removedItems: [],
      releasedItems: [],
      stillPulled: []
    };
    expect(reportNotice({ ...state, notifications: [], autoUpdateReport })).toEqual({ text: "Updated Publish to 1.4.0. Details shows what changed." });
  });

  it("says why a package was removed from this PC", () => {
    const report = { updatedItems: [], failedItems: [], repairedItems: [], extendedItems: [], removedItems: ["Old helper"], releasedItems: [], stillPulled: [] };
    expect(reportNotice({ ...state, notifications: [], autoUpdateReport: report })).toEqual({ text: "Removed Old helper: its publisher or an admin pulled it from every PC." });
    const both = { ...report, updatedItems: [{ id: "official/publish", sourceId: "official", localId: "publish" }], removedItems: ["A", "B"] };
    expect(reportNotice({ ...state, autoUpdateReport: both })?.text).toBe(
      "Updated Publish. Details shows what changed. Removed A, B: their publishers or an admin pulled them from every PC. Dana suggested a change to SQL helper."
    );
    const stuck = { ...report, removedItems: [], stillPulled: ["Probe was pulled from every PC and removed from the other apps. Cursor uses a settings file …"] };
    expect(reportNotice({ ...state, notifications: [], autoUpdateReport: stuck })?.text).toBe(stuck.stillPulled[0]);
  });

  it("stays quiet about repairs and updates the next sync retries", () => {
    expect(
      reportNotice({ ...state, notifications: [], autoUpdateReport: { updatedItems: [], failedItems: [], repairedItems: [], extendedItems: [], removedItems: [], releasedItems: [], stillPulled: [] } })
    ).toBeNull();
    const quiet = {
      updatedItems: [],
      failedItems: [{ id: "team-data/sql-helper", message: "locked" }],
      repairedItems: ["Publish"],
      extendedItems: ["SQL helper"],
      removedItems: [],
      releasedItems: ["Review"],
      stillPulled: []
    };
    expect(reportNotice({ ...state, notifications: [], autoUpdateReport: quiet })).toBeNull();
  });

  it("summarizes bulk failures by name and source", () => {
    const error = failuresError("uninstall", [{ id: "team-data/sql-helper", message: "Access is denied. (os error 5)" }], state.items, "Data team");
    expect(error.summary).toBe("Couldn't uninstall 1 package from Data team: SQL helper.");
    expect(error.detail).toBe("team-data/sql-helper: Access is denied. (os error 5)");
  });
});

describe("uninstalledNotice", () => {
  it("says which connector settings stay saved", () => {
    const connector = {
      componentId: "db",
      name: "Database",
      summary: "Starts uvx.",
      detail: "uvx db",
      environment: ["DB_TOKEN", "DB_URL"],
      missingEnvironment: ["DB_URL"],
      missingProgram: null,
      apps: [],
      installedApps: [],
      changed: false
    };
    const [first] = state.items;
    if (first === undefined) {
      throw new Error("the fixture lists items");
    }
    const item = { ...first, connectors: [connector] };
    expect(uninstalledNotice(item, undefined, "Backed up to", { backupPaths: [], warnings: [] })?.text).toBe(
      "DB_TOKEN stays saved for your Windows account, in case you install it again. Remove it in Edit environment variables for your account if you no longer need it."
    );
    expect(uninstalledNotice({ ...item, connectors: [] }, undefined, "Backed up to", { backupPaths: [], warnings: [] })).toBeNull();
  });
});

describe("outcomeNotice", () => {
  it("says nothing after a clean action", () => {
    expect(outcomeNotice("Backed up to", { backupPaths: [], warnings: [] })).toBeNull();
  });

  it("shows skipped apps as an amber note, with the backup folder to open", () => {
    expect(outcomeNotice("Your changed copy was backed up to", { backupPaths: ["C:\\backup"], warnings: ["Skipped Claude Desktop: its settings file could not be read."] })).toEqual({
      text: "Skipped Claude Desktop: its settings file could not be read. Your changed copy was backed up to C:\\backup.",
      folder: "C:\\backup",
      caution: true
    });
  });
});

describe("agent detection", () => {
  const [profile] = state.agentProfiles;
  if (profile === undefined) {
    throw new Error("The fixture needs an agent profile.");
  }
  const unsure = { ...profile, detected: false, enabled: true, detectionMessage: "Timed out reading the registry." };

  it("treats an inconclusive check as unknown, not absent", () => {
    expect(detectionUnsure(unsure)).toBe(true);
    expect(hasDetectedAgent([unsure])).toBe(true);
  });

  it("treats a disabled or silent miss as not installed", () => {
    expect(detectionUnsure({ ...unsure, enabled: false })).toBe(false);
    expect(detectionUnsure({ ...unsure, detectionMessage: null })).toBe(false);
    expect(hasDetectedAgent([{ ...unsure, detectionMessage: null }])).toBe(false);
  });
});

describe("itemCommand", () => {
  const tone = state.items.find((item) => item.status === "modified");
  const missing = state.items.find((item) => item.status === "missing");

  it("restores a changed package through replace, naming the backup of the changed copy", () => {
    expect(tone === undefined ? null : itemCommand(tone, undefined)).toMatchObject({ command: "replace_item", verb: "restore", backupLead: "Your changed copy was backed up to" });
  });

  it("uninstalls a package whose files are missing, without asking to replace anything", () => {
    expect(missing === undefined ? null : itemCommand(missing, undefined)).toMatchObject({ command: "uninstall_item", review: null, trustApproved: false });
  });

  it("asks before uninstalling a package that can never be installed again", () => {
    const removed = state.items.find((item) => item.status === "removed");
    const installed = state.items.find((item) => item.status === "installed");
    expect(removed === undefined ? null : itemCommand(removed, undefined)?.review).not.toBeNull();
    expect(installed === undefined ? "none" : itemCommand(installed, undefined)?.review).toBeNull();
  });

  it("ignores a part the package does not have", () => {
    expect(tone === undefined ? "none" : itemCommand(tone, "no-such-part")).toBeNull();
  });
});

describe("removal prompts", () => {
  const prompt = (): string => String(vi.mocked(confirm).mock.lastCall?.[0]);
  const plan = { sourceId: "official", items: [] };
  const [official] = state.sources;

  it("says a source the catalog lists comes back without its packages", async () => {
    if (official === undefined) {
      throw new Error("fixture has no source");
    }
    await reviewSourceRemoval(official, plan, state.repositories);
    expect(prompt()).toContain("it will be added back on the next check. Its packages will not be reinstalled.");
    await reviewSourceRemoval(official, plan, []);
    expect(prompt()).not.toContain("added back");
  });

  it("says a reset keeps the catalog's sources", async () => {
    await reviewReset();
    expect(prompt()).toContain("Sources from the catalog are added back automatically.");
  });
});
