import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConversationMessage, Draft, RecipientDto } from "./ipc/api";
import { AUTOSAVE_MS, ComposeState, SUGGEST_MS, type ComposeApi } from "./compose.svelte";
import { IpcFailure } from "./errors";
import { setLocale } from "./paraglide/runtime.js";

function empty(account = 1): Draft {
  return {
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
  };
}

const attachment = {
  blob: "ab".repeat(32),
  file_name: "a.pdf",
  content_type: "application/pdf",
  size: 3,
};

/** Limite de destinataires du faux (celle du domaine). */
const LIMIT = 100;

/** Membres `from`..`to` (exclu) d'une liste, `<prefix><n>@example.com`. */
function people(prefix: string, from: number, to: number): RecipientDto[] {
  return Array.from({ length: to - from }, (_, i) => ({
    name: "",
    address: `${prefix}${from + i}@example.com`,
  }));
}

function fakeApi(): ComposeApi & { saved: Draft[] } {
  const saved: Draft[] = [];
  return {
    saved,
    prepareCompose: vi.fn(async (account: number) => empty(account)),
    saveDraft: vi.fn(async (draft: Draft) => {
      saved.push(structuredClone(draft));
      return { id: 42, revision: saved.length };
    }),
    sendDraft: vi.fn(async () => 7),
    scheduleDraft: vi.fn(async () => 8),
    discardDraft: vi.fn(async () => undefined),
    // Lecture de la saisie faite en Rust (Recipient::parse) : ici, un
    // faux qui imite son contrat.
    parseRecipient: vi.fn(async (raw: string) => {
      const angle = /^(.*)<([^<>\s]+)>$/.exec(raw.trim());
      const address = angle ? (angle[2] ?? "") : raw.trim();
      if (!/^[^\s@]+@[^\s@]+$/.test(address)) throw new IpcFailure("invalid_input", "address");
      const name = (angle?.[1] ?? "").trim();
      return { name: /[@<>]/.test(name) ? "" : name, address };
    }),
    suggestContacts: vi.fn(async (_a: number, text: string) => [
      { name: "Alice", address: `${text}@example.com`, contact: 3 },
    ]),
    suggestContactLists: vi.fn(async (text: string) =>
      text.startsWith("fa") ? [{ id: 5, name: "Famille", members: 2, bcc: false }] : [],
    ),
    // Limite décidée par Rust (check_recipient_count) : ici, un faux qui
    // imite son contrat.
    checkRecipients: vi.fn(async (count: number) => {
      if (count > LIMIT) throw new IpcFailure("too_many_recipients");
    }),
    // Champ décidé par Rust : la liste 6 est « en Cci » ; les listes 7, 8
    // et 9 comptent 60 membres (9 en partage 50 avec 7).
    expandContactList: vi.fn(async (id: number, field: "to" | "cc" | "bcc") => {
      if (id === 404) throw new IpcFailure("contact_list_empty");
      if (id === 7) return { recipients: people("sept", 0, 60), field };
      if (id === 8) return { recipients: people("huit", 0, 60), field };
      if (id === 9) return { recipients: people("sept", 10, 70), field };
      return {
        recipients: [
          { name: "Bo", address: "bo@example.net" },
          { name: "Ana", address: "ana@example.com" },
        ],
        field: id === 6 ? ("bcc" as const) : field,
      };
    }),
    pickAttachments: vi.fn(async () => [attachment]),
    attachDropped: vi.fn(async () => [attachment]),
  };
}

const message: ConversationMessage = {
  id: 9,
  account: 1,
  mailbox: 10,
  date: 1_793_613_600,
  from: {
    name: "Alice",
    address: "alice@example.com",
    contact: null,
    show_address: false,
    auth: "unknown",
    contact_badge: false,
  },
  to: [],
  cc: [],
  subject: "Réunion",
  unread: false,
  flagged: false,
  has_attachments: false,
  labels: [],
  newest_copy: 9,
};

