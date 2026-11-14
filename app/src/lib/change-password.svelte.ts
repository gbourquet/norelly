// Changement du mot de passe d'un compte enregistré (mot de passe
// d'application Google révoqué, mot de passe changé chez le fournisseur).
// Rust essaie le nouveau mot de passe sur les deux serveurs avant de le
// garder ; ce contrôleur ne fait que relayer la saisie et la réponse.

import type { Api } from "./ipc/api";
import { m } from "./paraglide/messages.js";
import { errorText } from "./errors";
import { GOOGLE_APP_PASSWORDS_URL, failureMessage } from "./add-account.svelte";

export type ChangePasswordApi = Pick<Api, "changePassword" | "openExternal">;

export class ChangePasswordForm {
  password = $state("");
  busy = $state(false);
  message = $state<string | null>(null);
  /** Étapes du mot de passe d'application Google, décidées par Rust. */
  appPasswordHelp = $state(false);
  #api: ChangePasswordApi;
  #account: number;

  constructor(api: ChangePasswordApi, account: number, appPasswordHelp = false) {
    this.#api = api;
    this.#account = account;
    this.appPasswordHelp = appPasswordHelp;
  }

  /** Vrai si le mot de passe a été changé. */
  async submit(): Promise<boolean> {
    if (this.password === "") {
      this.message = m.password_msg_missing();
      return false;
    }
    this.busy = true;
    this.message = null;
    try {
      const result = await this.#api.changePassword(this.#account, this.password);
      switch (result.kind) {
        case "added":
          this.password = "";
          return true;
        case "failed":
          if (result.failure === "google_app_password") {
            this.appPasswordHelp = true;
            this.password = "";
          }
          this.message = failureMessage(result.failure, result.protocol, result.detail, "");
          return false;
        case "certificate":
          // Un certificat ne s'accepte qu'à l'ajout du compte, écran
          // d'alerte à l'appui : ici, la connexion est simplement refusée.
          this.message = failureMessage("security", result.problem.protocol, null, "");
          return false;
        default:
          this.message = failureMessage("internal", null, null, "");
          return false;
      }
    } catch (e) {
      this.message = errorText(e);
      return false;
    } finally {
      this.busy = false;
    }
  }

  /** Ouvre la page Google de création des mots de passe d'application. */
  async openAppPasswords(): Promise<void> {
    try {
      await this.#api.openExternal(GOOGLE_APP_PASSWORDS_URL);
    } catch (e) {
      this.message = errorText(e);
    }
  }
}
