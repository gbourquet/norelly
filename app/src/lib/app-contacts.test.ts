// Vue Contacts dans l'application : les commandes du courrier ne visent
// rien tant qu'elle est affichée ; celles des contacts suivent la fiche
// ouverte, les contacts cochés et le contact dont le menu est ouvert.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConversationDto } from "./ipc/api";

vi.mock("./ipc/api", () => ({
  IpcFailure: class extends Error {},
  api: {
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
  },
}));

const { app, contacts, mail, registerAppCommands, registry, search } = await import("./app.svelte");

const conversation: ConversationDto = {
  id: 1,
  account: 1,
  mailbox: 1,
  date: 0,
  subject: "Bonjour",
  senders: [],
  messages: 1,
  unread: 0,
  flagged: false,
  has_attachments: false,
};

function enabled(id: string): boolean {
  const command = registry.get(id);
  if (command === undefined) throw new Error(`commande ${id} absente`);
  return registry.isEnabled(command, app.context());
}

describe("vue Contacts", () => {
  let off: () => void = () => undefined;

  beforeEach(() => {
    off = registerAppCommands();
    mail.conversations = [conversation];
    mail.selectedIndex = 0;
    app.accounts = [
      {
        id: 1,
        display_name: "Moi",
        address: "moi@example.com",
      } as (typeof app.accounts)[number],
    ];
    app.selectedAccountId = 1;
  });

  afterEach(() => {
    off();
    app.showMail();
    contacts.clearMarks();
    void contacts.select(null);
    app.contactTarget = null;
    mail.conversations = [];
  });

  it("masque les commandes du courrier", async () => {
    expect(enabled("action.archive")).toBe(true);
    expect(enabled("conversation.next")).toBe(true);
    await app.showContacts();
    expect(app.view).toBe("contacts");
    for (const id of ["action.archive", "action.delete", "conversation.next", "conversation.open"])
      expect(enabled(id), id).toBe(false);
    app.showMail();
    expect(enabled("action.archive")).toBe(true);
  });

  it("ferme la recherche en ouvrant les contacts", async () => {
    search.open();
    await app.showContacts();
    expect(search.active).toBe(false);
  });

  it("propose les commandes des contacts selon la sélection", async () => {
    await app.showContacts();
    expect(enabled("contacts.new")).toBe(true);
    for (const id of ["contacts.edit", "contacts.delete", "contacts.write", "contacts.favorite"])
      expect(enabled(id), id).toBe(false);

    await contacts.select(4);
    for (const id of ["contacts.edit", "contacts.delete", "contacts.write", "contacts.favorite"])
      expect(enabled(id), id).toBe(true);
    expect(enabled("contacts.clear-selection")).toBe(false);

    // Plusieurs contacts cochés : ni modifier ni favori (une fiche à la fois).
    contacts.toggleMark(4);
    contacts.toggleMark(5);
    expect(enabled("contacts.edit")).toBe(false);
    expect(enabled("contacts.favorite")).toBe(false);
    expect(enabled("contacts.delete")).toBe(true);
    expect(enabled("contacts.write")).toBe(true);
    expect(enabled("contacts.clear-selection")).toBe(true);
  });

  it("écrire demande un compte d'envoi", async () => {
    await app.showContacts();
    await contacts.select(4);
    app.selectedAccountId = null;
    app.accounts = [];
    expect(enabled("contacts.write")).toBe(false);
  });

  it("le menu d'une ligne vise ce contact", async () => {
    await app.showContacts();
    await contacts.select(4);
    contacts.toggleMark(5);
    contacts.toggleMark(6);
    app.contactTarget = 9;
    expect(app.contactTargets()).toEqual([9]);
    expect(enabled("contacts.edit")).toBe(true);
    app.contactTarget = null;
    expect(app.contactTargets()).toEqual([5, 6]);
  });

  it("hors de la vue, les commandes des contacts (sauf ouvrir et créer) ne visent rien", async () => {
    await contacts.select(4);
    expect(enabled("contacts.open")).toBe(true);
    expect(enabled("contacts.new")).toBe(true);
    expect(enabled("contacts.edit")).toBe(false);
    expect(enabled("contacts.delete")).toBe(false);
  });
});
