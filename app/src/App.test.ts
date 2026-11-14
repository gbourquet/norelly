import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import type { AccountDto } from "./lib/ipc/api";

const accounts: AccountDto[] = [];
function savedDraft(account: number) {
  return {
    id: 5,
    account,
    to: [{ name: "", address: "bob@example.com" }],
    cc: [],
    bcc: [],
    subject: "",
    html: "<p>Texte</p>",
    in_reply_to: null,
    references: [],
    attachments: [],
  };
}
const syncListeners: ((e: unknown) => void)[] = [];
/** Carnet d'adresses factice. */
interface Card {
  id: number;
  name: string;
  fields: {
    display_name: string;
    given_name: string;
    family_name: string;
    nickname: string;
    organization: string;
    notes: string;
    emails: { address: string; label: string; preferred: boolean }[];
    phones: { number: string; label: string }[];
    favorite: boolean;
  };
}
const book: Card[] = [];
function cardOf(id: number, fields: Card["fields"]): Card {
  const name =
    fields.display_name ||
    [fields.given_name, fields.family_name].filter(Boolean).join(" ") ||
    (fields.emails[0]?.address ?? "");
  const emails = fields.emails.some((e) => e.preferred)
    ? fields.emails
    : fields.emails.map((e, i) => ({ ...e, preferred: i === 0 }));
  return { id, name, fields: { ...fields, emails } };
}
const askedSettings = {
  language: "system" as "system" | "fr" | "en",
  update_check: "enabled" as "not_asked" | "enabled" | "disabled",
  notifications: true,
  notification_privacy: false,
  attachment_threshold: 2 * 1024 * 1024,
  mark_seen_delay_secs: 5,
};
let storedSettings = { ...askedSettings };
let updatesSupported = true;

vi.mock("./lib/ipc/api", () => ({
  IpcFailure: class extends Error {},
  api: {
    listAccounts: vi.fn(async () => structuredClone(accounts)),
    discoverSettings: vi.fn(),
    addAccount: vi.fn(),
    trustCertificate: vi.fn(),
    allowFileSecrets: vi.fn(),
    removeAccount: vi.fn(async (id: number) => {
      const i = accounts.findIndex((a) => a.id === id);
      if (i >= 0) accounts.splice(i, 1);
    }),
    listFolders: vi.fn(async () => []),
    listConversations: vi.fn(async () => ({ items: [], next: null })),
    conversationMessages: vi.fn(async () => []),
    openMessage: vi.fn(),
    allowRemoteImages: vi.fn(),
    readingPreferences: vi.fn(async () => ({ prefer_plain_text: false })),
    setPreferPlainText: vi.fn(),
    syncAccount: vi.fn(),
    reconnectAccount: vi.fn(async () => ({ kind: "already_exists" })),
    openExternal: vi.fn(),
    saveAttachment: vi.fn(),
    openAttachment: vi.fn(),
    onSyncEvent: vi.fn(async (listener: (e: unknown) => void) => {
      syncListeners.push(listener);
      return () => undefined;
    }),
    getSettings: vi.fn(async () => ({ ...storedSettings })),
    saveSettings: vi.fn(async (s: typeof storedSettings) => {
      storedSettings = { ...s };
    }),
    showNotification: vi.fn(async () => undefined),
    updateStatus: vi.fn(async () => ({ supported: updatesSupported, available: null })),
    installUpdate: vi.fn(async () => undefined),
    onUpdateEvent: vi.fn(async () => () => undefined),
    onComposeEvent: vi.fn(async () => () => undefined),
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
    saveDraft: vi.fn(async () => ({ id: 1, revision: 1 })),
    sendDraft: vi.fn(async () => 3),
    scheduleDraft: vi.fn(async () => 4),
    unscheduleSend: vi.fn(async (account: number) => ({
      ...savedDraft(account),
      subject: "Programmé",
    })),
    listDrafts: vi.fn(async () => ({
      local: [{ id: 5, subject: "Repris", to: ["bob@example.com"], updated_at: 1_700_000_000 }],
      remote: [{ message: 31, subject: "Écrit ailleurs", date: 1_700_000_000 }],
    })),
    openDraft: vi.fn(async () => ({ ...savedDraft(7), subject: "Repris" })),
    discardDraft: vi.fn(),
    // Faux de la lecture Rust d'un destinataire (Recipient::parse).
    parseRecipient: vi.fn(async (raw: string) => {
      const angle = /^(.*)<([^<>\s]+)>$/.exec(raw.trim());
      return { name: (angle?.[1] ?? "").trim(), address: angle ? (angle[2] ?? "") : raw.trim() };
    }),
    suggestContacts: vi.fn(async () => []),
    pickAttachments: vi.fn(async () => []),
    attachDropped: vi.fn(async () => []),
    outbox: vi.fn(async () => []),
    retrySend: vi.fn(),
    cancelSend: vi.fn(),
    searchMessages: vi.fn(async () => ({ items: [], next: null })),
    createFolder: vi.fn(async () => undefined),
    renameFolder: vi.fn(async () => 13),
    moveFolder: vi.fn(async () => 14),
    deleteFolder: vi.fn(async () => undefined),
    performAction: vi.fn(async () => ({ ops: [9], messages: 1 })),
    listContacts: vi.fn(async (text: string, favoritesOnly: boolean) => ({
      items: book
        .filter((c) => c.name.toLowerCase().includes(text.toLowerCase()))
        .filter((c) => !favoritesOnly || c.fields.favorite)
        .map((c) => ({
          id: c.id,
          name: c.name,
          address: c.fields.emails.find((e) => e.preferred)?.address ?? "",
          organization: c.fields.organization || null,
          favorite: c.fields.favorite,
        })),
      next: null,
    })),
    getContact: vi.fn(async (id: number) => structuredClone(book.find((c) => c.id === id))),
    createContact: vi.fn(async (fields: Card["fields"]) => {
      const card = cardOf(40 + book.length, structuredClone(fields));
      book.push(card);
      return structuredClone(card);
    }),
    updateContact: vi.fn(async (id: number, fields: Card["fields"]) => {
      const card = cardOf(id, structuredClone(fields));
      book.splice(
        book.findIndex((c) => c.id === id),
        1,
        card,
      );
      return structuredClone(card);
    }),
    deleteContacts: vi.fn(async (ids: number[]) => {
      const before = book.length;
      book.splice(0, book.length, ...book.filter((c) => !ids.includes(c.id)));
      return before - book.length;
    }),
  },
}));

const { default: App } = await import("./App.svelte");
const { api } = await import("./lib/ipc/api");
const { app, contacts, registry } = await import("./lib/app.svelte");

const server = {
  host: "imap.example.com",
  port: 993,
  security: "tls" as const,
  username: "alice@example.com",
};

