import { describe, expect, it, vi } from "vitest";
import type { ContactDto, ContactDuplicateGroupDto } from "./ipc/api";
import { IpcFailure } from "./errors";
import { ContactDuplicatesState, type ContactDuplicatesApi } from "./contact-duplicates.svelte";

function contact(id: number, name: string): ContactDto {
  return {
    id,
    name,
    fields: {
      display_name: name,
      given_name: "",
      family_name: "",
      nickname: "",
      organization: "",
      notes: "",
      emails: [{ address: `c${id}@example.com`, label: "", preferred: true }],
      phones: [],
      favorite: false,
    },
  };
}

const alices: ContactDuplicateGroupDto = {
  contacts: [contact(3, "Alice"), contact(1, "alice"), contact(7, "Alice M.")],
  reasons: ["same_name", "same_phone"],
};
const bobs: ContactDuplicateGroupDto = {
  contacts: [contact(4, "Bob"), contact(5, "bob")],
  reasons: ["same_name"],
};

function fakeApi(groups: ContactDuplicateGroupDto[] | Error, merge?: Error) {
  return {
    findContactDuplicates: vi.fn(async () => {
      if (groups instanceof Error) throw groups;
      return groups;
    }),
    mergeContacts: vi.fn(async (keep: number) => {
      if (merge) throw merge;
      return contact(keep, "fusionné");
    }),
  } satisfies ContactDuplicatesApi;
}

describe("doublons du carnet", () => {
  it("cherche les doublons et présente le premier groupe, tout coché, la première fiche gardée", async () => {
    const state = new ContactDuplicatesState(fakeApi([alices, bobs]));
    const loading = state.open();
    expect(state.loading).toBe(true);
    await loading;
    expect(state.loading).toBe(false);
    expect(state.current).toEqual(alices);
    expect([state.position, state.total]).toEqual([1, 2]);
    expect(state.keep).toBe(3);
    expect(state.isChecked(1) && state.isChecked(3) && state.isChecked(7)).toBe(true);
    expect(state.canMerge).toBe(true);
    expect(state.finished).toBe(false);
  });

  it("sans doublon : rien à présenter, fini d'emblée", async () => {
    const state = new ContactDuplicatesState(fakeApi([]));
    await state.open();
    expect(state.current).toBeNull();
    expect(state.total).toBe(0);
    expect(state.finished).toBe(true);
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
    const state = new ContactDuplicatesState(fakeApi([bobs]));
    await state.open();
    state.toggle(4);
    expect(state.isChecked(4)).toBe(true);
    state.toggle(5);
    expect(state.isChecked(5)).toBe(false);
    expect(state.canMerge).toBe(false);
    // Garder une fiche décochée la recoche.
    state.choose(5);
    expect(state.keep).toBe(5);
    expect(state.isChecked(5)).toBe(true);
    expect(state.canMerge).toBe(true);
  });

  it("fusionne les fiches cochées dans la fiche gardée, dans l'ordre du groupe, puis passe au suivant", async () => {
    const api = fakeApi([alices, bobs]);
    const state = new ContactDuplicatesState(api);
    await state.open();
    state.choose(1);
    state.toggle(7);
    const merged = await state.merge();
    expect(api.mergeContacts).toHaveBeenCalledWith(1, [3]);
    expect(merged?.id).toBe(1);
    expect(state.merged).toBe(1);
    expect(state.current).toEqual(bobs);
    expect([state.position, state.total]).toEqual([1, 1]);
    expect(state.keep).toBe(4);
    expect(state.isChecked(5)).toBe(true);
    await state.merge();
    expect(state.merged).toBe(2);
    expect(state.finished).toBe(true);
    expect(state.current).toBeNull();
  });

  it("échec de la fusion : message traduit, groupe gardé pour réessayer", async () => {
    const state = new ContactDuplicatesState(
      fakeApi([alices], new IpcFailure("contact_merge_too_large")),
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
    state.skip();
    expect(state.current).toEqual(bobs);
    expect([state.position, state.total]).toEqual([2, 2]);
    expect(state.keep).toBe(4);
    state.skip();
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
    await state.open();
    expect(api.findContactDuplicates).toHaveBeenCalledTimes(2);
    expect(state.merged).toBe(0);
    expect(state.current).toEqual(bobs);
  });
});
