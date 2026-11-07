import { describe, expect, it, vi } from "vitest";
import type {
  ContactItem,
  ContactList,
  ContactListCursorDto,
  ContactListFieldsDto,
  ContactListItemDto,
} from "./ipc/api";
import { IpcFailure } from "./errors";
import {
  AddToListState,
  ContactListFormState,
  ContactListsState,
  DeleteContactListState,
  LIST_PAGE,
  MAX_LIST_MEMBERS,
  emptyListFields,
  type ContactListsApi,
} from "./contact-lists.svelte";

function person(id: number, name: string): ContactItem {
  return {
    id,
    name,
    address: `${name.toLowerCase()}@example.com`,
    organization: null,
    favorite: false,
  };
}

const people = [person(1, "Ana"), person(2, "Bo"), person(3, "Cy")];

function list(id: number, name: string, members: number[] = [], bcc = false): ContactList {
  return {
    id,
    fields: { name, description: "", members, bcc },
    members: members.map((m) => people.find((p) => p.id === m) ?? person(m, `n${m}`)),
  };
}

/** Carnet factice : listes triées par identifiant. */
function fakeApi(lists: ContactList[] = []) {
  const row = (l: ContactList): ContactListItemDto => ({
    id: l.id,
    name: l.fields.name,
    members: l.fields.members.length,
    bcc: l.fields.bcc,
  });
  const api = {
    listContactLists: vi.fn(
      async (text: string, cursor: ContactListCursorDto | null, limit: number) => {
        const rows = lists
          .map(row)
          .filter((l) => l.name.toLowerCase().includes(text.toLowerCase()))
          .filter((l) => cursor === null || l.id > cursor.id);
        const items = rows.slice(0, limit);
        const last = items.at(-1);
        const next =
          rows.length > limit && last !== undefined ? { sort_key: last.name, id: last.id } : null;
        return { items, next };
      },
    ),
    getContactList: vi.fn(async (id: number) => {
      const found = lists.find((l) => l.id === id);
      if (found === undefined) throw new IpcFailure("contact_list_not_found");
      return structuredClone(found);
    }),
    createContactList: vi.fn(async (fields: ContactListFieldsDto) => {
      if (lists.some((l) => l.fields.name.toLowerCase() === fields.name.trim().toLowerCase())) {
        throw new IpcFailure("contact_list_duplicate_name", "list_name", fields.name);
      }
      const created = list(100 + lists.length, fields.name.trim(), fields.members, fields.bcc);
      lists.push(created);
      return created;
    }),
    updateContactList: vi.fn(async (id: number, fields: ContactListFieldsDto) => {
      const index = lists.findIndex((l) => l.id === id);
      if (index < 0) throw new IpcFailure("contact_list_not_found");
      const updated = list(id, fields.name, fields.members, fields.bcc);
      lists[index] = updated;
      return updated;
    }),
    deleteContactList: vi.fn(async (id: number) => {
      const index = lists.findIndex((l) => l.id === id);
      if (index >= 0) lists.splice(index, 1);
      return index >= 0;
    }),
    addToContactList: vi.fn(async (id: number, contacts: number[]) => {
      const found = lists.find((l) => l.id === id);
      if (found === undefined) throw new IpcFailure("contact_list_not_found");
      const members = [...found.fields.members];
      for (const c of contacts) if (!members.includes(c)) members.push(c);
      const updated = list(id, found.fields.name, members, found.fields.bcc);
      lists[lists.indexOf(found)] = updated;
      return updated;
    }),
    listContacts: vi.fn(async (text: string) => ({
      items: people.filter((p) => p.name.toLowerCase().includes(text.toLowerCase())),
      next: null,
    })),
  } satisfies ContactListsApi;
  return api;
}

