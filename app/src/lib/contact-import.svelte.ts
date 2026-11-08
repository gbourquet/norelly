// Import d'un carnet (runes Svelte 5) : aperçu d'un fichier choisi dans le
// sélecteur du système, choix du sort des doublons, confirmation. Fichier
// lu, fiches validées et doublons décidés par Rust (ADR-012) : l'interface
// n'affiche que les chiffres de l'aperçu.

import type {
  Api,
  ContactImportOutcomeDto,
  ContactImportPreviewDto,
  DuplicatePolicyDto,
} from "./ipc/api";
import { errorText } from "./errors";

export type ContactImportApi = Pick<Api, "previewContactImport" | "applyContactImport">;

export class ContactImportState {
  readonly #api: ContactImportApi;
  /** Aperçu en attente de confirmation. */
  preview = $state<ContactImportPreviewDto | null>(null);
  policy = $state<DuplicatePolicyDto>("skip");
  busy = $state(false);
  error = $state<string | null>(null);

  /** Bilan de la politique choisie. */
  readonly counts = $derived(this.preview === null ? null : this.preview[this.policy]);
  /** Le sort des doublons est à choisir : des fiches sont déjà dans le
   * carnet (ou en double dans le fichier) et pourraient le compléter. */
  readonly offersPolicy = $derived(
    this.preview !== null &&
      (this.preview.skip.duplicates > 0 || this.preview.complete.complete > 0),
  );
  /** Quelque chose à écrire avec la politique choisie. */
  readonly canImport = $derived(
    this.counts !== null && !this.busy && this.counts.create + this.counts.complete > 0,
  );

  constructor(api: ContactImportApi) {
    this.#api = api;
  }

  /** Ouvre le sélecteur ; vrai si un aperçu est à afficher, faux si
   * l'utilisateur annule. Un fichier refusé lève l'erreur (code traduit
   * par l'appelant). */
  async choose(title: string, filter: string): Promise<boolean> {
    const preview = await this.#api.previewContactImport(title, filter);
    this.policy = "skip";
    this.error = null;
    this.busy = false;
    this.preview = preview;
    return preview !== null;
  }

  /** Importe l'aperçu selon la politique choisie ; renvoie le bilan,
   * sinon `null` (erreur affichée, aperçu gardé pour réessayer). */
  async confirm(): Promise<ContactImportOutcomeDto | null> {
    const preview = this.preview;
    if (preview === null) return null;
    this.busy = true;
    this.error = null;
    try {
      const outcome = await this.#api.applyContactImport(preview.token, this.policy);
      this.preview = null;
      return outcome;
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
  }

  /** Oublie l'aperçu (dialogue fermé). */
  close(): void {
    this.preview = null;
    this.error = null;
    this.busy = false;
  }
}