describe("rédaction", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    setLocale("fr", { reload: false });
  });
  afterEach(() => vi.useRealTimers());

  it("prépare une réponse avec un en-tête de citation localisé", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "reply", message);
    expect(api.prepareCompose).toHaveBeenCalledWith(
      1,
      "reply",
      9,
      expect.stringContaining("Alice a écrit :"),
    );
    await c.open(1, "forward", message);
    expect(api.prepareCompose).toHaveBeenLastCalledWith(
      1,
      "forward",
      9,
      expect.stringContaining("Message transféré"),
    );
    await c.open(2, "new", null);
    expect(api.prepareCompose).toHaveBeenLastCalledWith(2, "new", null, "");
    expect(c.draft?.account).toBe(2);
  });

  it("enregistre après une pause de frappe, puis réutilise l'identifiant", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    c.edit({ subject: "B" });
    c.edit({ subject: "Bonjour" });
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MS - 1);
    expect(api.saveDraft).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(api.saved).toHaveLength(1);
    expect(api.saved[0]?.subject).toBe("Bonjour");
    expect(c.draft?.id).toBe(42);
    c.edit({ html: "<p>x</p>" });
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MS);
    expect(api.saved[1]?.id).toBe(42);
    // Rien de neuf : pas d'enregistrement.
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MS * 3);
    expect(api.saved).toHaveLength(2);
  });

  it("gère destinataires et suggestions", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    expect(await c.addRecipient("to", "Bob <bob@example.com>")).toBe(true);
    expect(api.parseRecipient).toHaveBeenCalledWith("Bob <bob@example.com>");
    expect(await c.addRecipient("to", "bob@example.com")).toBe(false);
    expect(await c.addRecipient("cc", "n'importe quoi")).toBe(false);
    expect(await c.addRecipient("cc", "   ")).toBe(false);
    expect(c.draft?.to).toEqual([{ name: "Bob", address: "bob@example.com" }]);
    // Le destinataire est celui lu par Rust, nom écarté compris.
    expect(await c.addRecipient("cc", "Banque <support@banque.fr> <pirate@exemple.net>")).toBe(
      true,
    );
    expect(c.draft?.cc).toEqual([{ name: "", address: "pirate@exemple.net" }]);
    c.removeRecipient("to", 0);
    expect(c.draft?.to).toEqual([]);
    c.query("al");
    c.query("ali");
    await vi.advanceTimersByTimeAsync(SUGGEST_MS);
    expect(api.suggestContacts).toHaveBeenCalledTimes(1);
    expect(c.suggestions).toEqual([{ name: "Alice", address: "ali@example.com", contact: 3 }]);
    c.query("");
    await vi.advanceTimersByTimeAsync(SUGGEST_MS);
    expect(c.suggestions).toEqual([]);
  });

  it("propose les listes du carnet avec les autres suggestions", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    c.query("fam");
    await vi.advanceTimersByTimeAsync(SUGGEST_MS);
    expect(api.suggestContactLists).toHaveBeenCalledWith("fam");
    expect(c.listSuggestions).toEqual([{ id: 5, name: "Famille", members: 2, bcc: false }]);
    c.query("");
    expect(c.listSuggestions).toEqual([]);
  });

  it("déplie une liste choisie en puces relues par Rust, chacune une fois", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    await c.addRecipient("to", "ana@example.com");
    expect(await c.addList("to", 5)).toBe(true);
    expect(api.expandContactList).toHaveBeenCalledWith(5, "to");
    expect(api.parseRecipient).toHaveBeenCalledWith("Bo <bo@example.net>");
    expect(c.draft?.to).toEqual([
      { name: "", address: "ana@example.com" },
      { name: "Bo", address: "bo@example.net" },
    ]);
    expect(c.listSuggestions).toEqual([]);
  });

  it("range en Cci les membres d'une liste « en Cci », quel que soit le champ", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    expect(await c.addList("to", 6)).toBe(true);
    expect(c.draft?.to).toEqual([]);
    expect(c.draft?.bcc.map((r) => r.address)).toEqual(["bo@example.net", "ana@example.com"]);
  });

  it("n'ajoute pas les membres d'une liste à un autre brouillon ouvert entre-temps", async () => {
    const api = fakeApi();
    let release: () => void = () => undefined;
    const pending = new Promise<void>((resolve) => (release = resolve));
    const parse = api.parseRecipient;
    api.parseRecipient = vi.fn(async (raw: string) => {
      await pending;
      return parse(raw);
    });
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    const adding = c.addList("to", 5);
    await vi.advanceTimersByTimeAsync(0);
    await c.open(1, "reply", message);
    release();
    await adding;
    expect(c.draft?.to).toEqual([]);
    expect(c.draft?.bcc).toEqual([]);
  });

  it("affiche le refus d'une liste vide", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    expect(await c.addList("cc", 404)).toBe(false);
    expect(c.error).toBe("Cette liste n'a aucun membre : rien à ajouter.");
    expect(c.draft?.cc).toEqual([]);
  });

  it("préremplit les destinataires (« Écrire à »), chacun une fois", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    await c.addRecipients("to", [
      { name: "Dupont, Bob", address: "bob@example.com" },
      { name: "", address: "zoe@example.com" },
      { name: "Bob bis", address: "BOB@example.com" },
      // Fiche nommée comme une adresse : relue par Rust, nom écarté.
      { name: "ceo@banque.fr", address: "pirate@exemple.net" },
    ]);
    expect(c.draft?.to).toEqual([
      { name: "Dupont, Bob", address: "bob@example.com" },
      { name: "", address: "zoe@example.com" },
      { name: "", address: "pirate@exemple.net" },
    ]);
  });

  it("prévient dès l'insertion qu'une liste ferait dépasser la limite, sans l'insérer", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    expect(await c.addList("to", 7)).toBe(true);
    expect(c.recipientWarning).toBeNull();
    expect(await c.addList("cc", 8)).toBe(false);
    expect(api.checkRecipients).toHaveBeenLastCalledWith(120);
    expect(c.recipientWarning).toBe(
      "Destinataires non ajoutés : le message en compterait 120, au-delà de la limite de 100.",
    );
    expect(c.draft?.to).toHaveLength(60);
    expect(c.draft?.cc).toEqual([]);
    // Le brouillon reste enregistrable.
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MS);
    expect(c.error).toBeNull();
    expect(api.saved.at(-1)?.to).toHaveLength(60);
  });

  it("compte une adresse partagée par deux listes une seule fois", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    await c.addList("to", 7);
    expect(await c.addList("to", 9)).toBe(true);
    expect(api.checkRecipients).toHaveBeenLastCalledWith(70);
    expect(c.draft?.to).toHaveLength(70);
    expect(c.recipientWarning).toBeNull();
  });

  it("refuse aussi une adresse saisie au-delà de la limite, et efface l'avertissement au retrait", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    await c.addList("to", 7);
    await c.addList("cc", 9);
    await c.addRecipients("bcc", people("cci", 0, 30));
    expect(c.draft?.bcc).toHaveLength(30);
    expect(await c.addRecipient("to", "un-de-trop@example.com")).toBe(false);
    expect(api.checkRecipients).toHaveBeenLastCalledWith(101);
    expect(c.recipientWarning).toBe(
      "Destinataires non ajoutés : le message en compterait 101, au-delà de la limite de 100.",
    );
    c.removeRecipient("bcc", 0);
    expect(c.recipientWarning).toBeNull();
    expect(await c.addRecipient("to", "un-de-trop@example.com")).toBe(true);
  });

  it("revérifie la limite quand deux listes sont ajoutées en même temps", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    await c.addRecipients("cc", people("p", 0, 40));
    const results = await Promise.all([c.addList("to", 7), c.addList("bcc", 8)]);
    expect(results.filter(Boolean)).toHaveLength(1);
    const d = c.draft;
    expect((d?.to.length ?? 0) + (d?.cc.length ?? 0) + (d?.bcc.length ?? 0)).toBe(100);
    expect(c.recipientWarning).toContain("160");
  });

  it("efface l'avertissement à la tentative suivante", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    await c.addList("to", 7);
    expect(await c.addList("to", 8)).toBe(false);
    expect(c.recipientWarning).not.toBeNull();
    // Adresse déjà présente : refusée pour une autre raison.
    expect(await c.addRecipient("to", "sept0@example.com")).toBe(false);
    expect(c.recipientWarning).toBeNull();
  });

  it("n'ajoute aucun destinataire préparé (« Écrire à ») au-delà de la limite", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    expect(await c.addRecipients("to", people("p", 0, 150))).toBe(false);
    expect(c.draft?.to).toEqual([]);
    expect(c.recipientWarning).toContain("150");
  });

  it("joint des fichiers par le sélecteur et par dépôt", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    await c.pickAttachments("Joindre");
    expect(api.pickAttachments).toHaveBeenCalledWith(1, "Joindre");
    await c.attachDropped(5);
    expect(api.attachDropped).toHaveBeenCalledWith(1, 5);
    expect(c.attachments).toHaveLength(2);
    c.removeAttachment(0);
    expect(c.attachments).toHaveLength(1);
  });

  it("envoie le brouillon courant puis ferme", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    c.addRecipient("to", "bob@example.com");
    c.edit({ subject: "Salut" });
    expect(await c.send()).toBe(true);
    expect(api.sendDraft).toHaveBeenCalledWith(expect.objectContaining({ subject: "Salut" }));
    expect(c.draft).toBeNull();
    // Aucun enregistrement différé après l'envoi.
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MS * 2);
    expect(api.saveDraft).not.toHaveBeenCalled();
  });

  it("garde l'éditeur ouvert et explique un refus", async () => {
    const api = fakeApi();
    api.sendDraft = vi.fn(async () => {
      throw new IpcFailure("no_recipient");
    });
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    expect(await c.send()).toBe(false);
    expect(c.draft).not.toBeNull();
    expect(c.error).toBe("Ajoutez au moins un destinataire.");
  });

  it("enregistre à la fermeture et supprime à l'abandon", async () => {
    const api = fakeApi();
    const c = new ComposeState(api);
    await c.open(1, "new", null);
    c.edit({ subject: "En cours" });
    await c.close();
    expect(api.saved.map((d) => d.subject)).toEqual(["En cours"]);
    expect(c.draft).toBeNull();
    await c.open(1, "new", null);
    c.edit({ subject: "Jeté" });
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MS);
    await c.discard();
    expect(api.discardDraft).toHaveBeenCalledWith(42);
    expect(c.draft).toBeNull();
  });
});

describe("envoi différé et reprise d'un brouillon", () => {
  it("programme l'envoi à l'échéance choisie", async () => {
    const api = fakeApi();
    const compose = new ComposeState(api);
    await compose.open(1, "new", null);
    compose.edit({ subject: "Plus tard" });
    expect(await compose.send(1_900_000_000)).toBe(true);
    expect(api.scheduleDraft).toHaveBeenCalledWith(
      expect.objectContaining({ subject: "Plus tard" }),
      1_900_000_000,
    );
    expect(api.sendDraft).not.toHaveBeenCalled();
    expect(compose.draft).toBeNull();
  });

  it("rouvre un brouillon existant et l'enregistre sous le même identifiant", async () => {
    vi.useFakeTimers();
    const api = fakeApi();
    const compose = new ComposeState(api);
    compose.resume({ ...empty(), id: 5, subject: "Repris" });
    expect(compose.draft?.subject).toBe("Repris");
    compose.edit({ subject: "Repris et modifié" });
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MS);
    expect(api.saved.at(-1)).toEqual(
      expect.objectContaining({ id: 5, subject: "Repris et modifié" }),
    );
    vi.useRealTimers();
  });
});
