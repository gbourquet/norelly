// Création d'un dossier (runes Svelte 5) : nom saisi et emplacement
// choisis ici, nom complet décidé et revalidé par Rust.

import type { Api, FolderDto } from "./ipc/api";
import { errorText } from "./errors";
import { folderLabel } from "./folder-label";

export type NewFolderApi = Pick<Api, "createFolder">;

/** Emplacement proposé pour le nouveau dossier (`null` : premier niveau). */
export interface ParentChoice {
  id: number | null;
  label: string;
  depth: number;
}

/** Faux si `folder` est un dossier virtuel du serveur (tous les messages,
 * suivis) : rien ne peut y être rangé (Rust le refuse aussi). */
export function canHoldFolders(folder: FolderDto): boolean {
  return folder.role !== "all" && folder.role !== "flagged";
}

/** Dossier créé : nom saisi et identifiant local (`null` si la liste
 * n'est pas encore relue). */
export interface CreatedFolder {
  id: number | null;
  name: string;
}

/** Emplacement et nom proposés à l'ouverture du formulaire. */
export interface NewFolderPreset {
  parent?: number | null;
  name?: string;
}

/** Emplacements : le premier niveau, puis les dossiers du compte dans
 * l'ordre de l'arborescence, avec le retrait de l'arbre affiché. */
export function parentChoices(folders: FolderDto[], topLevel: string): ParentChoice[] {
  return [
    { id: null, label: topLevel, depth: 0 },
    ...folders.filter(canHoldFolders).map((f) => ({
      id: f.id,
      label: folderLabel(f),
      depth: f.indent + 1,
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

  /** Formulaire vide, au premier niveau, sauf emplacement ou nom
   * proposés. */
  reset(preset: NewFolderPreset = {}): void {
    this.name = preset.name ?? "";
    this.parent = preset.parent ?? null;
    this.error = null;
  }

  /** Crée le dossier ; renvoie son nom saisi et son identifiant si c'est
   * fait, sinon `null` (erreur affichée). */
  async submit(account: number): Promise<CreatedFolder | null> {
    if (!this.canSubmit) return null;
    this.busy = true;
    this.error = null;
    try {
      const id = await this.#api.createFolder(account, this.name, this.parent);
      return { id, name: this.name.trim() };
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
  }
}
