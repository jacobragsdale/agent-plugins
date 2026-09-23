import { fileTree, groupBySkill, isJunk, nextVersion, newestFirst, parseSkillMd, slugify } from "./format";

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
