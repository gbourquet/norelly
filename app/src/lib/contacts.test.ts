import { describe, expect, it, vi } from "vitest";
import type { ContactCursorDto, ContactDto, ContactFieldsDto, ContactItem } from "./ipc/api";
import { IpcFailure } from "./errors";
import {
  CONTACT_PAGE,
  ContactFormState,
  ContactsState,
  DeleteContactsState,
  emptyFields,
  type ContactsApi,
} from "./contacts.svelte";

/** Élément `i` d'une liste (le test échoue s'il manque). */
function at<T>(list: T[], i: number): T {
  const value = list[i];
  if (value === undefined) throw new Error(`élément ${i} absent`);
  return value;
}

function item(id: number, name: string, favorite = false): ContactItem {
  return { id, name, address: `${name.toLowerCase()}@example.com`, organization: null, favorite };
}

function contact(id: number, name: string, favorite = false): ContactDto {
  return {
    id,
    name,
    fields: {
      ...emptyFields(),
      display_name: name,
      emails: [{ address: `${name.toLowerCase()}@example.com`, label: "", preferred: true }],
      favorite,
    },
  };
}

/** Carnet factice : `pages` renvoyées dans l'ordre des appels. */
function fakeApi(book: ContactDto[] = []) {
  const all = () =>
    book
      .map((c) => ({
        ...item(c.id, c.name, c.fields.favorite),
        address: c.fields.emails[0]?.address ?? "",
      }))
      .sort((a, b) => a.id - b.id);
  const api = {
    listContacts: vi.fn(
      async (
        text: string,
        favoritesOnly: boolean,
        cursor: ContactCursorDto | null,
        limit: number,
      ) => {
        const rows = all()
          .filter((i) => i.name.toLowerCase().includes(text.toLowerCase()))
          .filter((i) => !favoritesOnly || i.favorite)
          .filter((i) => cursor === null || i.id > cursor.id);
        const items = rows.slice(0, limit);
        const last = items.at(-1);
        const next =
          rows.length > limit && last !== undefined ? { sort_key: last.name, id: last.id } : null;
        return { items, next };
      },
    ),
    getContact: vi.fn(async (id: number) => {
      const found = book.find((c) => c.id === id);
      if (found === undefined) throw new IpcFailure("contact_not_found");
      return structuredClone(found);
    }),
    createContact: vi.fn(async (fields: ContactFieldsDto) => {
      const created = { id: 100 + book.length, name: fields.display_name, fields };
      book.push(created);
      return created;
    }),
    updateContact: vi.fn(async (id: number, fields: ContactFieldsDto) => {
      const index = book.findIndex((c) => c.id === id);
      if (index < 0) throw new IpcFailure("contact_not_found");
      const updated = { id, name: fields.display_name || (book[index]?.name ?? ""), fields };
      book[index] = updated;
      return updated;
    }),
    deleteContacts: vi.fn(async (ids: number[]) => {
      const before = book.length;
      book.splice(0, book.length, ...book.filter((c) => !ids.includes(c.id)));
      return before - book.length;
    }),
  } satisfies ContactsApi;
  return api;
}

describe("liste des contacts", () => {
  it("charge la première page, puis la suite", async () => {
    const book = Array.from({ length: CONTACT_PAGE + 5 }, (_, i) => contact(i + 1, `C${i + 1}`));
    const api = fakeApi(book);
    const s = new ContactsState(api);
    await s.load();
    expect(s.items).toHaveLength(CONTACT_PAGE);
    expect(s.hasMore).toBe(true);
    await s.loadMore();
    expect(s.items).toHaveLength(CONTACT_PAGE + 5);
    expect(s.hasMore).toBe(false);
    // Plus rien à charger : pas d'appel inutile.
    await s.loadMore();
    expect(api.listContacts).toHaveBeenCalledTimes(2);
  });

  it("filtre sur le texte saisi et sur les favoris", async () => {
    const api = fakeApi([contact(1, "Alice", true), contact(2, "Bob"), contact(3, "Albert")]);
    const s = new ContactsState(api);
    await s.search("al");
    expect(s.items.map((i) => i.id)).toEqual([1, 3]);
    await s.showFavorites(true);
    expect(s.items.map((i) => i.id)).toEqual([1]);
    expect(api.listContacts).toHaveBeenLastCalledWith("al", true, null, CONTACT_PAGE);
  });

  it("ignore une réponse dépassée par une recherche plus récente", async () => {
    const api = fakeApi([contact(1, "Alice"), contact(2, "Bob")]);
    let release: () => void = () => undefined;
    const slow = new Promise<void>((resolve) => (release = resolve));
    const original = api.listContacts.getMockImplementation();
    api.listContacts.mockImplementationOnce(async (...args) => {
      await slow;
      return original ? original(...args) : { items: [], next: null };
    });
    const s = new ContactsState(api);
    const first = s.search("a");
    await s.search("bob");
    release();
    await first;
    expect(s.text).toBe("bob");
    expect(s.items.map((i) => i.id)).toEqual([2]);
  });

  it("affiche l'erreur de chargement", async () => {
    const api = fakeApi();
    api.listContacts.mockRejectedValueOnce(new IpcFailure("storage"));
    const s = new ContactsState(api);
    await s.load();
    expect(s.error).toMatch(/stockage|Stockage/);
    await s.load();
    expect(s.error).toBeNull();
  });
});

