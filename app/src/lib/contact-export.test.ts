import { describe, expect, it, vi } from "vitest";
import type { ContactExportOutcomeDto } from "./ipc/api";
import { IpcFailure } from "./errors";
import { ContactExportState, type ContactExportApi } from "./contact-export.svelte";

function fakeApi(result: ContactExportOutcomeDto | null | Error) {
  return {
    exportContacts: vi.fn(async () => {
      if (result instanceof Error) throw result;
      return result;
    }),
  } satisfies ContactExportApi;
}

const written = { exported: 3, file_name: "contacts.vcf" };

describe("export de contacts", () => {
  it("s'ouvre sur tout le carnet ou sur une sélection, en vCard 4.0 par défaut", () => {
    const state = new ContactExportState(fakeApi(written));
    expect(state.selection).toBeNull();
    state.open({ kind: "all" });
    expect(state.selection).toEqual({ kind: "all" });
    expect(state.format).toBe("vcard4");
    expect(state.count).toBeNull();
    state.open({ kind: "contacts", ids: [4, 2] });
    expect(state.count).toBe(2);
  });

  it("exporte au format choisi, avec les libellés du sélecteur de ce format", async () => {
    const api = fakeApi(written);
    const state = new ContactExportState(api);
    state.open({ kind: "contacts", ids: [4, 2] });
    state.format = "google_csv";
    const labels = vi.fn((format: string) => ({
      title: "Enregistrer",
      filter: `filtre ${format}`,
    }));
    expect(await state.confirm(labels)).toEqual(written);
    expect(labels).toHaveBeenCalledWith("google_csv");
    expect(api.exportContacts).toHaveBeenCalledWith(
      { kind: "contacts", ids: [4, 2] },
      "google_csv",
      "Enregistrer",
      "filtre google_csv",
    );
    // Écrit : la fenêtre se ferme.
    expect(state.selection).toBeNull();
    expect(state.busy).toBe(false);
  });

  it("enregistrement annulé : la fenêtre reste ouverte, sans erreur", async () => {
    const state = new ContactExportState(fakeApi(null));
    state.open({ kind: "all" });
    expect(await state.confirm(() => ({ title: "t", filter: "f" }))).toBeNull();
    expect(state.selection).toEqual({ kind: "all" });
    expect(state.error).toBeNull();
  });

  it("échec : message traduit, fenêtre gardée pour réessayer", async () => {
    const state = new ContactExportState(fakeApi(new IpcFailure("contact_export_too_large")));
    state.open({ kind: "all" });
    expect(await state.confirm(() => ({ title: "t", filter: "f" }))).toBeNull();
    expect(state.error).toContain("Trop de contacts");
    expect(state.selection).toEqual({ kind: "all" });
    expect(state.busy).toBe(false);
  });

  it("le format choisi reste celui de la fois précédente ; fermer oublie la sélection", () => {
    const state = new ContactExportState(fakeApi(written));
    state.open({ kind: "all" });
    state.format = "ldif";
    state.error = "ancienne erreur";
    state.close();
    expect(state.selection).toBeNull();
    state.open({ kind: "all" });
    expect(state.format).toBe("ldif");
    expect(state.error).toBeNull();
  });

  it("rien à exporter sans sélection", async () => {
    const api = fakeApi(written);
    const state = new ContactExportState(api);
    expect(await state.confirm(() => ({ title: "t", filter: "f" }))).toBeNull();
    expect(api.exportContacts).not.toHaveBeenCalled();
  });
});
