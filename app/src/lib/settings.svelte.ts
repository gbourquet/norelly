// Paramètres de l'application (runes Svelte 5) : lus et enregistrés par
// Rust, qui les revalide ; question de mise à jour au premier lancement.

import type { Api, SettingsDto } from "./ipc/api";
import { errorText } from "./errors";

export type SettingsApi = Pick<Api, "getSettings" | "saveSettings">;

export class SettingsState {
  readonly #api: SettingsApi;
  current = $state<SettingsDto | null>(null);
  error = $state<string | null>(null);

  /** La question des mises à jour n'a pas encore été posée. */
  /** Question fermée sans réponse pendant cette session. */
  #updateDismissed = $state(false);

  readonly askUpdate = $derived(
    this.current?.update_check === "not_asked" && !this.#updateDismissed,
  );

  constructor(api: SettingsApi) {
    this.#api = api;
  }

  async load(): Promise<void> {
    try {
      this.current = await this.#api.getSettings();
      this.error = null;
    } catch (e) {
      this.error = errorText(e);
    }
  }

  /** Enregistre ; renvoie vrai si c'est fait. */
  async save(next: SettingsDto): Promise<boolean> {
    try {
      await this.#api.saveSettings(next);
      this.current = next;
      this.error = null;
      return true;
    } catch (e) {
      this.error = errorText(e);
      return false;
    }
  }

  /** Question fermée sans réponse : masquée pour cette session. */
  dismissUpdate(): void {
    this.#updateDismissed = true;
  }

  /** Réponse à la question du premier lancement. */
  async answerUpdate(yes: boolean): Promise<void> {
    if (this.current === null) return;
    await this.save({ ...this.current, update_check: yes ? "enabled" : "disabled" });
  }
}
