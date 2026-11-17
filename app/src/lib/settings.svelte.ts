// Paramètres de l'application (runes Svelte 5) : lus et enregistrés par
// Rust, qui les revalide ; question de mise à jour au premier lancement ;
// délai du marquage comme lu.

import type { Api, SettingsDto } from "./ipc/api";
import { errorText } from "./errors";

export type SettingsApi = Pick<Api, "getSettings" | "saveSettings">;

/** Délai du marquage comme lu (millisecondes) ; `null` : réglages pas
 * encore chargés, ou « jamais ». */
export function markSeenDelayMs(settings: SettingsDto | null): number | null {
  const secs = settings?.mark_seen_delay_secs ?? null;
  return secs === null ? null : secs * 1000;
}

/** Délais proposés (secondes), « jamais » en dernier. */
const MARK_SEEN_DELAYS = [0, 2, 5, 10, 30];

/** Choix du délai du marquage comme lu (secondes ; `null` : jamais) ;
 * un délai enregistré hors de la liste y est ajouté, pour rester affiché. */
export function markSeenChoices(current: number | null): (number | null)[] {
  const delays =
    current === null || MARK_SEEN_DELAYS.includes(current)
      ? MARK_SEEN_DELAYS
      : [...MARK_SEEN_DELAYS, current].sort((a, b) => a - b);
  return [...delays, null];
}

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
