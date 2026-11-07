// Renommage et suppression d'un dossier (runes Svelte 5) : saisie et
// confirmation ici ; la décision (dossier protégé, nom, sous-dossiers) est
// prise et revalidée par Rust.

import type { Api, FolderDto } from "./ipc/api";
import { errorText } from "./errors";

export type FolderEditApi = Pick<Api, "renameFolder" | "deleteFolder">;

/** Dossier ordinaire : la boîte de réception et les dossiers spéciaux ne
 * proposent ni renommage ni suppression (Rust les refuse aussi). */
export function isEditableFolder(folder: FolderDto): boolean {
  return folder.role === "normal";
}

/** Dossier renommé : nouveau nom saisi et identifiant local sous ce nom
 * (`null` si la liste n'est pas encore relue). */
export interface RenamedFolder {
  id: number | null;
  name: string;
}

export class RenameFolderState {
  readonly #api: FolderEditApi;
  /** Nom actuel du dossier (dernier segment). */
  #current = $state("");
  name = $state("");
  busy = $state(false);
  error = $state<string | null>(null);

  readonly canSubmit = $derived(
    this.name.trim() !== "" && this.name.trim() !== this.#current && !this.busy,
  );

  constructor(api: FolderEditApi) {
    this.#api = api;
  }

  /** Formulaire prérempli avec le nom actuel du dossier. */
  reset(folder: FolderDto): void {
    this.#current = folder.leaf;
    this.name = folder.leaf;
    this.error = null;
  }

  /** Renomme le dossier ; `null` si rien n'est fait (erreur affichée). */
  async submit(account: number, folder: FolderDto): Promise<RenamedFolder | null> {
    if (!this.canSubmit) return null;
    this.busy = true;
    this.error = null;
    try {
      const id = await this.#api.renameFolder(account, folder.id, this.name);
      return { id, name: this.name.trim() };
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
  }
}

export class DeleteFolderState {
  readonly #api: FolderEditApi;
  busy = $state(false);
  error = $state<string | null>(null);

  constructor(api: FolderEditApi) {
    this.#api = api;
  }

  reset(): void {
    this.error = null;
  }

  /** Supprime le dossier ; `false` si rien n'est fait (erreur affichée). */
  async submit(account: number, folder: FolderDto): Promise<boolean> {
    if (this.busy) return false;
    this.busy = true;
    this.error = null;
    try {
      await this.#api.deleteFolder(account, folder.id);
      return true;
    } catch (e) {
      this.error = errorText(e);
      return false;
    } finally {
      this.busy = false;
    }
  }
}
