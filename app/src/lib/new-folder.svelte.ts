// Création d'un dossier (runes Svelte 5) : nom saisi et emplacement
// choisis ici, nom complet décidé et revalidé par Rust.

import type { Api, FolderDto } from "./ipc/api";
import { errorText } from "./errors";

export type NewFolderApi = Pick<Api, "createFolder">;

/** Emplacement proposé pour le nouveau dossier (`null` : premier niveau). */
export interface ParentChoice {
  id: number | null;
  label: string;
  depth: number;
}

/** Emplacements : le premier niveau, puis les dossiers du compte dans
 * l'ordre de l'arborescence. */
export function parentChoices(
  folders: FolderDto[],
  topLevel: string,
  inbox: string,
): ParentChoice[] {
  return [
    { id: null, label: topLevel, depth: 0 },
    ...folders
      // Dossiers virtuels du serveur : rien ne peut y être rangé.
      .filter((f) => f.role !== "all" && f.role !== "flagged")
      .map((f) => ({
        id: f.id,
        label: f.role === "inbox" ? inbox : f.leaf,
        depth: f.depth + 1,
      })),
  ];
}

export class NewFolderState {
  readonly #api: NewFolderApi;
  name = $state("");
  parent = $state<number | null>(null);
  busy = $state(false);
  error = $state<string | null>(null);

  readonly canSubmit = $derived(this.name.trim() !== "" && !this.busy);

  constructor(api: NewFolderApi) {
    this.#api = api;
  }

  /** Formulaire vide, au premier niveau. */
  reset(): void {
    this.name = "";
    this.parent = null;
    this.error = null;
  }

  /** Crée le dossier ; renvoie son nom saisi si c'est fait, sinon `null`
   * (erreur affichée). */
  async submit(account: number): Promise<string | null> {
    if (!this.canSubmit) return null;
    this.busy = true;
    this.error = null;
    try {
      await this.#api.createFolder(account, this.name, this.parent);
      return this.name.trim();
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
  }
}
