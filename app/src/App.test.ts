import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import type { AccountDto } from "./lib/ipc/api";

const accounts: AccountDto[] = [];
const syncListeners: ((e: unknown) => void)[] = [];
const askedSettings = {
  language: "system" as "system" | "fr" | "en",
  update_check: "enabled" as "not_asked" | "enabled" | "disabled",
  notifications: true,
  notification_privacy: false,
  attachment_threshold: 2 * 1024 * 1024,
};
let storedSettings = { ...askedSettings };

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
    discardDraft: vi.fn(),
    suggestContacts: vi.fn(async () => []),
    pickAttachments: vi.fn(async () => []),
    attachDropped: vi.fn(async () => []),
    outbox: vi.fn(async () => []),
    retrySend: vi.fn(),
    cancelSend: vi.fn(),
    searchMessages: vi.fn(async () => ({ items: [], next: null })),
  },
}));

const { default: App } = await import("./App.svelte");
const { api } = await import("./lib/ipc/api");
const { app } = await import("./lib/app.svelte");

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
  syncListeners.splice(0, syncListeners.length);
  storedSettings = { ...askedSettings };
  vi.clearAllMocks();
});

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
    expect(within(help).getByText("Palette de commandes")).toBeInTheDocument();
  });

  it("liste les comptes et supprime après confirmation", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
    });
    render(App);
    expect(await screen.findByText("Alice")).toBeInTheDocument();
    await fireEvent.click(
      screen.getByRole("button", { name: "Supprimer le compte alice@example.com" }),
    );
    const dialog = await screen.findByRole("dialog", { name: "Supprimer le compte ?" });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Supprimer le compte" }));
    await waitFor(() => expect(api.removeAccount).toHaveBeenCalledWith(7));
    expect(await screen.findByText("Aucun compte pour l'instant.")).toBeInTheDocument();
  });

  it("annuler la suppression ne supprime rien", async () => {
    accounts.push({
      id: 3,
      address: "bob@example.org",
      display_name: "Bob",
      imap: server,
      smtp: server,
    });
    render(App);
    await fireEvent.click(
      await screen.findByRole("button", { name: "Supprimer le compte bob@example.org" }),
    );
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
    } as AccountDto);
    render(App);
    await screen.findAllByText("alice@example.com");
    await fireEvent.keyDown(window, { key: "c" });
    const dialog = await screen.findByRole("dialog", { name: "Nouveau message" });
    expect(api.prepareCompose).toHaveBeenCalledWith(7, "new", null, "");
    const to = within(dialog).getByRole("combobox", { name: "À" });
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
    } as AccountDto);
    const found = (n: number, subject: string) => ({
      message: n,
      conversation: 100 + n,
      account: 7,
      mailbox: 30 + n,
      date: 1_700_000_000,
      from: { name: "Bob", address: "bob@example.com" },
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

  it("donne un nom accessible à chaque contrôle (passe d'accessibilité)", async () => {
    accounts.push({
      id: 7,
      address: "alice@example.com",
      display_name: "Alice",
      imap: server,
      smtp: server,
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
});