describe("fiche et sélection", () => {
  it("ouvre la fiche du contact choisi", async () => {
    const s = new ContactsState(fakeApi([contact(1, "Alice")]));
    await s.load();
    await s.select(1);
    expect(s.selectedId).toBe(1);
    expect(s.selected?.name).toBe("Alice");
    await s.select(null);
    expect(s.selected).toBeNull();
  });

  it("une fiche supprimée ailleurs disparaît de la sélection", async () => {
    const s = new ContactsState(fakeApi([contact(1, "Alice")]));
    await s.select(9);
    expect(s.selectedId).toBeNull();
    expect(s.error).toBe("Ce contact n'existe plus.");
  });

  it("vise les contacts cochés, sinon la fiche ouverte", async () => {
    const s = new ContactsState(fakeApi([contact(1, "A"), contact(2, "B"), contact(3, "C")]));
    await s.load();
    expect(s.targets()).toEqual([]);
    await s.select(2);
    expect(s.targets()).toEqual([2]);
    s.toggleMark(3);
    s.toggleMark(1);
    expect(s.targets()).toEqual([3, 1]);
    s.toggleMark(3);
    expect(s.targets()).toEqual([1]);
    s.clearMarks();
    expect(s.targets()).toEqual([2]);
  });

  it("bascule le favori de la fiche ouverte et relit la liste", async () => {
    const api = fakeApi([contact(1, "Alice")]);
    const s = new ContactsState(api);
    await s.load();
    await s.select(1);
    expect(await s.toggleFavorite()).toBe(true);
    expect(s.selected?.fields.favorite).toBe(true);
    expect(s.items[0]?.favorite).toBe(true);
    expect(api.updateContact).toHaveBeenLastCalledWith(
      1,
      expect.objectContaining({ favorite: true, display_name: "Alice" }),
    );
    expect(await s.toggleFavorite()).toBe(false);
  });

  it("après un enregistrement : liste relue, fiche ouverte", async () => {
    const api = fakeApi([contact(1, "Alice")]);
    const s = new ContactsState(api);
    await s.load();
    await s.saved(contact(1, "Alice B."));
    expect(s.selectedId).toBe(1);
    expect(s.selected?.name).toBe("Alice B.");
    expect(api.listContacts).toHaveBeenCalledTimes(2);
  });

  it("après une suppression : fiches retirées de la sélection et des cases", async () => {
    const s = new ContactsState(fakeApi([contact(1, "A"), contact(2, "B")]));
    await s.load();
    await s.select(1);
    s.toggleMark(1);
    s.toggleMark(2);
    await s.removed([1]);
    expect(s.selectedId).toBeNull();
    expect(s.marked).toEqual([2]);
  });
});

describe("écrire à des contacts", () => {
  it("donne l'adresse préférée et le nom de chaque contact visé", async () => {
    const api = fakeApi([contact(1, "Alice"), contact(2, "Bob")]);
    const s = new ContactsState(api);
    await s.search("ali");
    // Bob n'est pas dans la liste affichée : sa fiche est relue.
    expect(await s.recipients([1, 2])).toEqual([
      { name: "Alice", address: "alice@example.com" },
      { name: "Bob", address: "bob@example.com" },
    ]);
    expect(api.getContact).toHaveBeenCalledTimes(1);
  });

  it("sans nom, le destinataire n'a que son adresse", async () => {
    const nameless = contact(3, "x");
    nameless.name = "x@example.com";
    const s = new ContactsState(fakeApi([nameless]));
    expect(await s.recipients([3])).toEqual([{ name: "", address: "x@example.com" }]);
    await s.load();
    expect(await s.recipients([3])).toEqual([{ name: "", address: "x@example.com" }]);
  });

  it("ignore un contact disparu entre-temps", async () => {
    const s = new ContactsState(fakeApi([contact(1, "Alice")]));
    expect(await s.recipients([9, 1])).toEqual([{ name: "Alice", address: "alice@example.com" }]);
  });
});

