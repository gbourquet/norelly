// Listes de diffusion dans l'application : leurs commandes suivent la
// section « Listes » de la vue Contacts et la liste ouverte (ou celle dont
// le menu est ouvert) ; « Ajouter à une liste… » vise les contacts.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./ipc/api", () => ({
  IpcFailure: class extends Error {},
  api: {
    listConversations: vi.fn(async () => ({ items: [], next: null })),
    listContacts: vi.fn(async () => ({ items: [], next: null })),
    getContact: vi.fn(async (id: number) => ({
      id,
      name: "Alice",
      fields: {
        display_name: "Alice",
        given_name: "",
        family_name: "",
        nickname: "",
        organization: "",
        notes: "",
        emails: [{ address: "alice@example.com", label: "", preferred: true }],
        phones: [],
        favorite: false,
      },
    })),
    listContactLists: vi.fn(async () => ({
      items: [{ id: 7, name: "Famille", members: 1, bcc: false }],
      next: null,
    })),
    getContactList: vi.fn(async (id: number) => ({
      id,
      fields: { name: "Famille", description: "", members: [4], bcc: false },
      members: [],
    })),
    prepareCompose: vi.fn(async (account: number) => ({
      id: null,
      account,
      to: [],
      cc: [],
      bcc: [],
      subject: "",
      html: "",
      in_reply_to: null,
      references: [],
      attachments: [],
    })),
    expandContactList: vi.fn(async () => ({
      recipients: [{ name: "Alice", address: "alice@example.com" }],
      bcc: false,
    })),
    parseRecipient: vi.fn(async (raw: string) => ({
      name: raw.includes("<") ? raw.split("<")[0]?.trim() : "",
      address: raw.includes("<") ? (raw.split("<")[1]?.replace(">", "") ?? "") : raw,
    })),
  },
}));

const { addToList, app, compose, contacts, listForm, lists, registerAppCommands, registry } =
  await import("./app.svelte");

function enabled(id: string): boolean {
  const command = registry.get(id);
  if (command === undefined) throw new Error(`commande ${id} absente`);
  return registry.isEnabled(command, app.context());
}

const listCommands = ["lists.edit", "lists.delete", "lists.write"];

describe("listes de diffusion", () => {
  let off: () => void = () => undefined;

  beforeEach(() => {
    off = registerAppCommands();
    app.accounts = [
      { id: 1, display_name: "Moi", address: "moi@example.com" } as (typeof app.accounts)[number],
    ];
    app.selectedAccountId = 1;
  });

  afterEach(async () => {
    off();
    app.close();
    await lists.show(false);
    await lists.select(null);
    await contacts.select(null);
    contacts.clearMarks();
    app.showMail();
  });

  it("propose les commandes d'une liste quand la section est ouverte et une liste visée", async () => {
    await app.showContacts();
    expect(enabled("lists.new")).toBe(true);
    for (const id of listCommands) expect(enabled(id), id).toBe(false);
    await lists.show(true);
    for (const id of listCommands) expect(enabled(id), id).toBe(false);
    await lists.select(7);
    for (const id of listCommands) expect(enabled(id), id).toBe(true);
    // Menu d'une ligne : la liste pointée, même sans liste ouverte.
    await lists.select(null);
    const edit = registry.get("lists.edit");
    if (edit === undefined) throw new Error("commande lists.edit absente");
    expect(registry.isEnabled(edit, app.contextForList(7))).toBe(true);
  });

  it("dans la section Listes, les commandes des contacts ne visent rien", async () => {
    await app.showContacts();
    await contacts.select(4);
    expect(enabled("contacts.edit")).toBe(true);
    expect(enabled("contacts.add-to-list")).toBe(true);
    await lists.show(true);
    for (const id of ["contacts.edit", "contacts.delete", "contacts.write", "contacts.add-to-list"])
      expect(enabled(id), id).toBe(false);
  });

  it("« Nouvelle liste » ouvre la section et un formulaire vide", async () => {
    await registry.execute("lists.new", app.context());
    expect(app.view).toBe("contacts");
    expect(lists.shown).toBe(true);
    expect(app.dialog).toBe("contact-list");
    expect(listForm.id).toBeNull();
  });

  it("« Modifier la liste » reprend la liste visée", async () => {
    await app.showContacts();
    await lists.show(true);
    await lists.select(7);
    await registry.execute("lists.edit", app.context());
    expect(app.dialog).toBe("contact-list");
    expect(listForm.id).toBe(7);
    expect(listForm.fields.name).toBe("Famille");
  });

  it("« Supprimer la liste » demande confirmation pour la liste visée", async () => {
    await app.showContacts();
    await lists.show(true);
    app.listTarget = 7;
    await registry.execute("lists.delete", app.contextForList(7));
    expect(app.listTarget).toBeNull();
    expect(app.dialog).toBe("delete-contact-list");
    expect(app.listToDelete).toBe(7);
  });

  it("« Écrire à la liste » ouvre la rédaction avec ses membres", async () => {
    await app.showContacts();
    await lists.show(true);
    await lists.select(7);
    await registry.execute("lists.write", app.context());
    expect(app.dialog).toBe("compose");
    expect(compose.draft?.to).toEqual([{ name: "Alice", address: "alice@example.com" }]);
  });

  it("« Ajouter à une liste… » vise les contacts cochés ou la fiche ouverte", async () => {
    await app.showContacts();
    contacts.toggleMark(4);
    contacts.toggleMark(5);
    await registry.execute("contacts.add-to-list", app.context());
    expect(app.dialog).toBe("add-to-list");
    expect(app.contactsToAdd).toEqual([4, 5]);
    expect(addToList.chosen).toBe(7);
  });
});
