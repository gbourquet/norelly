import { describe, expect, it, vi } from "vitest";

import { ChangePasswordForm, type ChangePasswordApi } from "./change-password.svelte";
import type { AddAccountResult } from "./ipc/api";

const added: AddAccountResult = {
  kind: "added",
  send_unchecked: false,
  account: {
    id: 4,
    address: "alice@gmail.com",
    display_name: "Alice",
    imap: { host: "imap.gmail.com", port: 993, security: "tls", username: "alice@gmail.com" },
    smtp: { host: "smtp.gmail.com", port: 587, security: "starttls", username: "alice@gmail.com" },
    oauth_provider: null,
  },
};

function fakeApi(result: AddAccountResult | Error): ChangePasswordApi {
  return {
    changePassword: vi.fn(async () => {
      if (result instanceof Error) throw result;
      return result;
    }),
    openExternal: vi.fn(async () => undefined),
  };
}

describe("changement du mot de passe d'un compte", () => {
  it("envoie le nouveau mot de passe du compte, puis l'efface", async () => {
    const api = fakeApi(added);
    const form = new ChangePasswordForm(api, 4);
    form.password = "abcd efgh ijkl mnop";
    expect(await form.submit()).toBe(true);
    expect(api.changePassword).toHaveBeenCalledWith(4, "abcd efgh ijkl mnop");
    expect(form.password).toBe("");
    expect(form.message).toBeNull();
    expect(form.busy).toBe(false);
  });

  it("mot de passe vide : rien n'est envoyé", async () => {
    const api = fakeApi(added);
    const form = new ChangePasswordForm(api, 4);
    expect(await form.submit()).toBe(false);
    expect(api.changePassword).not.toHaveBeenCalled();
    expect(form.message).toBe("Saisissez le nouveau mot de passe.");
  });

  it("compte Google refusé : aide au mot de passe d'application, champ vidé", async () => {
    const api = fakeApi({
      kind: "failed",
      protocol: "imap",
      failure: "google_app_password",
      detail: "Invalid credentials",
    });
    const form = new ChangePasswordForm(api, 4);
    form.password = "ancien";
    expect(await form.submit()).toBe(false);
    expect(form.appPasswordHelp).toBe(true);
    expect(form.password).toBe("");
    expect(form.message).toBe(
      "Serveur de réception (IMAP) : identifiants refusés : un compte Google demande un mot de passe d'application. (« Invalid credentials »)",
    );
  });

  it("autre refus : message habituel, sans aide Google", async () => {
    const api = fakeApi({
      kind: "failed",
      protocol: "smtp",
      failure: "authentication",
      detail: null,
    });
    const form = new ChangePasswordForm(api, 4);
    form.password = "faux";
    await form.submit();
    expect(form.appPasswordHelp).toBe(false);
    expect(form.message).toBe(
      "Serveur d'envoi (SMTP) : identifiants refusés. Vérifiez le nom d'utilisateur et le mot de passe (un mot de passe d'application est parfois nécessaire).",
    );
  });

  it("certificat refusé : connexion refusée par sécurité, rien n'est accepté ici", async () => {
    const api = fakeApi({
      kind: "certificate",
      problem: {
        protocol: "imap",
        host: "imap.gmail.com",
        fingerprint: "AA",
        issue: "unknown_issuer",
        fingerprint_changed: false,
        can_trust: true,
      },
    });
    const form = new ChangePasswordForm(api, 4);
    form.password = "abcd";
    expect(await form.submit()).toBe(false);
    expect(form.message).toBe("Serveur de réception (IMAP) : connexion refusée par sécurité.");
  });

  it("erreur de l'application : message lisible", async () => {
    const api = fakeApi(new Error("panne"));
    const form = new ChangePasswordForm(api, 4);
    form.password = "abcd";
    expect(await form.submit()).toBe(false);
    expect(form.message).not.toBeNull();
    expect(form.busy).toBe(false);
  });

  it("aide déjà affichée quand le compte est hors ligne pour ce motif", async () => {
    const api = fakeApi(added);
    const form = new ChangePasswordForm(api, 4, true);
    expect(form.appPasswordHelp).toBe(true);
    await form.openAppPasswords();
    expect(api.openExternal).toHaveBeenCalledWith("https://myaccount.google.com/apppasswords");
  });
});
