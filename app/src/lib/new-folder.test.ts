import { describe, expect, it, vi } from "vitest";
import type { FolderDto } from "./ipc/api";
import { IpcFailure } from "./errors";
import { NewFolderState, parentChoices, type NewFolderApi } from "./new-folder.svelte";

function folder(id: number, name: string, depth: number, role = "normal"): FolderDto {
  return {
    id,
    name,
    leaf: name.split("/").at(-1) ?? name,
    depth,
    role,
    selectable: true,
    total: 0,
    unread: 0,
  };
}

describe("emplacements d'un nouveau dossier", () => {
  it("propose le premier niveau puis les dossiers, indentés", () => {
    const folders = [
      folder(1, "INBOX", 0, "inbox"),
      folder(2, "Projets", 0),
      folder(3, "Projets/2026", 1),
    ];
    expect(parentChoices(folders, "Premier niveau", "Boîte de réception")).toEqual([
      { id: null, label: "Premier niveau", depth: 0 },
      { id: 1, label: "Boîte de réception", depth: 1 },
      { id: 2, label: "Projets", depth: 1 },
      { id: 3, label: "2026", depth: 2 },
    ]);
  });

  it("écarte les dossiers virtuels (tous les messages, suivis)", () => {
    const folders = [
      folder(1, "[Gmail]/Tous", 1, "all"),
      folder(2, "[Gmail]/Suivis", 1, "flagged"),
    ];
    expect(parentChoices(folders, "Premier niveau", "Boîte").map((c) => c.id)).toEqual([null]);
  });
});

describe("nouveau dossier", () => {
  function fakeApi(fail?: IpcFailure) {
    const api: NewFolderApi = {
      createFolder: vi.fn(async () => {
        if (fail) throw fail;
      }),
    };
    return api;
  }

  it("crée le dossier saisi à l'emplacement choisi", async () => {
    const api = fakeApi();
    const state = new NewFolderState(api);
    expect(state.canSubmit).toBe(false);
    state.name = "  Factures ";
    state.parent = 2;
    expect(state.canSubmit).toBe(true);
    expect(await state.submit(7)).toBe("Factures");
    expect(api.createFolder).toHaveBeenCalledWith(7, "  Factures ", 2);
    expect(state.error).toBeNull();
    expect(state.busy).toBe(false);
  });

  it("n'envoie rien pour un nom vide", async () => {
    const api = fakeApi();
    const state = new NewFolderState(api);
    state.name = "   ";
    expect(state.canSubmit).toBe(false);
    expect(await state.submit(7)).toBeNull();
    expect(api.createFolder).not.toHaveBeenCalled();
  });

  it("affiche le refus et garde la saisie", async () => {
    const state = new NewFolderState(fakeApi(new IpcFailure("folder_exists")));
    state.name = "Projets";
    expect(await state.submit(7)).toBeNull();
    expect(state.error).toBeTruthy();
    expect(state.name).toBe("Projets");
    expect(state.busy).toBe(false);
  });

  it("repart d'un formulaire vide", () => {
    const state = new NewFolderState(fakeApi());
    state.name = "x";
    state.parent = 3;
    state.error = "erreur";
    state.reset();
    expect([state.name, state.parent, state.error]).toEqual(["", null, null]);
  });
});
