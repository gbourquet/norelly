import { describe, expect, it, vi } from "vitest";
import type { FolderDto } from "./ipc/api";
import { IpcFailure } from "./errors";
import {
  DeleteFolderState,
  RenameFolderState,
  isEditableFolder,
  type FolderEditApi,
} from "./folder-edit.svelte";

function folder(id: number, name: string, role = "normal", total = 0): FolderDto {
  return {
    id,
    name,
    leaf: name.split("/").at(-1) ?? name,
    depth: name.split("/").length - 1,
    role,
    selectable: true,
    total,
    unread: 0,
  };
}

function fakeApi(fail?: IpcFailure) {
  const api: FolderEditApi = {
    renameFolder: vi.fn(async () => {
      if (fail) throw fail;
      return 42;
    }),
    deleteFolder: vi.fn(async () => {
      if (fail) throw fail;
    }),
  };
  return api;
}

describe("dossiers modifiables", () => {
  it("seuls les dossiers ordinaires se renomment ou se suppriment", () => {
    expect(isEditableFolder(folder(1, "Projets"))).toBe(true);
    for (const role of ["inbox", "sent", "drafts", "trash", "junk", "archive", "all", "flagged"]) {
      expect(isEditableFolder(folder(1, "x", role)), role).toBe(false);
    }
  });
});

describe("renommer un dossier", () => {
  it("part du nom actuel et n'envoie rien tant qu'il est inchangé", async () => {
    const api = fakeApi();
    const state = new RenameFolderState(api);
    const projets = folder(5, "Clients/Projets");
    state.reset(projets);
    expect(state.name).toBe("Projets");
    expect(state.canSubmit).toBe(false);
    state.name = " Projets ";
    expect(state.canSubmit).toBe(false);
    state.name = "  ";
    expect(state.canSubmit).toBe(false);
    expect(await state.submit(7, projets)).toBeNull();
    expect(api.renameFolder).not.toHaveBeenCalled();
  });

  it("renomme et renvoie le nouveau nom et l'identifiant", async () => {
    const api = fakeApi();
    const state = new RenameFolderState(api);
    const projets = folder(5, "Projets");
    state.reset(projets);
    state.name = " Chantiers ";
    expect(state.canSubmit).toBe(true);
    expect(await state.submit(7, projets)).toEqual({ id: 42, name: "Chantiers" });
    expect(api.renameFolder).toHaveBeenCalledWith(7, 5, " Chantiers ");
    expect(state.busy).toBe(false);
    expect(state.error).toBeNull();
  });

  it("affiche le refus et garde la saisie", async () => {
    const state = new RenameFolderState(fakeApi(new IpcFailure("folder_exists")));
    const projets = folder(5, "Projets");
    state.reset(projets);
    state.name = "Factures";
    expect(await state.submit(7, projets)).toBeNull();
    expect(state.error).toBeTruthy();
    expect(state.name).toBe("Factures");
    expect(state.busy).toBe(false);
  });
});

describe("supprimer un dossier", () => {
  it("supprime le dossier", async () => {
    const api = fakeApi();
    const state = new DeleteFolderState(api);
    expect(await state.submit(7, folder(5, "Projets", "normal", 3))).toBe(true);
    expect(api.deleteFolder).toHaveBeenCalledWith(7, 5);
    expect(state.busy).toBe(false);
  });

  it("affiche le refus", async () => {
    const state = new DeleteFolderState(fakeApi(new IpcFailure("folder_has_subfolders")));
    expect(await state.submit(7, folder(5, "Projets"))).toBe(false);
    expect(state.error).toBeTruthy();
    state.reset();
    expect(state.error).toBeNull();
  });
});
