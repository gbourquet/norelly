// Doublons du carnet (runes Svelte 5) : groupes trouvés par Rust,
// présentés un par un ; l'utilisateur choisit la fiche gardée et les
// fiches à y réunir, puis fusionne ou passe. La fusion est recalculée par
// Rust d'après les fiches enregistrées (ADR-012).

import type { Api, ContactDto, ContactDuplicateGroupDto } from "./ipc/api";
import { errorText } from "./errors";

export type ContactDuplicatesApi = Pick<Api, "findContactDuplicates" | "mergeContacts">;

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

  readonly current = $derived<ContactDuplicateGroupDto | null>(this.groups[this.index] ?? null);
  /** « Groupe `position` sur `total` ». */
  readonly position = $derived(this.index + 1);
  readonly total = $derived(this.groups.length);
  /** Recherche faite et plus rien à examiner. */
  readonly finished = $derived(!this.loading && this.error === null && this.current === null);
  readonly canMerge = $derived(!this.busy && this.keep !== null && this.checked.length >= 2);

  constructor(api: ContactDuplicatesApi) {
    this.#api = api;
  }

  /** Relance la recherche et présente le premier groupe. */
  async open(): Promise<void> {
    this.close();
    this.loading = true;
    try {
      this.groups = await this.#api.findContactDuplicates();
      this.#present();
    } catch (e) {
      this.error = errorText(e);
    } finally {
      this.loading = false;
    }
  }

  isChecked(id: number): boolean {
    return this.checked.includes(id);
  }

  /** Coche ou décoche une fiche ; la fiche gardée reste cochée. */
  toggle(id: number): void {
    if (id === this.keep) return;
    this.checked = this.isChecked(id)
      ? this.checked.filter((c) => c !== id)
      : [...this.checked, id];
  }

  /** Garde cette fiche (cochée d'office). */
  choose(id: number): void {
    this.keep = id;
    if (!this.isChecked(id)) this.checked = [...this.checked, id];
  }

  /** Fusionne les fiches cochées dans la fiche gardée ; renvoie la fiche
   * gardée, sinon `null` (message affiché, groupe gardé). */
  async merge(): Promise<ContactDto | null> {
    const group = this.current;
    const keep = this.keep;
    if (group === null || keep === null || !this.canMerge) return null;
    const others = group.contacts
      .map((c) => c.id)
      .filter((id) => id !== keep && this.isChecked(id));
    this.busy = true;
    this.error = null;
    try {
      const kept = await this.#api.mergeContacts(keep, others);
      this.groups = this.groups.filter((g) => g !== group);
      this.merged += 1;
      this.#present();
      return kept;
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
  }

  /** Passe au groupe suivant sans rien écrire. */
  skip(): void {
    if (this.current === null || this.busy) return;
    this.index += 1;
    this.error = null;
    this.#present();
  }

  /** Fenêtre fermée : tout est oublié. */
  close(): void {
    this.groups = [];
    this.index = 0;
    this.keep = null;
    this.checked = [];
    this.loading = false;
    this.busy = false;
    this.error = null;
    this.merged = 0;
  }

  /** Présente le groupe `index` : toutes ses fiches cochées, la première
   * (la plus complète) gardée. */
  #present(): void {
    const group = this.groups[this.index];
    this.keep = group?.contacts[0]?.id ?? null;
    this.checked = group?.contacts.map((c) => c.id) ?? [];
  }
}
