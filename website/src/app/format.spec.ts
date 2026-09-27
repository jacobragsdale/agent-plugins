import {
  appLink,
  browseParams,
  fileTree,
  groupBySkill,
  installState,
  isJunk,
  nextVersion,
  newestFirst,
  packageIdFor,
  parseBrowse,
  parseSkillMd,
  skillMd,
  slugify,
  suggestNamespace,
  versionAtLeast,
  worksIn,
  worksInNote
} from "./format";
import type { WorksIn } from "./format";

describe("nextVersion", () => {
  it("starts at 1.0.0", () => {
    expect(nextVersion([], "patch")).toBe("1.0.0");
  });

  it("bumps past the highest version ever used, not the live one", () => {
    const used = ["1.0.0", "1.2.0", "1.10.1", "not-semver"];
    expect(nextVersion(used, "patch")).toBe("1.10.2");
    expect(nextVersion(used, "minor")).toBe("1.11.0");
    expect(nextVersion(used, "major")).toBe("2.0.0");
  });
});

describe("newestFirst", () => {
  it("orders numerically", () => {
    expect(newestFirst(["1.2.0", "1.10.0", "x", "1.9.3"])).toEqual(["1.10.0", "1.9.3", "1.2.0", "x"]);
  });

  it("puts a pre-release after its release and before older versions", () => {
    expect(newestFirst(["1.0.0", "1.1.0-beta.2", "1.1.0", "1.1.0-beta.10"])).toEqual(["1.1.0", "1.1.0-beta.10", "1.1.0-beta.2", "1.0.0"]);
  });
});

describe("slugify", () => {
  it("makes a valid package ID", () => {
    expect(slugify("  Café Meeting -- Notes! ")).toBe("cafe-meeting-notes");
    expect(slugify("x".repeat(70))).toHaveLength(64);
  });
});

describe("SKILL.md", () => {
  it("reads double-quoted values with colons, quotes, and newlines", () => {
    const text = '---\nname: "notes"\ndescription: "Use when: someone says \\"summarize\\"\\nor asks for notes."\n---\n\n# Notes\n\nSummarize.\n';
    expect(parseSkillMd(text)).toEqual({ name: "notes", description: 'Use when: someone says "summarize"\nor asks for notes.', body: "# Notes\n\nSummarize." });
  });

  it("reads plain, single-quoted, and folded descriptions", () => {
    expect(parseSkillMd("---\nname: a\ndescription: Plain text\n---\nBody")?.description).toBe("Plain text");
    expect(parseSkillMd("---\nname: a\ndescription: 'It''s quoted'\n---\nBody")?.description).toBe("It's quoted");
    expect(parseSkillMd("---\nname: a\ndescription: >\n  Folded\n  text\nother: x\n---\nBody")?.description).toBe("Folded text");
    expect(parseSkillMd("no frontmatter")).toBeNull();
  });
});

describe("isJunk", () => {
  it("drops version control and OS clutter", () => {
    expect(isJunk("skill/.git/config")).toBe(true);
    expect(isJunk("skill/.DS_Store")).toBe(true);
    expect(isJunk("skill/scripts/__pycache__/run.cpython-313.pyc")).toBe(true);
    expect(isJunk("skill/SKILL.md")).toBe(false);
  });
});

describe("groupBySkill", () => {
  it("puts each file under the nearest SKILL.md and the rest last", () => {
    const paths = ["agent-plugins.json", "skills/b/SKILL.md", "skills/b/refs/x.md", "skills/a/SKILL.md", "skills/a/nested/SKILL.md", "skills/a/nested/y.md", "skills/a/z.md"];
    const groups = groupBySkill(paths.map((path) => ({ path })));
    expect(groups.map((group) => [group.name, group.files.map((file) => file.path)])).toEqual([
      ["a", ["skills/a/SKILL.md", "skills/a/z.md"]],
      ["b", ["skills/b/SKILL.md", "skills/b/refs/x.md"]],
      ["nested", ["skills/a/nested/SKILL.md", "skills/a/nested/y.md"]],
      ["Other files", ["agent-plugins.json"]]
    ]);
  });
});

describe("fileTree", () => {
  it("nests folders at any depth, SKILL.md first", () => {
    const paths = ["s/zeta.md", "s/a/b/c/deep.md", "s/SKILL.md", "s/a/top.md", "s/a/b/mid.md"];
    const tree = fileTree(
      paths.map((path) => ({ path })),
      "s/"
    );
    expect(tree.files.map((file) => file.name)).toEqual(["SKILL.md", "zeta.md"]);
    const a = tree.folders[0];
    expect([a?.name, a?.path, a?.files.map((file) => file.name)]).toEqual(["a", "s/a/", ["top.md"]]);
    const c = a?.folders[0]?.folders[0];
    expect([c?.path, c?.files.map((file) => file.item.path)]).toEqual(["s/a/b/c/", ["s/a/b/c/deep.md"]]);
  });
});

describe("installState", () => {
  const app = { version: "0.2.0", installed: ["jacob/review", "data/sql"] };

  it("offers the app first when it's missing or too old to open links", () => {
    expect(installState(null, ["jacob/review"])).toBe("get");
    expect(installState({ ...app, version: "0.1.9" }, ["jacob/review"])).toBe("update");
    expect(installState({ ...app, version: "banana" }, ["jacob/review"])).toBe("update");
  });

  it("says installed only when every covered package is", () => {
    expect(installState(app, ["jacob/review"])).toBe("installed");
    expect(installState(app, ["jacob/review", "data/sql"])).toBe("installed");
    expect(installState(app, ["jacob/review", "data/other"])).toBe("install");
    expect(installState({ ...app, version: "1.0.0" }, [])).toBe("install");
  });

  it("offers Install while the caller isn't known yet", () => {
    expect(installState(undefined, ["jacob/review"])).toBe("install");
  });
});

