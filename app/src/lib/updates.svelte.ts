// Mises à jour de l'application (runes Svelte 5) : recherche, vérification
// de la signature et installation sont faites par Rust (installateur NSIS
// seulement, ADR-006) ; ici, seulement l'annonce et le bouton.

import type { Api, UpdateEventDto } from "./ipc/api";
import { errorText } from "./errors";

export type UpdatesApi = Pick<Api, "updateStatus" | "installUpdate">;

export class UpdatesState {
  readonly #api: UpdatesApi;
  supported = $state(false);
  available = $state<string | null>(null);
  installing = $state(false);
  error = $state<string | null>(null);

  constructor(api: UpdatesApi) {
    this.#api = api;
  }

  async load(): Promise<void> {
    const status = await this.#api.updateStatus();
    this.supported = status.supported;
    this.available = status.available;
  }

  /** Mise à jour trouvée par la recherche périodique. */
  handle(event: UpdateEventDto): void {
    if (event.kind === "available") this.available = event.version;
  }

  async install(): Promise<void> {
    if (this.installing) return;
    this.installing = true;
    this.error = null;
    try {
      await this.#api.installUpdate();
    } catch (e) {
      this.error = errorText(e);
    } finally {
      this.installing = false;
    }
  }

  /** Bandeau fermé : la mise à jour sera reproposée au lancement suivant. */
  dismiss(): void {
    this.available = null;
    this.error = null;
  }
}
