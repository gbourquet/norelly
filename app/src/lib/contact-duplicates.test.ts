import { describe, expect, it, vi } from "vitest";
import type { ContactDuplicateGroupDto, ContactMergePreviewDto, DuplicateCardDto } from "./ipc/api";
import { IpcFailure } from "./errors";
import { ContactDuplicatesState, type ContactDuplicatesApi } from "./contact-duplicates.svelte";

function card(id: number, name: string): DuplicateCardDto {
  return {
    id,
    name,
    emails: [{ address: `c${id}@example.com`, label: "", preferred: true }],
    phones: [],
    organization: "",
  };
}

const alices: ContactDuplicateGroupDto = {
  contacts: [card(3, "Alice"), card(1, "alice"), card(7, "Alice M.")],
  reasons: ["same_name", "same_phone"],
  truncated: false,
};
const bobs: ContactDuplicateGroupDto = {
  contacts: [card(4, "Bob"), card(5, "bob")],
  reasons: ["same_name"],
  truncated: false,
};

/** Aperçu factice : le nom de la fiche gardée suivi des fiches réunies. */
function previewOf(keep: number, others: number[]): ContactMergePreviewDto {
  return { name: `fiche ${keep} + ${others.join(",")}`, preferred_address: `c${keep}@example.com` };
}

function fakeApi(
  groups: ContactDuplicateGroupDto[] | Error,
  options: { merge?: Error; preview?: Error } = {},
) {
  return {
    findContactDuplicates: vi.fn(async () => {
      if (groups instanceof Error) throw groups;
      return groups;
    }),
    previewContactMerge: vi.fn(async (keep: number, others: number[]) => {
      if (options.preview) throw options.preview;
      return previewOf(keep, others);
    }),
    mergeContacts: vi.fn(async (keep: number) => {
      if (options.merge) throw options.merge;
      return {
        id: keep,
        name: "fusionné",
        fields: {
          display_name: "fusionné",
          given_name: "",
          family_name: "",
          nickname: "",
          organization: "",
          notes: "",
          emails: [{ address: `c${keep}@example.com`, label: "", preferred: true }],
          phones: [],
          favorite: false,
        },
      };
    }),
  } satisfies ContactDuplicatesApi;
}