describe("listes de diffusion affichées", () => {
  it("ne lit les listes qu'une fois la section ouverte, puis les filtre sur le nom", async () => {
    const api = fakeApi([list(1, "Famille", [1]), list(2, "Club", [])]);
    const lists = new ContactListsState(api);
    expect(lists.shown).toBe(false);
    await lists.show(true);
    expect(lists.shown).toBe(true);
    expect(api.listContactLists).toHaveBeenCalledWith("", null, LIST_PAGE);
    expect(lists.items.map((l) => l.name)).toEqual(["Famille", "Club"]);
    await lists.search("fam");
    expect(lists.items.map((l) => l.name)).toEqual(["Famille"]);
    await lists.show(false);
    expect(lists.shown).toBe(false);
  });

  it("charge la page suivante à la suite", async () => {
    const many = Array.from({ length: LIST_PAGE + 3 }, (_, i) => list(i + 1, `L${i + 1}`));
    const api = fakeApi(many);
    const lists = new ContactListsState(api);
    await lists.show(true);
    expect(lists.items).toHaveLength(LIST_PAGE);
    expect(lists.hasMore).toBe(true);
    await lists.loadMore();
    expect(lists.items).toHaveLength(LIST_PAGE + 3);
    expect(lists.hasMore).toBe(false);
  });

  it("ouvre une liste et l'oublie si elle a disparu", async () => {
    const api = fakeApi([list(1, "Famille", [2, 1])]);
    const lists = new ContactListsState(api);
    await lists.select(1);
    expect(lists.selected?.members.map((m) => m.name)).toEqual(["Bo", "Ana"]);
    await lists.select(9);
    expect(lists.selectedId).toBeNull();
    expect(lists.selected).toBeNull();
    expect(lists.error).toBe("Cette liste n'existe plus.");
  });

  it("après un enregistrement ou une suppression, relit la liste", async () => {
    const api = fakeApi([list(1, "Famille")]);
    const lists = new ContactListsState(api);
    await lists.show(true);
    const saved = list(1, "Famille proche", [1]);
    await lists.saved(saved);
    expect(lists.selected).toEqual(saved);
    expect(lists.selectedId).toBe(1);
    expect(api.listContactLists).toHaveBeenCalledTimes(2);
    await lists.removed(1);
    expect(lists.selected).toBeNull();
    expect(lists.selectedId).toBeNull();
  });
});

