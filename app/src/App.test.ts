import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import type { AccountDto } from "./lib/ipc/api";

const accounts: AccountDto[] = [];

vi.mock("./lib/ipc/api", () => ({
  IpcFailure: class extends Error {},
  api: {
    listAccounts: vi.fn(async () => structuredClone(accounts)),
    discoverSettings: vi.fn(),
    addAccount: vi.fn(),
    trustCertificate: vi.fn(),
    removeAccount: vi.fn(async (id: number) => {
      const i = accounts.findIndex((a) => a.id === id);
      if (i >= 0) accounts.splice(i, 1);
    }),
  },
}));

const { default: App } = await import("./App.svelte");
const { api } = await import("./lib/ipc/api");

const server = {
  host: "imap.example.com",
  port: 993,
  security: "tls" as const,
  username: "alice@example.com",
};

beforeEach(() => {
  accounts.splice(0, accounts.length);
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
});
