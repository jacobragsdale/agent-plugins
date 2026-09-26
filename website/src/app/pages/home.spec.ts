import { downloadsSchema } from "./home";

describe("downloadsSchema", () => {
  const manifest = (file: string): unknown => ({
    schemaVersion: 1,
    release: { version: "0.2.2", platforms: [{ id: "windows", name: "Windows", format: ".exe", architecture: "x64", file, sizeBytes: 5038312, sha256: "a".repeat(64) }] }
  });

  it("accepts the installer name Tauri writes, which has a space", () => {
    expect(downloadsSchema.safeParse(manifest("releases/0.2.2/Agent Plugins_0.2.2_x64-setup.exe")).success).toBe(true);
  });

  it("refuses paths that leave the downloads folder", () => {
    expect(downloadsSchema.safeParse(manifest("../secret.exe")).success).toBe(false);
    expect(downloadsSchema.safeParse(manifest("/etc/passwd")).success).toBe(false);
  });
});
