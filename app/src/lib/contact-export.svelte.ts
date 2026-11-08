// Export du carnet (runes Svelte 5) : choix du format dans une fenêtre,
// puis sélecteur d'enregistrement du système. Fiches préparées, fichier
// produit et écrit par Rust (ADR-012) : l'interface ne donne jamais de
// chemin.

import type {
  Api,
  ContactExportFormatDto,
  ContactExportOutcomeDto,
  ContactExportSelectionDto,
} from "./ipc/api";
import { errorText } from "./errors";

export type ContactExportApi = Pick<Api, "exportContacts">;

/** Libellés du sélecteur d'enregistrement pour un format. */
export type SaveLabels = (format: ContactExportFormatDto) => { title: string; filter: string };

export class ContactExportState {
  readonly #api: ContactExportApi;
  selection = $state<ContactExportSelectionDto | null>(null);
  format = $state<ContactExportFormatDto>("vcard4");
  busy = $state(false);
  error = $state<string | null>(null);
  /** Contacts choisis, `null` pour tout le carnet. */
  readonly count = $derived(this.selection?.kind === "contacts" ? this.selection.ids.length : null);

  constructor(api: ContactExportApi) {
    this.#api = api;
  }

  /** Fenêtre ouverte sur `selection` ; le format est celui de la fois
   * précédente. */
  open(selection: ContactExportSelectionDto): void {
    this.selection = selection;
    this.error = null;
    this.busy = false;
  }

  /** Ouvre le sélecteur d'enregistrement et écrit le fichier ; renvoie le
   * bilan, sinon `null` : enregistrement annulé (fenêtre gardée) ou échec
   * (message affiché). */
  async confirm(labels: SaveLabels): Promise<ContactExportOutcomeDto | null> {
    const selection = this.selection;
    if (selection === null) return null;
    this.busy = true;
    this.error = null;
    try {
      const { title, filter } = labels(this.format);
      const outcome = await this.#api.exportContacts(selection, this.format, title, filter);
      if (outcome !== null) this.selection = null;
      return outcome;
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
  }

  /** Fenêtre fermée. */
  close(): void {
    this.selection = null;
    this.error = null;
    this.busy = false;
  }
}