beforeEach(() => {
  // État global de l'application : aucune fenêtre ouverte d'un test à
  // l'autre.
  app.close();
  accounts.splice(0, accounts.length);
  book.splice(0, book.length);
  app.showMail();
  // Carnet : ni filtre, ni sélection d'un test à l'autre.
  contacts.text = "";
  contacts.favoritesOnly = false;
  contacts.clearMarks();
  void contacts.select(null);
  syncListeners.splice(0, syncListeners.length);
  storedSettings = { ...askedSettings };
  updatesSupported = true;
  vi.clearAllMocks();
});

/** Ouvre le menu « ⋯ » d'un compte dans la barre latérale. */
async function openAccountMenu(address: string): Promise<HTMLElement> {
  const button = await screen.findByRole("button", { name: `Actions du compte ${address}` });
  await fireEvent.pointerDown(button, { button: 0, pointerType: "mouse" });
  return screen.findByRole("menu", { name: `Actions du compte ${address}` });
}

/** Valeur présente, sinon échec du test. */
function must<T>(value: T | null | undefined): T {
  if (value === null || value === undefined) throw new Error("valeur attendue");
  return value;
}

/** Ouvre le menu « ⋯ » d'un dossier dans la barre latérale. */
async function openFolderMenu(folder: string): Promise<HTMLElement> {
  const button = await screen.findByRole("button", { name: `Actions du dossier ${folder}` });
  await fireEvent.pointerDown(button, { button: 0, pointerType: "mouse" });
  return screen.findByRole("menu", { name: `Actions du dossier ${folder}` });
}

/** Compte de Bob avec sa boîte de réception (11) et « Projets » (12). */
function bobWithProjets(): void {
  accounts.push({
    id: 3,
    address: "bob@example.org",
    display_name: "Bob",
    imap: server,
    smtp: server,
    oauth_provider: null,
  });
  vi.mocked(api.listFolders).mockResolvedValue([
    {
      id: 11,
      name: "INBOX",
      leaf: "INBOX",
      depth: 0,
      role: "inbox",
      selectable: true,
      total: 0,
      unread: 0,
    },
    {
      id: 12,
      name: "Projets",
      leaf: "Projets",
      depth: 0,
      role: "normal",
      selectable: true,
      total: 4,
      unread: 0,
    },
  ]);
}

