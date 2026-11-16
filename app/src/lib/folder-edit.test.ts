import { describe, expect, it, vi } from "vitest";
import type { FolderDto } from "./ipc/api";
import { IpcFailure } from "./errors";
import {
  DeleteFolderState,
  MoveFolderState,
  RenameFolderState,
  destinationChoices,
  isEditableFolder,
  type FolderEditApi,
} from "./folder-edit.svelte";

function folder(id: number, name: string, role = "normal", total = 0): FolderDto {
  return {
    id,
    name,
    leaf: name.split("/").at(-1) ?? name,
    depth: name.split("/").length - 1,
    indent: name.split("/").length - 1,
    label: name.split("/").at(-1) ?? name,
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
    moveFolder: vi.fn(async () => {
      if (fail) throw fail;
      return 43;
    }),
  };
  return api;
}

describe("résultat inconnu", () => {
  const unknown = () => new IpcFailure("folder_outcome_unknown", null, "délai");
  const text =
    "Le serveur n'a pas confirmé la modification : elle a peut-être été faite. " +
    "La liste des dossiers va être mise à jour : vérifiez-la avant de réessayer. (« délai »)";

  it("renommer : relit la liste des dossiers et le dit", async () => {
    const reload = vi.fn(async () => {});
    const state = new RenameFolderState(fakeApi(unknown()), reload);
    const projets = folder(5, "Projets");
    state.reset(projets);
    state.name = "Chantiers";
    expect(await state.submit(7, projets)).toBeNull();
    expect(reload).toHaveBeenCalledWith(7);
    expect(state.error).toBe(text);
    expect(state.busy).toBe(false);
  });

  it("déplacer : relit la liste des dossiers", async () => {
    const reload = vi.fn(async () => {});
    const state = new MoveFolderState(fakeApi(unknown()), reload);
    const bli = folder(4, "Bli");
    state.reset(bli, [folder(2, "Bla"), bli]);
    state.parent = 2;
    expect(await state.submit(7, bli)).toBeNull();
    expect(reload).toHaveBeenCalledWith(7);
    expect(state.error).toBe(text);
  });

  it("supprimer : relit la liste des dossiers", async () => {
    const reload = vi.fn(async () => {});
    const state = new DeleteFolderState(fakeApi(unknown()), reload);
    expect(await state.submit(7, folder(5, "Projets"))).toBe(false);
    expect(reload).toHaveBeenCalledWith(7);
    expect(state.error).toBe(text);
  });

  it("refus ordinaire : la liste n'est pas relue", async () => {
    const reload = vi.fn(async () => {});
    const state = new DeleteFolderState(fakeApi(new IpcFailure("folder_delete_failed")), reload);
    await state.submit(7, folder(5, "Projets"));
    expect(reload).not.toHaveBeenCalled();
  });
});

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

