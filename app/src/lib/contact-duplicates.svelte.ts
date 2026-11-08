// Doublons du carnet (runes Svelte 5) : groupes trouvés par Rust,
// présentés un par un ; l'utilisateur choisit la fiche gardée et les
// fiches à y réunir, voit ce que gardera la fiche (aperçu calculé par
// Rust), puis fusionne ou passe. La fusion est recalculée par Rust
// d'après les fiches enregistrées (ADR-012).

import type { Api, ContactDto, ContactDuplicateGroupDto, ContactMergePreviewDto } from "./ipc/api";
import { errorText } from "./errors";

export type ContactDuplicatesApi = Pick<
  Api,
  "findContactDuplicates" | "previewContactMerge" | "mergeContacts"
>;

export class ContactDuplicatesState {
  readonly #api: ContactDuplicatesApi;
  /** Groupes qui restent à examiner (un groupe fusionné en sort). */
  groups = $state<ContactDuplicateGroupDto[]>([]);
  /** Rang du groupe présenté parmi `groups`. */
  index = $state(0);
  /** Fiche gardée du groupe présenté. */
  keep = $state<number | null>(null);
  /** Fiches cochées du groupe présenté (la fiche gardée toujours). */
  checked = $state<number[]>([]);
  loading = $state(false);
  busy = $state(false);
  error = $state<string | null>(null);
  /** Fusions faites depuis l'ouverture. */
  merged = $state(0);
  /** Ce que garderait la fiche pour la sélection présentée. */
  preview = $state<ContactMergePreviewDto | null>(null);
  previewError = $state<string | null>(null);
  /** Numéro de la dernière demande d'aperçu : une réponse plus ancienne
   * est ignorée. */
  #request = 0;

  readonly current = $derived<ContactDuplicateGroupDto | null>(this.groups[this.index] ?? null);
  /** « Groupe `position` sur `total` ». */
  readonly position = $derived(this.index + 1);
  readonly total = $derived(this.groups.length);
  /** Recherche faite et plus rien à examiner. */
  readonly finished = $derived(!this.loading && this.error === null && this.current === null);
  readonly canMerge = $derived(
    !this.busy && this.keep !== null && this.checked.length >= 2 && this.preview !== null,
  );

  constructor(api: ContactDuplicatesApi) {
    this.#api = api;
  }

  /** Relance la recherche et présente le premier groupe. */
  async open(): Promise<void> {
    this.close();
    this.loading = true;
    try {
      this.groups = await this.#api.findContactDuplicates();
    } catch (e) {
      this.error = errorText(e);
      return;
    } finally {
      this.loading = false;
    }
    await this.#present();
  }

  isChecked(id: number): boolean {
    return this.checked.includes(id);
  }

  /** Coche ou décoche une fiche ; la fiche gardée reste cochée. */
  async toggle(id: number): Promise<void> {
    if (id === this.keep) return;
    this.checked = this.isChecked(id)
      ? this.checked.filter((c) => c !== id)
      : [...this.checked, id];
    await this.#refresh();
  }

  /** Garde cette fiche (cochée d'office). */
  async choose(id: number): Promise<void> {
    this.keep = id;
    if (!this.isChecked(id)) this.checked = [...this.checked, id];
    await this.#refresh();
  }

  /** Fusionne les fiches cochées dans la fiche gardée ; renvoie la fiche
   * gardée, sinon `null` (message affiché, groupe gardé). */
  async merge(): Promise<ContactDto | null> {
    const group = this.current;
    const keep = this.keep;
    if (group === null || keep === null || !this.canMerge) return null;
    this.busy = true;
    this.error = null;
    let kept: ContactDto;
    try {
      kept = await this.#api.mergeContacts(keep, this.#others());
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
    this.groups = this.groups.filter((g) => g !== group);
    this.merged += 1;
    await this.#present();
    return kept;
  }

  /** Passe au groupe suivant sans rien écrire. */
  async skip(): Promise<void> {
    if (this.current === null || this.busy) return;
    this.index += 1;
    this.error = null;
    await this.#present();
  }

  /** Fenêtre fermée : tout est oublié. */
  close(): void {
    this.#request += 1;
    this.groups = [];
    this.index = 0;
    this.keep = null;
    this.checked = [];
    this.loading = false;
    this.busy = false;
    this.error = null;
    this.merged = 0;
    this.preview = null;
    this.previewError = null;
  }

  /** Fiches cochées autres que la fiche gardée, dans l'ordre du groupe. */
  #others(): number[] {
    return (this.current?.contacts ?? [])
      .map((c) => c.id)
      .filter((id) => id !== this.keep && this.isChecked(id));
  }

  /** Présente le groupe `index` : toutes ses fiches cochées, la première
   * (la plus complète) gardée. */
  async #present(): Promise<void> {
    const group = this.groups[this.index];
    this.keep = group?.contacts[0]?.id ?? null;
    this.checked = group?.contacts.map((c) => c.id) ?? [];
    await this.#refresh();
  }

  /** Aperçu de la sélection présentée ; aucun sans autre fiche cochée. */
  async #refresh(): Promise<void> {
    const request = ++this.#request;
    const keep = this.keep;
    const others = this.#others();
    this.preview = null;
    this.previewError = null;
    if (keep === null || others.length === 0) return;
    try {
      const preview = await this.#api.previewContactMerge(keep, others);
      if (request === this.#request) this.preview = preview;
    } catch (e) {
      if (request === this.#request) this.previewError = errorText(e);
    }
  }
}
