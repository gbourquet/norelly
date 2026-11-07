// Vue Contacts dans l'application : les commandes du courrier ne visent
// rien tant qu'elle est affichée ; celles des contacts suivent la fiche
// ouverte, les contacts cochés et le contact dont le menu est ouvert.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConversationDto, ConversationMessage, CorrespondentDto } from "./ipc/api";

const listed = vi.hoisted(() => ({ calls: 0 }));

vi.mock("./ipc/api", () => ({
  IpcFailure: class extends Error {},
  api: {
    listConversations: vi.fn(async () => {
      listed.calls += 1;
      return { items: [], next: null };
    }),
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

const { app, contactForm, contacts, mail, registerAppCommands, registry, search } =
  await import("./app.svelte");

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

function message(id: number, from: CorrespondentDto | null): ConversationMessage {
  return {
    id,
    account: 1,
    mailbox: 1,
    date: 0,
    from,
    to: [],
    cc: [],
    subject: "Bonjour",
    unread: false,
    flagged: false,
    has_attachments: false,
  };
}

const bob: CorrespondentDto = { name: "Bob Martin", address: "bob@example.com", contact: null };
const alice: CorrespondentDto = { name: "Alice", address: "alice@example.com", contact: 4 };

describe("personnes d'un message et carnet", () => {
  let off: () => void = () => undefined;

  beforeEach(() => {
    off = registerAppCommands();
    mail.conversations = [conversation];
    mail.selectedIndex = 0;
  });

  afterEach(() => {
    off();
    app.close();
    app.showMail();
    app.personTarget = null;
    mail.opened = null;
    mail.messages = [];
    mail.conversations = [];
  });

  it("sans message ouvert ni personne visée, rien à ajouter ni à voir", () => {
    expect(app.targetPerson()).toBeNull();
    expect(app.context().person).toBeNull();
    expect(enabled("contacts.add-person")).toBe(false);
    expect(enabled("contacts.show-person")).toBe(false);
  });

  it("vise l'expéditeur du dernier message ouvert", () => {
    mail.opened = conversation;
    mail.messages = [message(1, alice), message(2, bob)];
    expect(app.targetPerson()).toEqual(bob);
    expect(app.context().person).toBe("stranger");
    expect(enabled("contacts.add-person")).toBe(true);
    expect(enabled("contacts.show-person")).toBe(false);
  });

  it("la personne d'un menu l'emporte ; un contact connu se consulte", () => {
    mail.opened = conversation;
    mail.messages = [message(2, bob)];
    app.personTarget = alice;
    expect(app.context().person).toBe("contact");
    expect(enabled("contacts.show-person")).toBe(true);
    expect(enabled("contacts.add-person")).toBe(false);
    expect(app.contextForPerson(bob).person).toBe("stranger");
  });

  it("ajouter au carnet ouvre la fiche pré-remplie, sans quitter le courrier", async () => {
    app.personTarget = bob;
    await registry.execute("contacts.add-person", app.context());
    expect(app.dialog).toBe("contact");
    expect(app.view).toBe("mail");
    expect(app.personTarget).toBeNull();
    expect(contactForm.id).toBeNull();
    expect(contactForm.fields.display_name).toBe("Bob Martin");
    expect(contactForm.fields.emails.map((e) => e.address)).toEqual(["bob@example.com"]);
  });

  it("voir le contact ouvre sa fiche dans la vue Contacts", async () => {
    app.personTarget = alice;
    await registry.execute("contacts.show-person", app.context());
    expect(app.view).toBe("contacts");
    expect(contacts.selectedId).toBe(4);
    expect(app.personTarget).toBeNull();
    // Dans la vue Contacts, aucune personne de message n'est visée.
    expect(enabled("contacts.add-person")).toBe(false);
  });

  it("revenir au courrier relit la liste (noms du carnet à jour)", async () => {
    await app.showContacts();
    const before = listed.calls;
    app.showMail();
    expect(listed.calls).toBe(before + 1);
    app.showMail();
    expect(listed.calls).toBe(before + 1);
  });
});