describe("déplacer un dossier", () => {
  const tree = [
    folder(1, "INBOX", "inbox"),
    folder(2, "Bla"),
    folder(3, "Bla/Sous"),
    folder(4, "Bli"),
    folder(5, "Bli/2026"),
    folder(6, "Bli/2026/Mars"),
    folder(7, "Tous", "all"),
    folder(8, "Zed"),
  ];

  it("propose tous les emplacements sauf le dossier et ses sous-dossiers", () => {
    const bli = folder(4, "Bli");
    expect(destinationChoices(tree, bli, "Premier niveau")).toEqual([
      { id: null, label: "Premier niveau", depth: 0 },
      { id: 1, label: "Boîte de réception", depth: 1 },
      { id: 2, label: "Bla", depth: 1 },
      { id: 3, label: "Sous", depth: 2 },
      { id: 8, label: "Zed", depth: 1 },
    ]);
  });

  it("écarte aussi les sous-dossiers écrits avec une autre casse", () => {
    // Serveur insensible à la casse : « bli/Autre » est sous « Bli ».
    const bli = folder(4, "Bli");
    const mixed = [...tree, folder(9, "bli/Autre"), folder(10, "BLI")];
    const ids = destinationChoices(mixed, bli, "Premier niveau").map((c) => c.id);
    expect(ids).not.toContain(9);
    expect(ids).not.toContain(10);
    // Majuscule et minuscule de longueurs différentes, comme en Rust.
    const upper = folder(11, "Ⱥ");
    const lower = [...tree, upper, folder(12, "ⱥ/x")];
    const choices = destinationChoices(lower, upper, "Premier niveau").map((c) => c.id);
    expect(choices).not.toContain(12);
  });

  it("part de l'emplacement actuel et n'envoie rien tant qu'il est inchangé", async () => {
    const api = fakeApi();
    const state = new MoveFolderState(api);
    const sub = folder(5, "Bli/2026");
    state.reset(sub, tree);
    expect(state.parent).toBe(4);
    expect(state.canSubmit).toBe(false);
    expect(await state.submit(7, sub)).toBeNull();
    expect(api.moveFolder).not.toHaveBeenCalled();
    state.reset(folder(4, "Bli"), tree);
    expect(state.parent).toBeNull();
  });

  it("déplace et renvoie le nom et l'identifiant à la nouvelle place", async () => {
    const api = fakeApi();
    const state = new MoveFolderState(api);
    const bli = folder(4, "Bli");
    state.reset(bli, tree);
    state.parent = 2;
    expect(state.canSubmit).toBe(true);
    expect(await state.submit(7, bli)).toEqual({ id: 43, name: "Bli" });
    expect(api.moveFolder).toHaveBeenCalledWith(7, 4, 2);
    expect(state.busy).toBe(false);
    expect(state.error).toBeNull();
  });

  it("lit la hiérarchie dans les noms, pas dans l'ordre de la liste", async () => {
    // Ordre réel de la liste : rôles spéciaux d'abord, puis par nom ; un
    // sous-dossier de la boîte de réception suit la corbeille.
    const listed = [
      folder(1, "INBOX", "inbox"),
      folder(9, "Trash", "trash"),
      folder(2, "Bla"),
      folder(10, "INBOX/Potager"),
      folder(11, "INBOX/Potager/Semis"),
      folder(12, "Trash/Vieux"),
    ];
    const potager = folder(10, "INBOX/Potager");
    expect(destinationChoices(listed, potager, "P").map((c) => c.id)).toEqual([null, 1, 9, 2, 12]);
    const state = new MoveFolderState(fakeApi());
    state.reset(potager, listed);
    expect(state.parent).toBe(1);
    state.reset(folder(12, "Trash/Vieux"), listed);
    expect(state.parent).toBe(9);
    // Un nom qui commence pareil n'est pas un sous-dossier.
    const similar = [folder(2, "Bla"), folder(3, "Bla bla"), folder(4, "Bla bla/x")];
    expect(destinationChoices(similar, folder(2, "Bla"), "P").map((c) => c.id)).toEqual([
      null,
      3,
      4,
    ]);
  });

  it("parent non listé : chaque emplacement, premier niveau compris, est un déplacement", async () => {
    // `INBOX/Perso` n'est pas annoncé : le dossier n'est ni au premier
    // niveau, ni directement dans la boîte de réception où il est affiché.
    const banque = folder(13, "INBOX/Perso/Banque");
    const listed = [folder(1, "INBOX", "inbox"), banque];
    const api = fakeApi();
    const state = new MoveFolderState(api);
    state.reset(banque, listed);
    expect(state.parent).toBeNull();
    expect(state.canSubmit).toBe(true);
    expect(await state.submit(7, banque)).toEqual({ id: 43, name: "Banque" });
    expect(api.moveFolder).toHaveBeenCalledWith(7, 13, null);
    state.reset(banque, listed);
    state.parent = 1;
    expect(state.canSubmit).toBe(true);
  });

  it("affiche le refus et garde le choix", async () => {
    const state = new MoveFolderState(fakeApi(new IpcFailure("folder_exists")));
    const bli = folder(4, "Bli");
    state.reset(bli, tree);
    state.parent = 2;
    expect(await state.submit(7, bli)).toBeNull();
    expect(state.error).toBeTruthy();
    expect(state.parent).toBe(2);
    expect(state.busy).toBe(false);
  });
});
