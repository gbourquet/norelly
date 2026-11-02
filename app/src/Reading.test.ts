import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import type { AccountDto, ConversationDto, MessageViewDto } from "./lib/ipc/api";

const account: AccountDto = {
  id: 1,
  address: "alice@example.com",
  display_name: "Alice",
  imap: { host: "imap.example.com", port: 993, security: "tls", username: "alice" },
  smtp: { host: "smtp.example.com", port: 465, security: "tls", username: "alice" },
};

function conversation(id: number): ConversationDto {
  return {
    id,
    account: 1,
    mailbox: 5,
    date: 1_700_000_000 - id,
    subject: `Sujet ${id}`,
    senders: [{ name: `Expéditeur ${id}`, address: `e${id}@example.com` }],
    messages: 1,
    unread: id === 1 ? 1 : 0,
    flagged: false,
    has_attachments: id === 1,
  };
}

const view: MessageViewDto = {
  id: 0,
  mode: "html",
  notice: null,
  has_html: true,
  blocked_images: 2,
  attachments: [
    {
      part: "2",
      name: "rapport.pdf",
      media_type: "application/pdf",
      size: 2048,
      available: true,
      can_open: true,
    },
    {
      part: "3",
      name: "outil.exe",
      media_type: "application/x-msdownload",
      size: 4096,
      available: false,
      can_open: false,
    },
  ],
};

vi.mock("./lib/ipc/api", () => ({
  IpcFailure: class extends Error {},
  api: {
    listAccounts: vi.fn(async () => [structuredClone(account)]),
    discoverSettings: vi.fn(),
    addAccount: vi.fn(),
    trustCertificate: vi.fn(),
    allowFileSecrets: vi.fn(),
    removeAccount: vi.fn(),
    listFolders: vi.fn(async () => [
      {
        id: 5,
        name: "INBOX",
        leaf: "INBOX",
        depth: 0,
        role: "inbox",
        selectable: true,
        total: 3,
        unread: 1,
      },
    ]),
    listConversations: vi.fn(async () => ({
      items: [conversation(1), conversation(2), conversation(3)],
      next: null,
    })),
    conversationMessages: vi.fn(async (id: number) => [
      {
        id: id * 10,
        account: 1,
        mailbox: 5,
        date: 1_700_000_000,
        from: { name: `Expéditeur ${id}`, address: `e${id}@example.com` },
        to: [{ name: "Alice", address: "alice@example.com" }],
        cc: [],
        subject: `Sujet ${id}`,
        unread: false,
        flagged: false,
        has_attachments: false,
      },
    ]),
    openMessage: vi.fn(async (id: number) => ({ ...structuredClone(view), id })),
    allowRemoteImages: vi.fn(async () => undefined),
    readingPreferences: vi.fn(async () => ({ prefer_plain_text: false })),
    setPreferPlainText: vi.fn(async () => undefined),
    syncAccount: vi.fn(),
    openExternal: vi.fn(async () => undefined),
    saveAttachment: vi.fn(async () => "rapport.pdf"),
    openAttachment: vi.fn(async () => undefined),
    onSyncEvent: vi.fn(async () => () => undefined),
    performAction: vi.fn(async (ids: number[]) => ({
      ops: ids.map((id) => id + 100),
      messages: ids.length,
    })),
    undoActions: vi.fn(async (ops: number[]) => ops.length),
    setArchiveFolder: vi.fn(async () => undefined),
  },
}));

const { default: App } = await import("./App.svelte");
const { api } = await import("./lib/ipc/api");
const { mail } = await import("./lib/app.svelte");

beforeEach(() => {
  vi.clearAllMocks();
  mail.back();
  // État partagé entre les tests (module unique) : sélection remise au début.
  mail.selectedIndex = 0;
  mail.marked = [];
  mail.lastAction = null;
  mail.notice = null;
});