describe("versionAtLeast", () => {
  it("compares numerically", () => {
    expect(versionAtLeast("0.10.0", "0.2.0")).toBe(true);
    expect(versionAtLeast("0.2.0", "0.2.0")).toBe(true);
    expect(versionAtLeast("0.2.0-beta.1", "0.2.0")).toBe(false);
  });
});

describe("appLink", () => {
  it("builds the desktop app's links", () => {
    expect(appLink("install", "jacob/review")).toBe("agent-plugins://install/jacob/review");
    expect(appLink("open", "data/pack/one")).toBe("agent-plugins://open/data/pack/one");
  });
});

describe("suggestNamespace", () => {
  it("makes a valid short name from a team's name", () => {
    expect(suggestNamespace("Data Engineering")).toBe("data-engineering");
    expect(suggestNamespace("Platform & Infrastructure Team")).toBe("platform-infrast");
    expect(suggestNamespace("42 Club")).toBe("t-42-club");
    expect(suggestNamespace("X")).toBe("");
  });
});

describe("worksIn", () => {
  const row = (kinds: readonly string[], transports: readonly string[], app: string): WorksIn | undefined => worksIn(kinds, transports).find((entry) => entry.app === app);

  it("gives skills to every app but Claude Desktop", () => {
    expect(row(["skill"], [], "Cursor")?.works).toBe("yes");
    expect(row(["skill"], [], "pi")?.works).toBe("yes");
    expect(row(["skill"], [], "Claude Desktop")).toMatchObject({ works: "no", note: expect.stringContaining("claude.ai") as unknown });
  });

  it("follows each app's MCP transports", () => {
    expect(row(["mcpServer"], ["stdio"], "Claude Desktop")?.works).toBe("yes");
    expect(row(["mcpServer"], ["streamable-http"], "Claude Desktop")?.works).toBe("no");
    expect(row(["mcpServer"], ["sse"], "OpenCode")?.works).toBe("no");
    expect(row(["mcpServer"], [], "OpenCode")?.works).toBe("yes");
    expect(row(["mcpServer"], ["stdio"], "pi")).toMatchObject({ works: "no", note: "pi doesn't use MCP servers." });
  });

  it("says some when an app takes part of a package", () => {
    expect(row(["skill", "mcpServer"], ["stdio"], "Claude Desktop")?.works).toBe("some");
    expect(row(["skill", "mcpServer"], ["stdio"], "pi")?.works).toBe("some");
    expect(row(["skill", "mcpServer"], ["stdio"], "GitHub Copilot")).toMatchObject({ works: "yes", note: null });
  });

  it("puts the main apps first and names only them on cards", () => {
    expect(
      worksIn(["skill"], [])
        .slice(0, 4)
        .map((entry) => entry.app)
    ).toEqual(["GitHub Copilot", "Cursor", "Claude Code", "Claude Desktop"]);
    expect(worksInNote(["skill"], [])).toBe("Not in Claude Desktop");
    expect(worksInNote(["mcpServer"], ["stdio"])).toBeNull();
    expect(worksInNote(["skill", "mcpServer"], ["stdio"])).toBeNull();
  });
});

describe("packageIdFor", () => {
  it("drops the space prefix and leaves room for it in the installed name", () => {
    expect(packageIdFor("Jacob Meeting Notes", "jacob")).toBe("meeting-notes");
    expect(packageIdFor("jacob-review", "jacob")).toBe("review");
    expect(packageIdFor("Review", "jacob")).toBe("review");
    expect(packageIdFor("a".repeat(80), "data-team")).toHaveLength(54);
    expect(packageIdFor(`${"a".repeat(57)} b`, "jacob")).toBe("a".repeat(57));
  });
});

describe("skillMd", () => {
  it("writes frontmatter that reads back, even with colons and quotes", () => {
    const text = skillMd({ name: "standup", description: 'Use when asked: "write my standup"', body: "# Standup\n\nKeep it short." });
    expect(parseSkillMd(text)).toEqual({ name: "standup", description: 'Use when asked: "write my standup"', body: "# Standup\n\nKeep it short." });
  });
});

describe("parseBrowse", () => {
  it("falls back to defaults for missing or unknown values", () => {
    expect(parseBrowse({})).toEqual({ q: "", lane: "all", sort: "popular", tag: null, installed: false });
    expect(parseBrowse({ lane: "nope", sort: "random", tag: "" })).toEqual({ q: "", lane: "all", sort: "popular", tag: null, installed: false });
  });

  it("round-trips through the query string", () => {
    const state = parseBrowse({ q: "report pdf", lane: "team", sort: "new", tag: "writing", installed: "true" });
    expect(state).toEqual({ q: "report pdf", lane: "team", sort: "new", tag: "writing", installed: true });
    const params = browseParams(state);
    expect(parseBrowse(Object.fromEntries(Object.entries(params).filter((entry): entry is [string, string] => entry[1] !== null)))).toEqual(state);
    expect(browseParams(parseBrowse({}))).toEqual({ q: null, lane: null, sort: null, tag: null, installed: null });
  });
});
