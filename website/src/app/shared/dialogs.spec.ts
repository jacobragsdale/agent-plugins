import { TestBed } from "@angular/core/testing";
import { MAT_DIALOG_DATA, MatDialogRef } from "@angular/material/dialog";
import type { Access } from "../api";
import { Api } from "../api";
import { VisibilityDialog } from "./dialogs";

describe("VisibilityDialog", () => {
  it("shows a package without its own rule following its space's restriction", async () => {
    const rules: Record<string, Access> = { team: { target: "team", users: ["CORP\\a"], groups: [] }, "team/tool": { target: "team/tool", users: [], groups: [] } };
    TestBed.configureTestingModule({
      providers: [
        { provide: MAT_DIALOG_DATA, useValue: { namespace: "team", packageId: "tool", label: "tool" } },
        { provide: MatDialogRef, useValue: { close: () => undefined } },
        { provide: Api, useValue: { access: async (ns: string, packageId?: string) => Promise.resolve(rules[packageId === undefined ? ns : `${ns}/${packageId}`]) } }
      ]
    });
    const fixture = TestBed.createComponent(VisibilityDialog);
    await fixture.whenStable();
    fixture.detectChanges();
    const element: unknown = fixture.nativeElement;
    const text = element instanceof HTMLElement ? element.textContent : "";
    expect(text).not.toContain("Everyone who signs in");
    expect(text).toContain("CORP\\a");
  });
});