describe("lecture", () => {
  it("liste la boîte unifiée, navigue avec j/k et ouvre avec Entrée", async () => {
    render(App);
    const list = await screen.findByRole("listbox", { name: "Conversations" });
    expect(await within(list).findByText("Sujet 1")).toBeInTheDocument();
    expect(api.listConversations).toHaveBeenCalledWith(
      { kind: "unified", role: "inbox" },
      null,
      100,
    );
    expect(screen.getByRole("heading", { name: "Boîte de réception unifiée" })).toBeInTheDocument();

    await fireEvent.keyDown(window, { key: "j" });
    await waitFor(() =>
      expect(within(list).getByRole("option", { selected: true })).toHaveTextContent("Sujet 2"),
    );
    await fireEvent.keyDown(window, { key: "k" });
    await fireEvent.keyDown(window, { key: "Enter" });

    const reader = await screen.findByRole("article", { name: "Conversation" });
    expect(within(reader).getByRole("heading", { name: "Sujet 1" })).toBeInTheDocument();
    const frame = await within(reader).findByTitle("Contenu du message");
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(frame.getAttribute("src")).toMatch(/^mailview:\/\/localhost\/10\?v=\d+$/);
    expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");

    // Échap : retour à la liste.
    await fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("article")).toBeNull());
  });

  it("bannière d'images bloquées, pièces jointes et messages d'agent validés", async () => {
    render(App);
    // Chargement initial terminé (l'état de lecture est partagé entre tests).
    await waitFor(() => expect(api.listConversations).toHaveBeenCalled());
    await screen.findByText("Sujet 1");
    await fireEvent.keyDown(window, { key: "Enter" });
    const banner = await screen.findByText(/Images distantes bloquées \(2\)/);
    expect(banner).toBeInTheDocument();
    await fireEvent.click(
      screen.getByRole("button", { name: "Afficher les images de cet expéditeur" }),
    );
    await waitFor(() => expect(api.allowRemoteImages).toHaveBeenCalledWith(10));

    const attachments = screen.getByRole("list", { name: "Pièces jointes" });
    const items = within(attachments).getAllByRole("listitem");
    expect(
      within(items[0] as HTMLElement).getByRole("button", { name: "Ouvrir" }),
    ).toBeInTheDocument();
    // Exécutable : enregistrement seulement.
    expect(within(items[1] as HTMLElement).queryByRole("button", { name: "Ouvrir" })).toBeNull();
    await fireEvent.click(
      within(items[0] as HTMLElement).getByRole("button", { name: "Enregistrer…" }),
    );
    expect(await screen.findByText("Enregistré sous « rapport.pdf ».")).toBeInTheDocument();
    expect(api.saveAttachment).toHaveBeenCalledWith(10, "2", "Enregistrer la pièce jointe dans…");

    // Un message qui ne vient pas de l'iframe du message est ignoré.
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "norelly:link", href: "https://exemple.fr" },
        origin: "null",
        source: window,
      }),
    );
    await new Promise((r) => setTimeout(r, 0));
    expect(api.openExternal).not.toHaveBeenCalled();
  });

  it("actions au clavier : e, s, x, puis annulation par z", async () => {
    render(App);
    const list = await screen.findByRole("listbox", { name: "Conversations" });
    await within(list).findByText("Sujet 1");
    await fireEvent.keyDown(window, { key: "e" });
    await waitFor(() =>
      expect(api.performAction).toHaveBeenCalledWith(
        [1],
        { kind: "unified", role: "inbox" },
        { kind: "archive" },
      ),
    );
    const status = await screen.findByText("Archivé.");
    expect(status.closest("[role=status]")).not.toBeNull();
    await fireEvent.keyDown(window, { key: "z" });
    await waitFor(() => expect(api.undoActions).toHaveBeenCalledWith([101]));
    expect(await screen.findByText("Action annulée.")).toBeInTheDocument();

    // s : suivre ; Maj+I : marquer comme lu.
    await fireEvent.keyDown(window, { key: "s" });
    await waitFor(() =>
      expect(api.performAction).toHaveBeenLastCalledWith([1], expect.anything(), {
        kind: "mark_flagged",
        value: true,
      }),
    );
    await fireEvent.keyDown(window, { key: "I", shiftKey: true });
    await waitFor(() =>
      expect(api.performAction).toHaveBeenLastCalledWith([1], expect.anything(), {
        kind: "mark_seen",
        value: true,
      }),
    );

    // x : sélection multiple, puis # sur les conversations cochées.
    await fireEvent.keyDown(window, { key: "x" });
    await fireEvent.keyDown(window, { key: "j" });
    await fireEvent.keyDown(window, { key: "x" });
    expect(await screen.findByText("2 sélectionnée(s)")).toBeInTheDocument();
    await fireEvent.keyDown(window, { key: "#", shiftKey: true });
    await waitFor(() =>
      expect(api.performAction).toHaveBeenLastCalledWith([1, 2], expect.anything(), {
        kind: "delete",
      }),
    );
  });

  it("v ouvre le choix du dossier et déplace", async () => {
    vi.mocked(api.listFolders).mockResolvedValue([
      {
        id: 5,
        name: "INBOX",
        leaf: "INBOX",
        depth: 0,
        role: "inbox",
        selectable: true,
        total: 3,
        unread: 1,
      },
      {
        id: 7,
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
    const list = await screen.findByRole("listbox", { name: "Conversations" });
    await within(list).findByText("Sujet 1");
    await fireEvent.keyDown(window, { key: "v" });
    const picker = await screen.findByRole("dialog", { name: "Déplacer vers…" });
    // Saisie au clavier, flèche puis Entrée (le dossier où se trouve déjà
    // la conversation est exclu).
    expect(within(picker).queryByText("INBOX")).toBeNull();
    const input = within(picker).getByPlaceholderText("Rechercher un dossier…");
    await fireEvent.input(input, { target: { value: "proj" } });
    await within(picker).findByText("Projets");
    await fireEvent.keyDown(input, { key: "ArrowDown" });
    await fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() =>
      expect(api.performAction).toHaveBeenLastCalledWith([1], expect.anything(), {
        kind: "move",
        mailbox: 7,
      }),
    );
    expect(await screen.findByText("Déplacé vers « Projets ».")).toBeInTheDocument();
  });

  it("actions à la souris : barre d'outils, boutons de ligne, menu contextuel", async () => {
    render(App);
    const list = await screen.findByRole("listbox", { name: "Conversations" });
    await within(list).findByText("Sujet 1");

    // Bouton de ligne : vise cette ligne, sans ouvrir la conversation.
    const row3 = within(list).getByText("Sujet 3").closest("[role=option]") as HTMLElement;
    // Visibles au survol seulement (souris) : hors de l'arbre accessible.
    await fireEvent.click(
      await within(row3).findByRole("button", { name: "Archiver", hidden: true }),
    );
    await waitFor(() =>
      expect(api.performAction).toHaveBeenLastCalledWith([3], expect.anything(), {
        kind: "archive",
      }),
    );
    expect(screen.queryByRole("article", { name: "Conversation" })).toBeNull();
    // Bascule selon l'état : la conversation 1 est non lue.
    const row1 = within(list).getByText("Sujet 1").closest("[role=option]") as HTMLElement;
    expect(
      within(row1).getByRole("button", { name: "Marquer comme lu", hidden: true }),
    ).toBeInTheDocument();

    // Menu contextuel : clic droit sur une ligne.
    const row2 = within(list).getByText("Sujet 2").closest("[role=option]") as HTMLElement;
    await fireEvent.contextMenu(row2);
    const menu = await screen.findByRole("menu", { name: "Actions sur la conversation" });
    expect(within(menu).getByText("Ouvrir la conversation")).toBeInTheDocument();
    expect(within(menu).getByText("Déplacer vers…")).toBeInTheDocument();
    await fireEvent.click(within(menu).getByText("Supprimer"));
    await waitFor(() =>
      expect(api.performAction).toHaveBeenLastCalledWith([2], expect.anything(), {
        kind: "delete",
      }),
    );

    // Barre d'outils de la vue de lecture.
    await fireEvent.click(within(row1).getByText("Sujet 1"));
    const reader = await screen.findByRole("article", { name: "Conversation" });
    const toolbar = within(reader).getByRole("toolbar", { name: "Actions sur la conversation" });
    const flag = within(toolbar).getByRole("button", { name: "Suivre (ou ne plus suivre)" });
    expect(flag).toHaveAttribute("title", "Suivre (ou ne plus suivre) (S)");
    await fireEvent.click(flag);
    await waitFor(() =>
      expect(api.performAction).toHaveBeenLastCalledWith([1], expect.anything(), {
        kind: "mark_flagged",
        value: true,
      }),
    );
  });
});
