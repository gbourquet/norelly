import { describe, expect, it, vi } from "vitest";
import type { ContactImportPreviewDto } from "./ipc/api";
import { IpcFailure } from "./errors";
import { ContactImportState, type ContactImportApi } from "./contact-import.svelte";

function preview(over: Partial<ContactImportPreviewDto> = {}): ContactImportPreviewDto {
  return {
    token: 4,
    file_name: "contacts.csv",
    format: "google_csv",
    cards: 6,
    skip: { create: 3, complete: 0, duplicates: 2 },
    complete: { create: 3, complete: 1, duplicates: 1 },
    renamed: 0,
    rejected_total: 1,
    rejected: [{ card: 6, hint: "Jean", reason: "no_address" }],
    ...over,
  };
}

function fakeApi(result: ContactImportPreviewDto | null | Error = preview()) {
  return {
    previewContactImport: vi.fn(async () => {
      if (result instanceof Error) throw result;
      return result;
    }),
    applyContactImport: vi.fn(async () => ({ created: 3, completed: 1 })),
  } satisfies ContactImportApi;
}

describe("import de contacts", () => {
  it("ouvre le sélecteur avec ses libellés et garde l'aperçu", async () => {
    const api = fakeApi();
    const state = new ContactImportState(api);
    expect(await state.choose("Choisir", "Fichiers de contacts")).toBe(true);
    expect(api.previewContactImport).toHaveBeenCalledWith("Choisir", "Fichiers de contacts");
    expect(state.preview?.token).toBe(4);
    expect(state.policy).toBe("skip");
    expect(state.counts).toEqual({ create: 3, complete: 0, duplicates: 2 });
    expect(state.canImport).toBe(true);
    // Doublons présents : le choix de la politique est proposé.
    expect(state.offersPolicy).toBe(true);
    state.policy = "complete";
    expect(state.counts).toEqual({ create: 3, complete: 1, duplicates: 1 });
  });

  it("annulation du sélecteur : rien à afficher", async () => {
    const state = new ContactImportState(fakeApi(null));
    expect(await state.choose("t", "f")).toBe(false);
    expect(state.preview).toBeNull();
    expect(state.error).toBeNull();
  });

  it("fichier refusé : l'erreur est rendue, pas d'aperçu", async () => {
    const state = new ContactImportState(fakeApi(new IpcFailure("contact_import_unknown_format")));
    await expect(state.choose("t", "f")).rejects.toBeInstanceOf(IpcFailure);
    expect(state.preview).toBeNull();
  });

  it("sans doublon ni rien à importer : ni choix ni import", async () => {
    const empty = { create: 0, complete: 0, duplicates: 0 };
    const state = new ContactImportState(
      fakeApi(preview({ skip: empty, complete: empty, rejected_total: 2 })),
    );
    await state.choose("t", "f");
    expect(state.offersPolicy).toBe(false);
    expect(state.canImport).toBe(false);
  });

  it("doublons à compléter seulement : import possible en complétant", async () => {
    const state = new ContactImportState(
      fakeApi(
        preview({
          skip: { create: 0, complete: 0, duplicates: 2 },
          complete: { create: 0, complete: 2, duplicates: 0 },
        }),
      ),
    );
    await state.choose("t", "f");
    expect(state.canImport).toBe(false);
    state.policy = "complete";
    expect(state.canImport).toBe(true);
  });

  it("confirme avec le jeton et la politique choisie, puis oublie l'aperçu", async () => {
    const api = fakeApi();
    const state = new ContactImportState(api);
    await state.choose("t", "f");
    state.policy = "complete";
    const done = state.confirm();
    expect(state.busy).toBe(true);
    expect(await done).toEqual({ created: 3, completed: 1 });
    expect(api.applyContactImport).toHaveBeenCalledWith(4, "complete");
    expect(state.busy).toBe(false);
    expect(state.preview).toBeNull();
  });

  it("échec de l'import : erreur affichée, aperçu gardé pour réessayer", async () => {
    const api = fakeApi();
    api.applyContactImport.mockRejectedValueOnce(new IpcFailure("contact_duplicate_address"));
    const state = new ContactImportState(api);
    await state.choose("t", "f");
    expect(await state.confirm()).toBeNull();
    expect(state.error).not.toBeNull();
    expect(state.preview).not.toBeNull();
    expect(state.busy).toBe(false);
  });

  it("nouvel aperçu : politique et erreur remises à zéro", async () => {
    const api = fakeApi();
    api.applyContactImport.mockRejectedValueOnce(new IpcFailure("storage"));
    const state = new ContactImportState(api);
    await state.choose("t", "f");
    state.policy = "complete";
    await state.confirm();
    await state.choose("t", "f");
    expect(state.policy).toBe("skip");
    expect(state.error).toBeNull();
  });
});