describe("formulaire d'un contact", () => {
  it("part d'une fiche vide avec une adresse préférée à saisir", () => {
    const f = new ContactFormState(fakeApi());
    f.reset(null);
    expect(f.id).toBeNull();
    expect(f.fields.emails).toEqual([{ address: "", label: "", preferred: true }]);
    expect(f.canSubmit).toBe(false);
    at(f.fields.emails, 0).address = "a@example.com";
    expect(f.canSubmit).toBe(true);
  });

  it("pré-remplit la fiche à modifier, sans la partager", () => {
    const source = contact(4, "Alice");
    const f = new ContactFormState(fakeApi());
    f.reset(source);
    expect(f.id).toBe(4);
    f.fields.display_name = "Autre";
    expect(source.fields.display_name).toBe("Alice");
  });

  it("pré-remplit un nouveau contact avec le nom et l'adresse d'un correspondant", () => {
    const f = new ContactFormState(fakeApi());
    f.reset(contact(4, "Alice"));
    f.prefill({ name: "Bob Martin", address: "bob@example.com" });
    expect(f.id).toBeNull();
    expect(f.fields.display_name).toBe("Bob Martin");
    expect(f.fields.emails).toEqual([{ address: "bob@example.com", label: "", preferred: true }]);
    expect(f.canSubmit).toBe(true);
    f.prefill({ name: "", address: "zoe@example.com" });
    expect(f.fields.display_name).toBe("");
    expect(at(f.fields.emails, 0).address).toBe("zoe@example.com");
  });

  it("ajoute et retire des adresses ; une seule est préférée", () => {
    const f = new ContactFormState(fakeApi());
    f.reset(null);
    f.addEmail();
    f.addEmail();
    expect(f.fields.emails).toHaveLength(3);
    f.setPreferred(2);
    expect(f.fields.emails.map((e) => e.preferred)).toEqual([false, false, true]);
    // Retirer l'adresse préférée : la première le devient.
    f.removeEmail(2);
    expect(f.fields.emails.map((e) => e.preferred)).toEqual([true, false]);
    // La dernière ligne reste.
    f.removeEmail(0);
    f.removeEmail(0);
    expect(f.fields.emails).toHaveLength(1);
    expect(f.fields.emails[0]?.preferred).toBe(true);
  });

  it("borne le nombre de lignes d'adresses et de téléphones", () => {
    const f = new ContactFormState(fakeApi());
    f.reset(null);
    for (let i = 0; i < 30; i += 1) {
      f.addEmail();
      f.addPhone();
    }
    expect(f.fields.emails).toHaveLength(20);
    expect(f.fields.phones).toHaveLength(20);
    expect(f.canAddEmail).toBe(false);
    expect(f.canAddPhone).toBe(false);
    f.removePhone(0);
    expect(f.canAddPhone).toBe(true);
  });

  it("n'envoie pas les lignes laissées vides", async () => {
    const api = fakeApi();
    const f = new ContactFormState(api);
    f.reset(null);
    f.addEmail();
    f.addPhone();
    f.addPhone();
    at(f.fields.emails, 1).address = " b@example.com ";
    f.setPreferred(1);
    at(f.fields.phones, 1).number = "01 02";
    f.fields.given_name = "Bob";
    const saved = await f.submit();
    expect(saved?.id).toBe(100);
    expect(api.createContact).toHaveBeenCalledWith(
      expect.objectContaining({
        given_name: "Bob",
        emails: [{ address: " b@example.com ", label: "", preferred: true }],
        phones: [{ number: "01 02", label: "" }],
      }),
    );
  });

  it("modifie la fiche existante", async () => {
    const api = fakeApi([contact(4, "Alice")]);
    const f = new ContactFormState(api);
    f.reset(contact(4, "Alice"));
    f.fields.nickname = "Ali";
    await f.submit();
    expect(api.updateContact).toHaveBeenCalledWith(4, expect.objectContaining({ nickname: "Ali" }));
    expect(api.createContact).not.toHaveBeenCalled();
  });

  it("affiche le refus de Rust et garde la saisie", async () => {
    const api = fakeApi();
    api.createContact.mockRejectedValueOnce(
      new IpcFailure("contact_duplicate_address", null, "a@example.com"),
    );
    const f = new ContactFormState(api);
    f.reset(null);
    at(f.fields.emails, 0).address = "a@example.com";
    expect(await f.submit()).toBeNull();
    expect(f.error).toBe("Cette adresse appartient déjà à un autre contact. (« a@example.com »)");
    expect(f.fields.emails[0]?.address).toBe("a@example.com");
    expect(f.busy).toBe(false);
    f.reset(null);
    expect(f.error).toBeNull();
  });
});

describe("suppression de contacts", () => {
  it("supprime les fiches visées et renvoie leur nombre", async () => {
    const api = fakeApi([contact(1, "A"), contact(2, "B")]);
    const d = new DeleteContactsState(api);
    expect(await d.submit([1, 2])).toBe(2);
    expect(api.deleteContacts).toHaveBeenCalledWith([1, 2]);
  });

  it("affiche l'échec", async () => {
    const api = fakeApi();
    api.deleteContacts.mockRejectedValueOnce(new IpcFailure("storage"));
    const d = new DeleteContactsState(api);
    expect(await d.submit([1])).toBeNull();
    expect(d.error).not.toBeNull();
    d.reset();
    expect(d.error).toBeNull();
  });
});