describe("formulaire d'une liste", () => {
  it("part vide pour une nouvelle liste", () => {
    const form = new ContactListFormState(fakeApi());
    form.reset(null);
    expect(form.id).toBeNull();
    expect(form.fields).toEqual(emptyListFields());
    expect(form.members).toEqual([]);
    expect(form.canSubmit).toBe(false);
    form.fields.name = "  ";
    expect(form.canSubmit).toBe(false);
    form.fields.name = "Famille";
    expect(form.canSubmit).toBe(true);
  });

  it("reprend une liste existante avec ses membres", () => {
    const form = new ContactListFormState(fakeApi());
    const existing = list(4, "Club", [2, 1], true);
    form.reset(existing);
    expect(form.id).toBe(4);
    expect(form.fields.bcc).toBe(true);
    expect(form.members.map((m) => m.id)).toEqual([2, 1]);
    form.fields.name = "Autre";
    expect(existing.fields.name).toBe("Club");
  });

  it("cherche des contacts à ajouter, chacun une fois, et les retire", async () => {
    const api = fakeApi();
    const form = new ContactListFormState(api);
    form.reset(null);
    await form.find("a");
    expect(form.candidates.map((c) => c.name)).toEqual(["Ana"]);
    form.add(person(1, "Ana"));
    form.add(person(1, "Ana"));
    form.add(person(2, "Bo"));
    expect(form.members.map((m) => m.id)).toEqual([1, 2]);
    expect(form.isMember(1)).toBe(true);
    form.remove(1);
    expect(form.members.map((m) => m.id)).toEqual([2]);
    expect(form.isMember(1)).toBe(false);
    await form.find("  ");
    expect(form.candidates).toEqual([]);
  });

  it("ignore une recherche dépassée", async () => {
    const api = fakeApi();
    let release: () => void = () => undefined;
    api.listContacts.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = () => resolve({ items: [person(9, "Vieux")], next: null });
        }),
    );
    const form = new ContactListFormState(api);
    form.reset(null);
    const slow = form.find("v");
    await form.find("bo");
    release();
    await slow;
    expect(form.candidates.map((c) => c.name)).toEqual(["Bo"]);
  });

  it("borne le nombre de membres", () => {
    const form = new ContactListFormState(fakeApi());
    form.reset(null);
    for (let i = 1; i <= MAX_LIST_MEMBERS + 2; i++) form.add(person(i, `P${i}`));
    expect(form.members).toHaveLength(MAX_LIST_MEMBERS);
    expect(form.canAdd).toBe(false);
  });

  it("enregistre une nouvelle liste avec ses membres dans l'ordre", async () => {
    const api = fakeApi();
    const form = new ContactListFormState(api);
    form.reset(null);
    form.fields.name = "Famille";
    form.fields.bcc = true;
    form.add(person(2, "Bo"));
    form.add(person(1, "Ana"));
    const saved = await form.submit();
    expect(api.createContactList).toHaveBeenCalledWith({
      name: "Famille",
      description: "",
      members: [2, 1],
      bcc: true,
    });
    expect(saved?.fields.name).toBe("Famille");
  });

  it("modifie une liste existante", async () => {
    const api = fakeApi([list(1, "Famille", [1])]);
    const form = new ContactListFormState(api);
    form.reset(list(1, "Famille", [1]));
    form.remove(1);
    form.add(person(3, "Cy"));
    await form.submit();
    expect(api.updateContactList).toHaveBeenCalledWith(1, {
      name: "Famille",
      description: "",
      members: [3],
      bcc: false,
    });
  });

  it("garde la saisie et affiche le refus de Rust", async () => {
    const api = fakeApi([list(1, "Famille")]);
    const form = new ContactListFormState(api);
    form.reset(null);
    form.fields.name = "famille";
    expect(await form.submit()).toBeNull();
    expect(form.error).toBe("Une autre liste porte déjà ce nom. (« famille »)");
    expect(form.fields.name).toBe("famille");
    expect(form.busy).toBe(false);
  });
});

describe("ajout de contacts à une liste", () => {
  it("propose les listes, la première choisie, puis ajoute les contacts", async () => {
    const api = fakeApi([list(1, "Club", [1]), list(2, "Famille")]);
    const add = new AddToListState(api);
    await add.open();
    expect(add.lists.map((l) => l.name)).toEqual(["Club", "Famille"]);
    expect(add.chosen).toBe(1);
    add.chosen = 2;
    const updated = await add.submit([3, 1]);
    expect(api.addToContactList).toHaveBeenCalledWith(2, [3, 1]);
    expect(updated?.fields.members).toEqual([3, 1]);
  });

  it("sans liste, rien n'est choisi ni envoyé", async () => {
    const api = fakeApi();
    const add = new AddToListState(api);
    await add.open();
    expect(add.chosen).toBeNull();
    expect(await add.submit([1])).toBeNull();
    expect(api.addToContactList).not.toHaveBeenCalled();
  });

  it("affiche le refus de Rust", async () => {
    const api = fakeApi([list(1, "Club")]);
    api.addToContactList.mockRejectedValueOnce(new IpcFailure("contact_list_too_many_members"));
    const add = new AddToListState(api);
    await add.open();
    expect(await add.submit([1])).toBeNull();
    expect(add.error).toBe("Une liste compte au plus 100 membres.");
  });
});

describe("suppression d'une liste", () => {
  it("supprime, ou affiche le refus", async () => {
    const api = fakeApi([list(1, "Club")]);
    const del = new DeleteContactListState(api);
    del.reset();
    expect(await del.submit(1)).toBe(true);
    api.deleteContactList.mockRejectedValueOnce(new IpcFailure("storage"));
    expect(await del.submit(1)).toBeNull();
    expect(del.error).not.toBeNull();
    expect(del.busy).toBe(false);
  });
});