describe("doublons du carnet", () => {
  it("cherche les doublons et présente le premier groupe, tout coché, la première fiche gardée", async () => {
    const api = fakeApi([alices, bobs]);
    const state = new ContactDuplicatesState(api);
    const loading = state.open();
    expect(state.loading).toBe(true);
    await loading;
    expect(state.loading).toBe(false);
    expect(state.current).toEqual(alices);
    expect([state.position, state.total]).toEqual([1, 2]);
    expect(state.keep).toBe(3);
    expect(state.isChecked(1) && state.isChecked(3) && state.isChecked(7)).toBe(true);
    // Aperçu calculé par Rust pour la sélection présentée.
    expect(api.previewContactMerge).toHaveBeenLastCalledWith(3, [1, 7]);
    expect(state.preview).toEqual(previewOf(3, [1, 7]));
    expect(state.canMerge).toBe(true);
    expect(state.finished).toBe(false);
  });

  it("sans doublon : rien à présenter, fini d'emblée", async () => {
    const api = fakeApi([]);
    const state = new ContactDuplicatesState(api);
    await state.open();
    expect(state.current).toBeNull();
    expect(state.total).toBe(0);
    expect(state.finished).toBe(true);
    expect(api.previewContactMerge).not.toHaveBeenCalled();
  });

  it("échec de la recherche : message traduit", async () => {
    const state = new ContactDuplicatesState(
      fakeApi(new IpcFailure("contact_duplicates_too_many")),
    );
    await state.open();
    expect(state.error).not.toBeNull();
    expect(state.loading).toBe(false);
    expect(state.current).toBeNull();
  });

  it("la fiche gardée reste cochée ; il faut au moins une autre fiche pour fusionner", async () => {
    const api = fakeApi([bobs]);
    const state = new ContactDuplicatesState(api);
    await state.open();
    await state.toggle(4);
    expect(state.isChecked(4)).toBe(true);
    await state.toggle(5);
    expect(state.isChecked(5)).toBe(false);
    expect(state.preview).toBeNull();
    expect(state.canMerge).toBe(false);
    expect(api.previewContactMerge).toHaveBeenCalledTimes(1);
    // Garder une fiche décochée la recoche ; l'aperçu suit.
    await state.choose(5);
    expect(state.keep).toBe(5);
    expect(state.isChecked(5)).toBe(true);
    expect(state.preview).toEqual(previewOf(5, [4]));
    expect(state.canMerge).toBe(true);
  });

  it("aperçu refusé (fusion trop grosse) : message, fusion impossible", async () => {
    const state = new ContactDuplicatesState(
      fakeApi([alices], { preview: new IpcFailure("contact_merge_too_large") }),
    );
    await state.open();
    expect(state.preview).toBeNull();
    expect(state.previewError).not.toBeNull();
    expect(state.canMerge).toBe(false);
    expect(await state.merge()).toBeNull();
  });

  it("une réponse d'aperçu périmée est ignorée", async () => {
    const api = fakeApi([alices]);
    const state = new ContactDuplicatesState(api);
    await state.open();
    let release: (v: ContactMergePreviewDto) => void = () => {};
    api.previewContactMerge.mockImplementationOnce(
      () => new Promise<ContactMergePreviewDto>((resolve) => (release = resolve)),
    );
    const slow = state.toggle(7);
    await state.toggle(1);
    release(previewOf(3, [1]));
    await slow;
    // Seule compte la dernière sélection : 3 sans 1 ni 7.
    expect(state.preview).toBeNull();
    expect(state.canMerge).toBe(false);
  });

  it("fusionne les fiches cochées dans la fiche gardée, dans l'ordre du groupe, puis passe au suivant", async () => {
    const api = fakeApi([alices, bobs]);
    const state = new ContactDuplicatesState(api);
    await state.open();
    await state.choose(1);
    await state.toggle(7);
    const merged = await state.merge();
    expect(api.mergeContacts).toHaveBeenCalledWith(1, [3]);
    expect(merged?.id).toBe(1);
    expect(state.merged).toBe(1);
    expect(state.current).toEqual(bobs);
    expect([state.position, state.total]).toEqual([1, 1]);
    expect(state.keep).toBe(4);
    expect(state.isChecked(5)).toBe(true);
    expect(state.preview).toEqual(previewOf(4, [5]));
    await state.merge();
    expect(state.merged).toBe(2);
    expect(state.finished).toBe(true);
    expect(state.current).toBeNull();
  });

  it("échec de la fusion : message traduit, groupe gardé pour réessayer", async () => {
    const state = new ContactDuplicatesState(
      fakeApi([alices], { merge: new IpcFailure("contact_merge_conflict") }),
    );
    await state.open();
    expect(await state.merge()).toBeNull();
    expect(state.error).not.toBeNull();
    expect(state.current).toEqual(alices);
    expect(state.merged).toBe(0);
    expect(state.busy).toBe(false);
  });

  it("passer un groupe présente le suivant sans rien écrire", async () => {
    const api = fakeApi([alices, bobs]);
    const state = new ContactDuplicatesState(api);
    await state.open();
    await state.skip();
    expect(state.current).toEqual(bobs);
    expect([state.position, state.total]).toEqual([2, 2]);
    expect(state.keep).toBe(4);
    expect(state.preview).toEqual(previewOf(4, [5]));
    await state.skip();
    expect(state.finished).toBe(true);
    expect(api.mergeContacts).not.toHaveBeenCalled();
  });

  it("rouvrir relance la recherche et remet le bilan à zéro", async () => {
    const api = fakeApi([bobs]);
    const state = new ContactDuplicatesState(api);
    await state.open();
    await state.merge();
    state.close();
    expect(state.total).toBe(0);
    expect(state.preview).toBeNull();
    await state.open();
    expect(api.findContactDuplicates).toHaveBeenCalledTimes(2);
    expect(state.merged).toBe(0);
    expect(state.current).toEqual(bobs);
  });
});
