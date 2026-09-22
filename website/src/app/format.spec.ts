import { buildSkillMd, editSkillMd, isJunk, nextVersion, newestFirst, parseSkillMd, slugify } from "./format";

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
});

describe("slugify", () => {
  it("makes a valid package ID", () => {
    expect(slugify("  Café Meeting -- Notes! ")).toBe("cafe-meeting-notes");
    expect(slugify("x".repeat(70))).toHaveLength(64);
  });
});

describe("SKILL.md", () => {
  it("quotes values so colons, quotes, and newlines survive", () => {
    const text = buildSkillMd({ name: "notes", description: 'Use when: someone says "summarize"\nor asks for notes.', body: "# Notes\n\nSummarize." });
    expect(text).toBe('---\nname: "notes"\ndescription: "Use when: someone says \\"summarize\\"\\nor asks for notes."\n---\n\n# Notes\n\nSummarize.\n');
    expect(parseSkillMd(text)).toEqual({ name: "notes", description: 'Use when: someone says "summarize"\nor asks for notes.', body: "# Notes\n\nSummarize." });
  });

  it("reads plain, single-quoted, and folded descriptions", () => {
    expect(parseSkillMd("---\nname: a\ndescription: Plain text\n---\nBody")?.description).toBe("Plain text");
    expect(parseSkillMd("---\nname: a\ndescription: 'It''s quoted'\n---\nBody")?.description).toBe("It's quoted");
    expect(parseSkillMd("---\nname: a\ndescription: >\n  Folded\n  text\nother: x\n---\nBody")?.description).toBe("Folded text");
    expect(parseSkillMd("no frontmatter")).toBeNull();
  });

  it("edits the description and body and keeps every other key", () => {
    const original = "---\nname: review\ndescription: >\n  Old\n  text\nallowed-tools: Read\n---\n\nOld body\n";
    expect(editSkillMd(original, "New: text", "New body")).toBe('---\nname: review\ndescription: "New: text"\nallowed-tools: Read\n---\n\nNew body\n');
  });
});

describe("isJunk", () => {
  it("drops version control and OS clutter", () => {
    expect(isJunk("skill/.git/config")).toBe(true);
    expect(isJunk("skill/.DS_Store")).toBe(true);
    expect(isJunk("skill/SKILL.md")).toBe(false);
  });
});