describe("App", () => {
  it("affiche l'état vide et propose l'ajout d'un compte", async () => {
    render(App);
    expect(screen.getByRole("heading", { name: "Norelly" })).toBeInTheDocument();
    expect(await screen.findByText("Aucun compte pour l'instant.")).toBeInTheDocument();
    expect(screen.getAllByRole("separator")).toHaveLength(2);
  });

  it("ouvre la palette avec Ctrl+K et l'aide avec ?", async () => {
    render(App);
    await fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    expect(await screen.findByPlaceholderText("Rechercher une commande…")).toBeInTheDocument();
    expect(screen.getByText("Ajouter un compte…")).toBeInTheDocument();
    // Suppression indisponible sans compte sélectionné.
    expect(screen.queryByText("Supprimer le compte sélectionné…")).toBeNull();
    await fireEvent.keyDown(document.activeElement ?? window, { key: "Escape" });
    await waitFor(() =>
      expect(screen.queryByPlaceholderText("Rechercher une commande…")).toBeNull(),
    );

    await fireEvent.keyDown(window, { key: "?", shiftKey: true });
    const help = await screen.findByRole("dialog", { name: "Raccourcis clavier" });
    expect(within(help).getByText("Palette de commandes", { selector: "dt" })).toBeInTheDocument();
  });

  it("liste les comptes et supprime après confirmation", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
      oauth_provider: null,
    });
    render(App);
    expect(await screen.findByText("Alice")).toBeInTheDocument();
    await openAccountMenu("alice@example.com");
    await fireEvent.click(await screen.findByRole("menuitem", { name: "Supprimer ce compte…" }));
    const dialog = await screen.findByRole("dialog", { name: "Supprimer le compte ?" });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Supprimer le compte" }));
    await waitFor(() => expect(api.removeAccount).toHaveBeenCalledWith(7));
    expect(await screen.findByText("Aucun compte pour l'instant.")).toBeInTheDocument();
  });

  it("traduit le nom d'un dossier spécial, nom du serveur en info-bulle", async () => {
    bobWithProjets();
    vi.mocked(api.listFolders).mockResolvedValue([
      ...(await api.listFolders(3)),
      {
        id: 13,
        name: "[Gmail]/Sent Mail",
        leaf: "Sent Mail",
        depth: 1,
        role: "sent",
        selectable: true,
        total: 0,
        unread: 0,
      },
    ]);
    render(App);
    const sent = await screen.findByRole("button", { name: "Envoyés" });
    expect(sent).toHaveAttribute("title", "Nom sur le serveur : Sent Mail");
    expect(screen.queryByText("Sent Mail")).toBeNull();
    // Dossier ordinaire : son nom, sans info-bulle.
    expect(screen.getByRole("button", { name: "Projets" })).not.toHaveAttribute("title");
    await fireEvent.click(sent);
    expect(await screen.findByRole("heading", { name: "Envoyés" })).toBeInTheDocument();
  });

  it("crée un dossier depuis le menu du compte", async () => {
    accounts.push({
      id: 3,
      address: "bob@example.org",
      display_name: "Bob",
      imap: server,
      smtp: server,
      oauth_provider: null,
    });
    vi.mocked(api.listFolders).mockResolvedValue([
      {
        id: 11,
        name: "INBOX",
        leaf: "INBOX",
        depth: 0,
        role: "inbox",
        selectable: true,
        total: 0,
        unread: 0,
      },
      {
        id: 12,
        name: "Projets",
        leaf: "Projets",
        depth: 0,
        role: "normal",
        selectable: true,
        total: 0,
        unread: 0,
      },
    ]);
    render(App);
    await openAccountMenu("bob@example.org");
    await fireEvent.click(await screen.findByRole("menuitem", { name: "Nouveau dossier…" }));
    const dialog = await screen.findByRole("dialog", { name: "Nouveau dossier" });
    const create = within(dialog).getByRole("button", { name: "Créer" });
    expect(create).toBeDisabled();
    await fireEvent.input(within(dialog).getByLabelText("Nom du dossier"), {
      target: { value: "2027" },
    });
    const location = within(dialog).getByLabelText("Emplacement");
    expect(
      within(location)
        .getAllByRole("option")
        .map((o) => o.textContent?.trim()),
    ).toEqual(["Au premier niveau", "Boîte de réception", "Projets"]);
    await fireEvent.change(location, { target: { value: "12" } });
    await fireEvent.click(create);
    await waitFor(() => expect(api.createFolder).toHaveBeenCalledWith(3, "2027", 12));
    expect(await screen.findByText("Dossier « 2027 » créé.")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "Nouveau dossier" })).toBeNull();
    vi.mocked(api.listFolders).mockResolvedValue([]);
  });

  it("renomme un dossier depuis son menu", async () => {
    bobWithProjets();
    render(App);
    // Boîte de réception : ni renommage ni suppression, donc pas de menu.
    await screen.findByRole("button", { name: "Actions du dossier Projets" });
    expect(
      screen.queryByRole("button", { name: "Actions du dossier Boîte de réception" }),
    ).toBeNull();
    const menu = await openFolderMenu("Projets");
    await fireEvent.click(within(menu).getByRole("menuitem", { name: "Renommer le dossier…" }));
    const dialog = await screen.findByRole("dialog", { name: "Renommer le dossier" });
    const input = within(dialog).getByLabelText("Nouveau nom");
    expect(input).toHaveValue("Projets");
    const rename = within(dialog).getByRole("button", { name: "Renommer" });
    expect(rename).toBeDisabled();
    await fireEvent.input(input, { target: { value: "Chantiers" } });
    await fireEvent.click(rename);
    await waitFor(() => expect(api.renameFolder).toHaveBeenCalledWith(3, 12, "Chantiers"));
    expect(await screen.findByText("Dossier renommé en « Chantiers ».")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "Renommer le dossier" })).toBeNull();
    vi.mocked(api.listFolders).mockResolvedValue([]);
  });

  it("déplace un dossier depuis son menu", async () => {
    bobWithProjets();
    render(App);
    const menu = await openFolderMenu("Projets");
    await fireEvent.click(within(menu).getByRole("menuitem", { name: "Déplacer le dossier…" }));
    const dialog = await screen.findByRole("dialog", { name: "Déplacer le dossier" });
    expect(dialog).toHaveTextContent("Projets");
    const location = within(dialog).getByLabelText("Nouvel emplacement");
    // Ni le dossier lui-même ni ses sous-dossiers.
    expect(
      within(location)
        .getAllByRole("option")
        .map((o) => o.textContent?.trim()),
    ).toEqual(["Au premier niveau", "Boîte de réception"]);
    const move = within(dialog).getByRole("button", { name: "Déplacer" });
    expect(move).toBeDisabled();
    await fireEvent.change(location, { target: { value: "11" } });
    await fireEvent.click(move);
    await waitFor(() => expect(api.moveFolder).toHaveBeenCalledWith(3, 12, 11));
    expect(await screen.findByText("Dossier « Projets » déplacé.")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "Déplacer le dossier" })).toBeNull();
    vi.mocked(api.listFolders).mockResolvedValue([]);
  });

  it("supprime un dossier depuis son menu, après confirmation", async () => {
    bobWithProjets();
    render(App);
    const menu = await openFolderMenu("Projets");
    await fireEvent.click(within(menu).getByRole("menuitem", { name: "Supprimer le dossier…" }));
    const dialog = await screen.findByRole("dialog", { name: "Supprimer le dossier" });
    // Le nombre de messages connu localement peut être faux (dossier pas
    // encore synchronisé) : jamais de compte ni de « vide » annoncés.
    expect(dialog).toHaveTextContent(
      "Le dossier « Projets » et tous les messages qu'il contient seront supprimés définitivement du serveur",
    );
    expect(dialog).not.toHaveTextContent("(4)");
    expect(api.deleteFolder).not.toHaveBeenCalled();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Supprimer le dossier" }));
    await waitFor(() => expect(api.deleteFolder).toHaveBeenCalledWith(3, 12));
    expect(await screen.findByText("Dossier « Projets » supprimé.")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "Supprimer le dossier" })).toBeNull();
    vi.mocked(api.listFolders).mockResolvedValue([]);
  });

  it("glisse une conversation sur un dossier pour l'y déplacer", async () => {
    bobWithProjets();
    const conversation = (id: number, subject: string) => ({
      id,
      account: 3,
      mailbox: 11,
      date: 1_700_000_000,
      subject,
      senders: [],
      messages: 1,
      unread: 0,
      flagged: false,
      has_attachments: false,
    });
    vi.mocked(api.listConversations).mockResolvedValue({
      items: [conversation(21, "Devis cuisine"), conversation(22, "Facture")],
      next: null,
    });
    render(App);
    const row = must((await screen.findByText("Devis cuisine")).closest('[role="option"]'));
    const projets = must((await screen.findByText("Projets")).closest("button"));
    const inbox = must(
      screen
        .getAllByText("Boîte de réception")
        .map((e) => e.closest("button"))
        .find((b) => b?.dataset.dropFolder === "11"),
    );
    let under: Element = inbox;
    document.elementFromPoint = vi.fn(() => under);

    // Un petit déplacement reste un clic : rien n'est emporté.
    await fireEvent.pointerDown(row, { button: 0, pointerType: "mouse", clientX: 10, clientY: 10 });
    await fireEvent.pointerMove(window, { pointerType: "mouse", clientX: 12, clientY: 11 });
    expect(document.querySelector(".drag-ghost")).toBeNull();

    // Au-delà du seuil : l'étiquette suit le pointeur.
    await fireEvent.pointerMove(window, { pointerType: "mouse", clientX: 60, clientY: 40 });
    expect(document.querySelector(".drag-ghost")).toHaveTextContent("Devis cuisine");
    // Dossier d'origine : pas une cible.
    expect(inbox.closest("li")).not.toHaveClass("drop-over");
    under = projets;
    await fireEvent.pointerMove(window, { pointerType: "mouse", clientX: 20, clientY: 200 });
    expect(projets.closest("li")).toHaveClass("drop-over");
    await fireEvent.pointerUp(window, { pointerType: "mouse", clientX: 20, clientY: 200 });
    await waitFor(() =>
      expect(api.performAction).toHaveBeenCalledWith(
        [21],
        { kind: "unified", role: "inbox" },
        {
          kind: "move",
          mailbox: 12,
        },
      ),
    );
    expect(document.querySelector(".drag-ghost")).toBeNull();
    expect(await screen.findByText("Déplacé vers « Projets ».")).toBeInTheDocument();
    vi.mocked(api.listFolders).mockResolvedValue([]);
    vi.mocked(api.listConversations).mockResolvedValue({ items: [], next: null });
  });

  it("glisse un dossier dans un autre, puis au premier niveau", async () => {
    bobWithProjets();
    const extra = (id: number, name: string) => ({
      id,
      name,
      leaf: name.split("/").at(-1) ?? name,
      depth: name.split("/").length - 1,
      role: "normal",
      selectable: true,
      total: 0,
      unread: 0,
    });
    const listed = await api.listFolders(3);
    vi.mocked(api.listFolders).mockResolvedValue([
      ...listed,
      extra(13, "Factures"),
      extra(14, "Factures/2026"),
    ]);
    render(App);
    const button = async (text: string) =>
      must((await screen.findByText(text)).closest<HTMLElement>("button"));
    const projets = await button("Projets");
    const factures = await button("Factures");
    const year = await button("2026");
    const root = must(
      screen.getByText("bob@example.org").closest<HTMLElement>("[data-drop-account]"),
    );
    let under: Element = projets;
    document.elementFromPoint = vi.fn(() => under);
    const drag = async (from: HTMLElement, to: Element) => {
      under = from;
      await fireEvent.pointerDown(from, {
        button: 0,
        pointerType: "mouse",
        clientX: 10,
        clientY: 10,
      });
      await fireEvent.pointerMove(window, { pointerType: "mouse", clientX: 60, clientY: 40 });
      under = to;
      await fireEvent.pointerMove(window, { pointerType: "mouse", clientX: 20, clientY: 200 });
    };

    // Dans lui-même : pas une cible.
    await drag(projets, projets);
    expect(document.querySelector(".drag-ghost")).toHaveTextContent("Projets");
    expect(projets.closest("li")).not.toHaveClass("drop-over");
    // Dans un autre dossier.
    under = factures;
    await fireEvent.pointerMove(window, { pointerType: "mouse", clientX: 20, clientY: 220 });
    expect(factures.closest("li")).toHaveClass("drop-over");
    await fireEvent.pointerUp(window, { pointerType: "mouse", clientX: 20, clientY: 220 });
    await waitFor(() => expect(api.moveFolder).toHaveBeenCalledWith(3, 12, 13));
    expect(await screen.findByText("Dossier « Projets » déplacé.")).toBeInTheDocument();
    // Le clic qui termine le glisser n'affiche pas le dossier.
    await fireEvent.click(projets);
    expect(api.listConversations).not.toHaveBeenCalledWith(
      { kind: "mailbox", id: 12 },
      expect.anything(),
      expect.anything(),
    );

    // Un sous-dossier, au premier niveau : sur la ligne du compte.
    await drag(year, root);
    expect(root).toHaveClass("drop-over");
    await fireEvent.pointerUp(window, { pointerType: "mouse", clientX: 20, clientY: 200 });
    await waitFor(() => expect(api.moveFolder).toHaveBeenCalledWith(3, 14, null));
    vi.mocked(api.listFolders).mockResolvedValue([]);
  });

  it("Échap annule un glisser en cours", async () => {
    bobWithProjets();
    vi.mocked(api.listConversations).mockResolvedValue({
      items: [
        {
          id: 21,
          account: 3,
          mailbox: 11,
          date: 1_700_000_000,
          subject: "Devis cuisine",
          senders: [],
          messages: 1,
          unread: 0,
          flagged: false,
          has_attachments: false,
        },
      ],
      next: null,
    });
    render(App);
    const row = must((await screen.findByText("Devis cuisine")).closest('[role="option"]'));
    const projets = must((await screen.findByText("Projets")).closest("button"));
    document.elementFromPoint = vi.fn(() => projets);
    await fireEvent.pointerDown(row, { button: 0, pointerType: "mouse", clientX: 10, clientY: 10 });
    await fireEvent.pointerMove(window, { pointerType: "mouse", clientX: 20, clientY: 200 });
    expect(document.querySelector(".drag-ghost")).not.toBeNull();
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(document.querySelector(".drag-ghost")).toBeNull();
    await fireEvent.pointerUp(window, { pointerType: "mouse", clientX: 20, clientY: 200 });
    expect(api.performAction).not.toHaveBeenCalled();
    vi.mocked(api.listFolders).mockResolvedValue([]);
    vi.mocked(api.listConversations).mockResolvedValue({ items: [], next: null });
  });

  it("annuler la suppression ne supprime rien", async () => {
    accounts.push({
      id: 3,
      address: "bob@example.org",
      display_name: "Bob",
      imap: server,
      smtp: server,
      oauth_provider: null,
    });
    render(App);
    await openAccountMenu("bob@example.org");
    await fireEvent.click(await screen.findByRole("menuitem", { name: "Supprimer ce compte…" }));
    const dialog = await screen.findByRole("dialog", { name: "Supprimer le compte ?" });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Annuler" }));
    expect(api.removeAccount).not.toHaveBeenCalled();
  });

  it("ignore les raccourcis sans modificateur dans un champ de saisie", async () => {
    render(App);
    await fireEvent.click(
      await screen.findByRole("button", { name: "Ajouter un compte", hidden: false }),
    );
    const address = await screen.findByLabelText("Adresse électronique");
    await fireEvent.keyDown(address, { key: "?", shiftKey: true });
    expect(screen.queryByRole("dialog", { name: "Raccourcis clavier" })).toBeNull();
  });

  it("rédige avec c et envoie avec Ctrl+Entrée", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
      oauth_provider: null,
    } as AccountDto);
    render(App);
    await screen.findAllByText("alice@example.com");
    await fireEvent.keyDown(window, { key: "c" });
    const dialog = await screen.findByRole("dialog", { name: "Nouveau message" });
    expect(api.prepareCompose).toHaveBeenCalledWith(7, "new", null, "");
    const to = within(dialog).getByRole("combobox", { name: "À" });
    // Bouton Fermer en dernier dans l'ordre de tabulation : le premier
    // champ reçoit le focus à l'ouverture.
    expect(within(dialog).getAllByRole("button").at(-1)).toHaveAccessibleName("Fermer");
    await fireEvent.input(to, { target: { value: "bob@example.com" } });
    await fireEvent.keyDown(to, { key: "Enter" });
    expect(within(dialog).getByText("bob@example.com")).toBeInTheDocument();
    const subject = within(dialog).getByRole("textbox", { name: "Objet" });
    await fireEvent.input(subject, { target: { value: "Bonjour" } });
    await fireEvent.keyDown(subject, { key: "Enter", ctrlKey: true });
    await waitFor(() =>
      expect(api.sendDraft).toHaveBeenCalledWith(
        expect.objectContaining({
          subject: "Bonjour",
          to: [{ name: "", address: "bob@example.com" }],
        }),
      ),
    );
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Nouveau message" })).toBeNull(),
    );
  });

  it("cherche avec /, surligne, ouvre un résultat et revient", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
      oauth_provider: null,
    } as AccountDto);
    const found = (n: number, subject: string) => ({
      message: n,
      conversation: 100 + n,
      account: 7,
      mailbox: 30 + n,
      date: 1_700_000_000,
      from: { name: "Bob", address: "bob@example.com", contact: null },
      subject: [
        { text: "Facture", hit: true },
        { text: subject, hit: false },
      ],
      snippet: [
        { text: "la ", hit: false },
        { text: "facture", hit: true },
      ],
      unread: false,
      flagged: false,
      has_attachments: false,
    });
    vi.mocked(api.searchMessages).mockResolvedValue({
      items: [found(1, " d'avril"), found(2, " <img src=x onerror=alert(1)>")],
      next: null,
    });
    render(App);
    await screen.findAllByText("alice@example.com");
    await fireEvent.keyDown(window, { key: "/" });
    const box = await screen.findByRole("searchbox", { name: "Rechercher dans les messages" });
    await waitFor(() => expect(box).toHaveFocus());
    await fireEvent.input(box, { target: { value: "facture" } });
    await fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(api.searchMessages).toHaveBeenCalledWith(
        "facture",
        { kind: "all" },
        expect.any(Number),
        0,
        50,
      ),
    );
    const results = await screen.findByRole("listbox", { name: "Résultats de la recherche" });
    await waitFor(() => expect(within(results).getAllByRole("option")).toHaveLength(2));
    await waitFor(() => expect(results).toHaveFocus());
    const marks = [...results.querySelectorAll("mark")].map((e) => e.textContent);
    expect(marks).toEqual(["Facture", "facture", "Facture", "facture"]);
    // Texte reçu affiché comme du texte, jamais interprété.
    expect(within(results).getByText(/<img src=x onerror=alert\(1\)>/)).toBeInTheDocument();
    expect(results.querySelector("img")).toBeNull();

    await fireEvent.keyDown(results, { key: "j" });
    await fireEvent.keyDown(results, { key: "Enter" });
    await waitFor(() => expect(api.conversationMessages).toHaveBeenCalledWith(102));
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.getByRole("listbox", { name: "Résultats de la recherche" })).toBeInTheDocument();
    await fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("searchbox")).toBeNull());
    expect(screen.getByRole("listbox", { name: "Conversations" })).toBeInTheDocument();
  });

  it("signale une recherche sans résultat", async () => {
    vi.mocked(api.searchMessages).mockResolvedValue({ items: [], next: null });
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
      oauth_provider: null,
    } as AccountDto);
    render(App);
    await screen.findAllByText("alice@example.com");
    await fireEvent.click(screen.getByRole("button", { name: "Rechercher dans les messages" }));
    const box = await screen.findByRole("searchbox", { name: "Rechercher dans les messages" });
    await fireEvent.input(box, { target: { value: "zzz" } });
    await fireEvent.keyDown(box, { key: "Enter" });
    expect(await screen.findByText("Aucun message trouvé.")).toBeInTheDocument();
    await fireEvent.keyDown(box, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("searchbox")).toBeNull());
  });

  it("demande au premier lancement s'il faut chercher les mises à jour (oui présélectionné)", async () => {
    storedSettings = { ...askedSettings, update_check: "not_asked" };
    render(App);
    const dialog = await screen.findByRole("dialog", { name: "Mises à jour" });
    const yes = within(dialog).getByRole("button", { name: "Oui" });
    await waitFor(() => expect(yes).toHaveFocus());
    await fireEvent.click(yes);
    await waitFor(() =>
      expect(api.saveSettings).toHaveBeenCalledWith(
        expect.objectContaining({ update_check: "enabled" }),
      ),
    );
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Mises à jour" })).toBeNull());
  });

  it("ouvre les paramètres avec Ctrl+, et les enregistre", async () => {
    render(App);
    await waitFor(() => expect(api.getSettings).toHaveBeenCalled());
    await fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    const dialog = await screen.findByRole("dialog", { name: "Paramètres" });
    await fireEvent.click(
      within(dialog).getByRole("checkbox", {
        name: "Masquer l'expéditeur et le sujet dans les notifications",
      }),
    );
    await fireEvent.click(within(dialog).getByRole("button", { name: "Enregistrer" }));
    await waitFor(() =>
      expect(api.saveSettings).toHaveBeenCalledWith(
        expect.objectContaining({ notification_privacy: true, update_check: "enabled" }),
      ),
    );
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Paramètres" })).toBeNull());
  });

  it("notifie les nouveaux messages quand la fenêtre n'a pas le focus", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
      oauth_provider: null,
    } as AccountDto);
    const focus = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    render(App);
    await screen.findAllByText("alice@example.com");
    await waitFor(() => expect(syncListeners.length).toBeGreaterThan(0));
    for (const l of syncListeners) {
      l({
        kind: "new_mail",
        account: 7,
        count: 1,
        from: { name: "Bob", address: "bob@example.com" },
        subject: "Devis",
      });
    }
    await waitFor(() => expect(api.showNotification).toHaveBeenCalledWith("Bob", "Devis"));
    focus.mockReturnValue(true);
    vi.mocked(api.showNotification).mockClear();
    for (const l of syncListeners) {
      l({ kind: "new_mail", account: 7, count: 2, from: null, subject: null });
    }
    await new Promise((r) => setTimeout(r, 20));
    expect(api.showNotification).not.toHaveBeenCalled();
    focus.mockRestore();
  });

  it("compte Microsoft à reconnecter : bouton visible et entrée du menu", async () => {
    accounts.push({
      id: 9,
      address: "carol@outlook.com",
      display_name: "Carol",
      imap: server,
      smtp: server,
      oauth_provider: "microsoft",
    } as AccountDto);
    render(App);
    await screen.findAllByText("carol@outlook.com");
    expect(screen.queryByRole("button", { name: "Se reconnecter" })).toBeNull();
    await waitFor(() => expect(syncListeners.length).toBeGreaterThan(0));
    for (const l of syncListeners) {
      l({ kind: "offline", account: 9, failure: "reauth_required", detail: null });
    }
    const button = await screen.findByRole("button", { name: "Se reconnecter" });
    expect(button).toHaveAttribute("title", "Reconnexion nécessaire : carol@outlook.com");
    await fireEvent.click(button);
    await waitFor(() => expect(api.reconnectAccount).toHaveBeenCalledWith(9));
    const menu = await openAccountMenu("carol@outlook.com");
    expect(within(menu).getByRole("menuitem", { name: "Se reconnecter" })).toBeEnabled();
  });

  it("synchronisation en cours : progression visible sous le compte, puis effacée", async () => {
    accounts.push({
      id: 8,
      address: "dave@gmail.com",
      display_name: "Dave",
      imap: server,
      smtp: server,
      oauth_provider: null,
    } as AccountDto);
    render(App);
    await screen.findAllByText("dave@gmail.com");
    await waitFor(() => expect(syncListeners.length).toBeGreaterThan(0));
    const send = (done: number) => {
      for (const l of syncListeners) l({ kind: "sync_progress", account: 8, done, total: 61 });
    };
    send(12);
    const status = await screen.findByRole("status", {
      name: "Synchronisation de dave@gmail.com : 12 dossiers sur 61",
    });
    expect(status).toHaveTextContent("Synchronisation… 12 / 61");
    send(61);
    await waitFor(() => expect(screen.queryByText("Synchronisation… 61 / 61")).toBeNull());
    expect(screen.queryByText(/Synchronisation…/)).toBeNull();
  });

  it("donne un nom accessible à chaque contrôle (passe d'accessibilité)", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
      oauth_provider: null,
    } as AccountDto);
    render(App);
    await screen.findAllByText("alice@example.com");
    const unnamed = () =>
      ["button", "textbox", "searchbox", "checkbox", "combobox", "listbox"].flatMap((role) =>
        screen
          .queryAllByRole(role, { name: "" })
          .map((e) => `${role} : ${e.outerHTML.slice(0, 80)}`),
      );
    expect(unnamed()).toEqual([]);
    await fireEvent.keyDown(window, { key: "/" });
    await screen.findByRole("searchbox");
    expect(unnamed()).toEqual([]);
    await fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    await screen.findByRole("dialog", { name: "Paramètres" });
    expect(unnamed()).toEqual([]);
  });

  it("rouvre un brouillon depuis la liste des brouillons", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
      oauth_provider: null,
    } as AccountDto);
    render(App);
    await screen.findAllByText("alice@example.com");
    await registry.execute("drafts.open", app.context());
    const list = await screen.findByRole("dialog", { name: "Brouillons" });
    await fireEvent.click(within(list).getByRole("button", { name: /Repris/ }));
    await waitFor(() => expect(api.openDraft).toHaveBeenCalledWith(5));
    const dialog = await screen.findByRole("dialog", { name: "Nouveau message" });
    expect(within(dialog).getByRole("textbox", { name: "Objet" })).toHaveValue("Repris");
  });

  it("programme l'envoi à la date choisie", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
      oauth_provider: null,
    } as AccountDto);
    render(App);
    await screen.findAllByText("alice@example.com");
    await fireEvent.keyDown(window, { key: "c" });
    const dialog = await screen.findByRole("dialog", { name: "Nouveau message" });
    const to = within(dialog).getByRole("combobox", { name: "À" });
    await fireEvent.input(to, { target: { value: "bob@example.com" } });
    await fireEvent.keyDown(to, { key: "Enter" });
    const later = within(dialog).getByLabelText("Envoyer plus tard");
    await fireEvent.input(later, { target: { value: "2030-01-15T09:00" } });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Programmer" }));
    await waitFor(() =>
      expect(api.scheduleDraft).toHaveBeenCalledWith(
        expect.objectContaining({ to: [{ name: "", address: "bob@example.com" }] }),
        Math.floor(new Date("2030-01-15T09:00").getTime() / 1000),
      ),
    );
    expect(api.sendDraft).not.toHaveBeenCalled();
  });

  it("reprend un envoi programmé pour le modifier", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
      oauth_provider: null,
    } as AccountDto);
    vi.mocked(api.outbox).mockResolvedValue([
      {
        id: 11,
        subject: "Programmé",
        recipients: ["bob@example.com"],
        state: { state: "queued", not_before: 1_900_000_000 },
        attempts: 0,
        queued_at: 1_700_000_000,
      },
    ]);
    render(App);
    await screen.findAllByText("alice@example.com");
    await registry.execute("compose.outbox", app.context());
    const box = await screen.findByRole("dialog", { name: "Boîte d'envoi" });
    expect(within(box).getByText(/^Programmé pour le /)).toBeInTheDocument();
    await fireEvent.click(within(box).getByRole("button", { name: "Modifier" }));
    await waitFor(() => expect(api.unscheduleSend).toHaveBeenCalledWith(7, 11));
    const dialog = await screen.findByRole("dialog", { name: "Nouveau message" });
    expect(within(dialog).getByRole("textbox", { name: "Objet" })).toHaveValue("Programmé");
    vi.mocked(api.outbox).mockResolvedValue([]);
  });

  it("reprend un brouillon écrit par un autre client", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
      oauth_provider: null,
    } as AccountDto);
    vi.mocked(api.prepareCompose).mockResolvedValueOnce({
      ...savedDraft(7),
      id: null,
      subject: "Écrit ailleurs",
    });
    render(App);
    await screen.findAllByText("alice@example.com");
    await registry.execute("drafts.open", app.context());
    const list = await screen.findByRole("dialog", { name: "Brouillons" });
    await fireEvent.click(within(list).getByRole("button", { name: /Écrit ailleurs/ }));
    await waitFor(() => expect(api.prepareCompose).toHaveBeenCalledWith(7, "resume", 31, ""));
    const dialog = await screen.findByRole("dialog", { name: "Nouveau message" });
    expect(within(dialog).getByRole("textbox", { name: "Objet" })).toHaveValue("Écrit ailleurs");
  });

  it("ne pose pas la question sur un canal qui ne se met pas à jour lui-même", async () => {
    storedSettings = { ...askedSettings, update_check: "not_asked" };
    updatesSupported = false;
    render(App);
    await waitFor(() => expect(api.updateStatus).toHaveBeenCalled());
    await waitFor(() => expect(api.getSettings).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByRole("dialog", { name: "Mises à jour" })).toBeNull();
  });

  it("Échap sur la question des mises à jour n'est pas un consentement", async () => {
    storedSettings = { ...askedSettings, update_check: "not_asked" };
    render(App);
    const dialog = await screen.findByRole("dialog", { name: "Mises à jour" });
    await fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Mises à jour" })).toBeNull());
    expect(api.saveSettings).not.toHaveBeenCalled();
  });

  it("barre latérale : nouveau message, menu du compte, paramètres", async () => {
    accounts.push(
      {
        id: 7,
        address: "alice@example.com",
        display_name: "Alice",
        imap: server,
        smtp: server,
        oauth_provider: null,
      },
      {
        id: 8,
        address: "bob@example.org",
        display_name: "Bob",
        imap: server,
        smtp: server,
        oauth_provider: null,
      },
    );
    render(App);
    const nav = await screen.findByRole("navigation", { name: "Comptes et dossiers" });

    // Nouveau message : bouton libellé en haut, raccourci dans l'infobulle.
    const write = within(nav).getByRole("button", { name: "Nouveau message" });
    expect(write).toHaveTextContent("Nouveau message");
    expect(write).toHaveAttribute("title", "Nouveau message — C");
    await waitFor(() => expect(write).toBeEnabled());
    await fireEvent.click(write);
    expect(await screen.findByRole("dialog", { name: "Nouveau message" })).toBeInTheDocument();
    expect(api.prepareCompose).toHaveBeenCalledWith(7, "new", null, "");
    app.close();

    // Menu du compte : actions visibles, appliquées à ce compte.
    const menu = await openAccountMenu("bob@example.org");
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((i) => i.textContent?.replace(/\s+/g, " ").trim()),
    ).toEqual([
      "Synchroniser",
      "Brouillons…",
      "Boîte d'envoi",
      "Nouveau dossier…",
      "Choisir le dossier d'archives…",
      "Supprimer ce compte…",
    ]);
    await fireEvent.click(within(menu).getByRole("menuitem", { name: "Synchroniser" }));
    await waitFor(() => expect(api.syncAccount).toHaveBeenCalledWith(8));

    // Paramètres et ajout de compte en bas de la barre.
    expect(within(nav).getByRole("button", { name: "Ajouter un compte" })).toHaveTextContent(
      "Ajouter un compte",
    );
    const settingsButton = within(nav).getByRole("button", { name: "Paramètres" });
    expect(settingsButton).toHaveAttribute("title", "Paramètres — Ctrl+,");
    await fireEvent.click(settingsButton);
    expect(await screen.findByRole("dialog", { name: "Paramètres" })).toBeInTheDocument();
  });

  it("bouton « ? » visible : aide des raccourcis par catégorie, fermée par un bouton", async () => {
    render(App);
    const nav = await screen.findByRole("navigation", { name: "Comptes et dossiers" });
    const help = within(nav).getByRole("button", { name: "Aide des raccourcis clavier" });
    expect(help).toHaveTextContent("?");
    expect(help).toHaveAttribute("title", "Aide des raccourcis clavier — ?");
    await fireEvent.click(help);
    const dialog = await screen.findByRole("dialog", { name: "Raccourcis clavier" });
    const groups = within(dialog)
      .getAllByRole("heading", { level: 3 })
      .map((h) => h.textContent);
    expect(groups).toEqual(
      expect.arrayContaining([
        "Général",
        "Lecture",
        "Actions",
        "Rédaction",
        "Dans la fenêtre de rédaction",
      ]),
    );
    expect(within(dialog).getByText("Envoyer le message")).toBeInTheDocument();

    // Fermeture à la souris.
    await fireEvent.click(within(dialog).getByRole("button", { name: "Fermer" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Raccourcis clavier" })).toBeNull(),
    );

    // La palette s'ouvre aussi à la souris, depuis l'aide.
    await fireEvent.click(help);
    const again = await screen.findByRole("dialog", { name: "Raccourcis clavier" });
    const palette = within(again).getByRole("button", { name: "Palette de commandes" });
    expect(palette).toHaveAttribute("title", "Palette de commandes — Ctrl+K");
    await fireEvent.click(palette);
    expect(await screen.findByPlaceholderText("Rechercher une commande…")).toBeInTheDocument();
    const paletteDialog = screen.getByRole("dialog", { name: "Palette de commandes" });
    await fireEvent.click(within(paletteDialog).getByRole("button", { name: "Fermer" }));
    await waitFor(() =>
      expect(screen.queryByPlaceholderText("Rechercher une commande…")).toBeNull(),
    );
  });

  it("carnet d'adresses à la souris : ajouter, modifier, favori, écrire, supprimer", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
      oauth_provider: null,
    });
    render(App);
    const nav = await screen.findByRole("navigation", { name: "Comptes et dossiers" });
    const open = within(nav).getByRole("button", { name: "Contacts" });
    expect(open).toHaveAttribute("title", "Contacts — Ctrl+Maj+B");
    await fireEvent.click(open);
    expect(await screen.findByRole("heading", { name: "Contacts" })).toBeInTheDocument();
    expect(open).toHaveAttribute("aria-current", "true");
    expect(await screen.findByText(/Aucun contact pour l'instant/)).toBeInTheDocument();
    expect(screen.getByText(/Choisissez un contact/)).toBeInTheDocument();

    // Nouveau contact : deux adresses, la seconde préférée.
    await fireEvent.click(screen.getByRole("button", { name: "Nouveau contact" }));
    const form = await screen.findByRole("dialog", { name: "Nouveau contact" });
    const save = within(form).getByRole("button", { name: "Enregistrer" });
    expect(save).toBeDisabled();
    await fireEvent.input(within(form).getByLabelText("Prénom"), { target: { value: "Zoé" } });
    await fireEvent.input(within(form).getByLabelText("Nom"), { target: { value: "Martin" } });
    await fireEvent.input(within(form).getByLabelText("Adresse électronique 1"), {
      target: { value: "zoe@example.com" },
    });
    await fireEvent.click(within(form).getByRole("button", { name: "Ajouter une adresse" }));
    await fireEvent.input(within(form).getByLabelText("Adresse électronique 2"), {
      target: { value: "zoe.pro@example.com" },
    });
    await fireEvent.click(must(within(form).getAllByRole("radio")[1]));
    await fireEvent.click(within(form).getByRole("button", { name: "Ajouter un téléphone" }));
    await fireEvent.input(within(form).getByLabelText("Téléphone 1"), {
      target: { value: "01 02 03 04 05" },
    });
    await fireEvent.input(within(form).getByLabelText("Notes"), {
      target: { value: "<b>pas du HTML</b>" },
    });
    await fireEvent.click(save);
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Nouveau contact" })).toBeNull(),
    );
    expect(api.createContact).toHaveBeenCalledWith(
      expect.objectContaining({
        given_name: "Zoé",
        emails: [
          { address: "zoe@example.com", label: "", preferred: false },
          { address: "zoe.pro@example.com", label: "", preferred: true },
        ],
        phones: [{ number: "01 02 03 04 05", label: "" }],
      }),
    );
    const card = await screen.findByRole("article", { name: "Fiche de Zoé Martin" });
    expect(within(card).getByText("zoe.pro@example.com")).toBeInTheDocument();
    expect(within(card).getByText("préférée")).toBeInTheDocument();
    expect(within(card).getByText("<b>pas du HTML</b>")).toBeInTheDocument();
    expect(card.querySelector("b")).toBeNull();
    expect(screen.getByRole("option", { name: /Zoé Martin/ })).toBeInTheDocument();
    expect(screen.getByText("Contact « Zoé Martin » ajouté.")).toBeInTheDocument();

    // Modifier : fenêtre préremplie.
    await fireEvent.click(within(card).getByRole("button", { name: "Modifier le contact" }));
    const edit = await screen.findByRole("dialog", { name: "Modifier le contact" });
    expect(within(edit).getByLabelText("Prénom")).toHaveValue("Zoé");
    // La première adresse retirée, la seconde reste préférée.
    await fireEvent.click(within(edit).getByRole("button", { name: "Retirer l'adresse 1" }));
    expect(within(edit).getByRole("button", { name: "Retirer l'adresse 1" })).toBeDisabled();
    await fireEvent.input(within(edit).getByLabelText("Surnom"), { target: { value: "Zozo" } });
    await fireEvent.click(within(edit).getByRole("button", { name: "Enregistrer" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Modifier le contact" })).toBeNull(),
    );
    expect(api.updateContact).toHaveBeenCalledWith(
      40,
      expect.objectContaining({ nickname: "Zozo" }),
    );
    expect(await screen.findByText("Surnom : Zozo")).toBeInTheDocument();

    // Favori depuis le menu du clic droit d'une ligne.
    await fireEvent.contextMenu(screen.getByRole("option", { name: /Zoé Martin/ }));
    const menu = await screen.findByRole("menu", { name: "Actions sur le contact" });
    await fireEvent.click(within(menu).getByRole("menuitem", { name: /Ajouter aux favoris/ }));
    expect(await screen.findByText("Zoé Martin ajouté aux favoris.")).toBeInTheDocument();
    await fireEvent.click(screen.getByRole("button", { name: "Favoris" }));
    await waitFor(() => expect(api.listContacts).toHaveBeenLastCalledWith("", true, null, 100));
    expect((await screen.findAllByLabelText("Favori", { selector: "svg" })).length).toBeGreaterThan(
      0,
    );

    // Écrire au contact : rédaction adressée à l'adresse préférée.
    await fireEvent.click(
      within(await screen.findByRole("article", { name: "Fiche de Zoé Martin" })).getByRole(
        "button",
        { name: "Écrire au contact" },
      ),
    );
    const compose = await screen.findByRole("dialog", { name: "Nouveau message" });
    expect(api.prepareCompose).toHaveBeenCalledWith(7, "new", null, "");
    expect(await within(compose).findByText(/zoe\.pro@example\.com/)).toBeInTheDocument();
    app.close();

    // Supprimer, après confirmation.
    await fireEvent.click(screen.getByRole("button", { name: "Supprimer le contact" }));
    const confirm = await screen.findByRole("dialog", { name: "Supprimer le contact" });
    expect(within(confirm).getByText(/Supprimer « Zoé Martin »/)).toBeInTheDocument();
    await fireEvent.click(within(confirm).getByRole("button", { name: "Supprimer" }));
    await waitFor(() => expect(api.deleteContacts).toHaveBeenCalledWith([40]));
    expect(await screen.findByText("1 contact(s) supprimé(s).")).toBeInTheDocument();
    expect(screen.queryByRole("article", { name: "Fiche de Zoé Martin" })).toBeNull();

    // Retour au courrier par la boîte unifiée.
    await fireEvent.click(within(nav).getByRole("button", { name: "Boîte de réception unifiée" }));
    expect(screen.queryByRole("heading", { name: "Contacts" })).toBeNull();
    expect(open).not.toHaveAttribute("aria-current");
  });

  it("carnet : refus affiché, recherche, sélection multiple", async () => {
    book.push(
      cardOf(1, {
        display_name: "",
        given_name: "Alice",
        family_name: "",
        nickname: "",
        organization: "ACME",
        notes: "",
        emails: [{ address: "alice@example.com", label: "", preferred: true }],
        phones: [],
        favorite: false,
      }),
      cardOf(2, {
        display_name: "",
        given_name: "Bob",
        family_name: "",
        nickname: "",
        organization: "",
        notes: "",
        emails: [{ address: "bob@example.com", label: "", preferred: true }],
        phones: [],
        favorite: false,
      }),
    );
    const { IpcFailure } = await import("./lib/errors");
    vi.mocked(api.createContact).mockRejectedValueOnce(
      new IpcFailure("contact_duplicate_address", null, "alice@example.com"),
    );
    render(App);
    const nav = await screen.findByRole("navigation", { name: "Comptes et dossiers" });
    await fireEvent.click(within(nav).getByRole("button", { name: "Contacts" }));
    expect(await screen.findByRole("option", { name: /Alice/ })).toBeInTheDocument();
    expect(
      screen.getByText(
        (_, el) =>
          el?.classList.contains("detail") === true &&
          el.textContent?.replace(/\s+/g, " ").trim() === "alice@example.com · ACME",
      ),
    ).toBeInTheDocument();

    // Adresse déjà prise : fenêtre gardée, refus expliqué.
    await fireEvent.click(screen.getByRole("button", { name: "Nouveau contact" }));
    const form = await screen.findByRole("dialog", { name: "Nouveau contact" });
    await fireEvent.input(within(form).getByLabelText("Adresse électronique 1"), {
      target: { value: "alice@example.com" },
    });
    await fireEvent.click(within(form).getByRole("button", { name: "Enregistrer" }));
    expect(await within(form).findByRole("alert")).toHaveTextContent(
      "Cette adresse appartient déjà à un autre contact. (« alice@example.com »)",
    );
    await fireEvent.click(within(form).getByRole("button", { name: "Annuler" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Nouveau contact" })).toBeNull(),
    );

    // Recherche au fil de la frappe.
    await fireEvent.input(screen.getByRole("searchbox", { name: "Rechercher un contact" }), {
      target: { value: "bo" },
    });
    await waitFor(() => expect(api.listContacts).toHaveBeenLastCalledWith("bo", false, null, 100));
    await waitFor(() => expect(screen.queryByRole("option", { name: /Alice/ })).toBeNull());
    await fireEvent.input(screen.getByRole("searchbox", { name: "Rechercher un contact" }), {
      target: { value: "" },
    });
    await waitFor(() => expect(api.listContacts).toHaveBeenLastCalledWith("", false, null, 100));
    expect(await screen.findByRole("option", { name: /Alice/ })).toBeInTheDocument();

    // Sélection multiple : barre, suppression groupée, désélection.
    await fireEvent.click(screen.getByRole("checkbox", { name: "Sélectionner Alice" }));
    await fireEvent.click(screen.getByRole("checkbox", { name: "Sélectionner Bob" }));
    expect(screen.getByText("2 contact(s) sélectionné(s)")).toBeInTheDocument();
    expect(screen.queryByRole("article")).toBeNull();
    await fireEvent.click(
      within(screen.getByRole("region", { name: "Contacts sélectionnés" })).getByRole("button", {
        name: "Supprimer le contact",
      }),
    );
    const confirm = await screen.findByRole("dialog", { name: "Supprimer les contacts" });
    expect(
      within(confirm).getByText("Supprimer 2 contacts du carnet d'adresses ?"),
    ).toBeInTheDocument();
    await fireEvent.click(within(confirm).getByRole("button", { name: "Annuler" }));
    await fireEvent.click(screen.getByRole("button", { name: "Désélectionner les contacts" }));
    expect(screen.queryByText("2 contact(s) sélectionné(s)")).toBeNull();
    expect(api.deleteContacts).not.toHaveBeenCalled();
  });
});
