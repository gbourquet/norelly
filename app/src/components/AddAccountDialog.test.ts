import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import AddAccountDialog from "./AddAccountDialog.svelte";
import type { AccountApi, AccountDto, AddAccountResult, ServerDto } from "../lib/ipc/api";

const server = (host: string, port: number): ServerDto => ({
  host,
  port,
  security: "tls",
  username: "alice@example.com",
});
const account: AccountDto = {
  id: 1,
  address: "alice@example.com",
  display_name: "Alice",
  imap: server("imap.example.com", 993),
  smtp: server("smtp.example.com", 465),
  oauth_provider: null,
};

function fakeApi(results: AddAccountResult[]): AccountApi {
  return {
    listAccounts: vi.fn(async () => []),
    discoverSettings: vi.fn(async () => ({
      imap: {
        server: account.imap,
        source: "mozilla_ispdb" as const,
        requires_confirmation: false,
      },
      smtp: {
        server: account.smtp,
        source: "mozilla_ispdb" as const,
        requires_confirmation: false,
      },
    })),
    addAccount: vi.fn(async () => {
      const next = results.shift();
      if (next === undefined) throw new Error("appel inattendu");
      return next;
    }),
    trustCertificate: vi.fn(async () => undefined),
    removeAccount: vi.fn(),
    allowFileSecrets: vi.fn(async () => undefined),
    secretsBackend: vi.fn(async () => "system_keyring" as const),
    openExternal: vi.fn(async () => undefined),
  };
}

describe("fenêtre d'ajout de compte", () => {
  it("découvre, demande la confiance dans le certificat, puis ajoute", async () => {
    const api = fakeApi([
      {
        kind: "certificate",
        problem: {
          protocol: "imap",
          host: "imap.example.com",
          fingerprint: "AA:BB",
          issue: "unknown_issuer",
          fingerprint_changed: false,
          can_trust: true,
        },
      },
      { kind: "added", account },
    ]);
    const onAdded = vi.fn();
    render(AddAccountDialog, { open: true, api, onClose: vi.fn(), onAdded });
    await fireEvent.input(document.querySelector("#account-address") as HTMLInputElement, {
      target: { value: "alice@example.com" },
    });
    await fireEvent.input(document.querySelector("#account-password") as HTMLInputElement, {
      target: { value: "secret" },
    });
    // Découverte complète : la connexion est tentée aussitôt.
    await fireEvent.click(screen.getByRole("button", { name: "Continuer" }));
    expect(await screen.findByText(/AA:BB/)).toBeInTheDocument();
    await fireEvent.click(screen.getByRole("button", { name: "Faire confiance à ce serveur" }));
    await waitFor(() => expect(onAdded).toHaveBeenCalledWith(account));
    expect(api.trustCertificate).toHaveBeenCalledWith("imap.example.com", "AA:BB");
  });

  it("signale un compte déjà présent", async () => {
    const api = fakeApi([{ kind: "already_exists" }]);
    render(AddAccountDialog, { open: true, api, onClose: vi.fn(), onAdded: vi.fn() });
    await fireEvent.input(document.querySelector("#account-address") as HTMLInputElement, {
      target: { value: "alice@example.com" },
    });
    await fireEvent.input(document.querySelector("#account-password") as HTMLInputElement, {
      target: { value: "secret" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Continuer" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(api.addAccount).toHaveBeenCalledTimes(1);
  });

  it("compte Google refusé : étapes du mot de passe d'application et lien vers Google", async () => {
    const api = fakeApi([
      { kind: "failed", protocol: "imap", failure: "google_app_password", detail: null },
    ]);
    render(AddAccountDialog, { open: true, api, onClose: vi.fn(), onAdded: vi.fn() });
    await fireEvent.input(document.querySelector("#account-address") as HTMLInputElement, {
      target: { value: "alice@gmail.com" },
    });
    await fireEvent.input(document.querySelector("#account-password") as HTMLInputElement, {
      target: { value: "mot-de-passe-du-compte" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Continuer" }));
    const help = await screen.findByRole("region", {
      name: "Créer un mot de passe d'application Google",
    });
    expect(help.querySelectorAll("ol > li")).toHaveLength(3);
    // Le champ du mot de passe attend le nouveau code, vide.
    expect((document.querySelector("#account-password") as HTMLInputElement).value).toBe("");
    await fireEvent.click(screen.getByRole("button", { name: "Ouvrir la page Google" }));
    expect(api.openExternal).toHaveBeenCalledWith("https://myaccount.google.com/apppasswords");
  });

  it("fichier non chiffré : la garantie du trousseau n'est pas affichée", async () => {
    const api = fakeApi([]);
    vi.mocked(api.secretsBackend).mockResolvedValue("file");
    render(AddAccountDialog, { open: true, api, onClose: vi.fn(), onAdded: vi.fn() });
    await waitFor(() => expect(api.secretsBackend).toHaveBeenCalled());
    expect(screen.queryByText(/jamais dans les fichiers de Norelly/)).toBeNull();
  });

  it("trousseau du système : la garantie du trousseau est affichée", async () => {
    const api = fakeApi([]);
    render(AddAccountDialog, { open: true, api, onClose: vi.fn(), onAdded: vi.fn() });
    expect(await screen.findByText(/jamais dans les fichiers de Norelly/)).toBeInTheDocument();
  });
});
