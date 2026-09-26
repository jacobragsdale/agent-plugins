import { TestBed } from "@angular/core/testing";
import { MAT_DIALOG_DATA, MatDialogRef } from "@angular/material/dialog";
import type { Share } from "../api";
import { Api } from "../api";
import { ShareDialog } from "./share-dialog";

function view(target: string, overrides: Partial<Share>): Share {
  return { target, visibility: "private", effective: "private", users: [], teams: [], groups: [], link: null, ...overrides };
}

describe("ShareDialog", () => {
  it("shows a package that follows its space with the space's setting and list", async () => {
    const views: Record<string, Share> = {
      team: view("team", { users: [{ account: "CORP\\a", displayName: "Ada" }] }),
      "team/tool": view("team/tool", { visibility: "inherit", teams: [{ namespace: "platform", displayName: "Platform Team" }] })
    };
    TestBed.configureTestingModule({
      providers: [
        { provide: MAT_DIALOG_DATA, useValue: { namespace: "team", id: "tool", label: "Tool", spaceName: "Data Team", owners: "members of Data Team" } },
        { provide: MatDialogRef, useValue: { close: () => undefined } },
        { provide: Api, useValue: { share: async (ns: string, id?: string): Promise<Share | undefined> => Promise.resolve(views[id === undefined ? ns : `${ns}/${id}`]) } }
      ]
    });
    const fixture = TestBed.createComponent(ShareDialog);
    await fixture.whenStable();
    fixture.detectChanges();
    const element: unknown = fixture.nativeElement;
    const text = element instanceof HTMLElement ? element.textContent : "";
    expect(text).toContain("Same as Data Team (private)");
    expect(text).toContain("It follows Data Team. Only members of Data Team");
    expect(text).toContain("Platform Team");
    expect(text).not.toContain("Everyone who signs in");
  });
});
