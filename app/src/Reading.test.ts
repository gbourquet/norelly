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
  },
}));

const { default: App } = await import("./App.svelte");
const { api } = await import("./lib/ipc/api");
const { mail } = await import("./lib/app.svelte");

beforeEach(() => {
  vi.clearAllMocks();
  mail.back();
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
    expect(api.saveAttachment).toHaveBeenCalledWith(10, "2");

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
});
