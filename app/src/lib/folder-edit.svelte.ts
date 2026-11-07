// Renommage, déplacement et suppression d'un dossier (runes Svelte 5) :
// saisie et confirmation ici ; la décision (dossier protégé, nom,
// emplacement, sous-dossiers) est prise et revalidée par Rust.

import type { Api, FolderDto } from "./ipc/api";
import { parentChoices, type ParentChoice } from "./new-folder.svelte";
import { errorText } from "./errors";

export type FolderEditApi = Pick<Api, "renameFolder" | "deleteFolder" | "moveFolder">;

/** Dossier ordinaire : la boîte de réception et les dossiers spéciaux ne
 * proposent ni renommage, ni déplacement, ni suppression (Rust les refuse
 * aussi). */
export function isEditableFolder(folder: FolderDto): boolean {
  return folder.role === "normal";
}

/** Dossier renommé ou déplacé : nom affiché et identifiant local sous son
 * nouveau nom complet (`null` si la liste n'est pas encore relue). */
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

/** Nom complet du dossier qui contient `folder` (`null` : premier
 * niveau) : son nom sans le séparateur ni le dernier segment. */
export function parentName(folder: FolderDto): string | null {
  if (folder.depth === 0) return null;
  return folder.name.slice(0, folder.name.length - folder.leaf.length - 1);
}

/** Vrai si `f` est `folder` ou l'un de ses sous-dossiers, d'après les noms
 * (un dossier spécial rangé dans `folder` n'est pas affiché à sa suite). */
export function isSameOrUnder(f: FolderDto, folder: FolderDto): boolean {
  if (f.id === folder.id) return true;
  if (f.depth <= folder.depth || !f.name.startsWith(folder.name)) return false;
  // Séparateur du serveur : le caractère qui précède le dernier segment.
  const delimiter = f.name.charAt(f.name.length - f.leaf.length - 1);
  return f.name.charAt(folder.name.length) === delimiter;
}

/** Dossier qui contient `folder` (`null` : premier niveau). */
function parentOf(folders: FolderDto[], folder: FolderDto): number | null {
  const name = parentName(folder);
  return folders.find((f) => f.name === name)?.id ?? null;
}

/** Emplacements où déplacer `folder` : ceux d'un nouveau dossier, sauf le
 * dossier lui-même et ses sous-dossiers (Rust le refuse aussi). */
export function destinationChoices(
  folders: FolderDto[],
  folder: FolderDto,
  topLevel: string,
  inbox: string,
): ParentChoice[] {
  return parentChoices(
    folders.filter((f) => !isSameOrUnder(f, folder)),
    topLevel,
    inbox,
  );
}

export class MoveFolderState {
  readonly #api: FolderEditApi;
  /** Emplacement actuel du dossier. */
  #current = $state<number | null>(null);
  parent = $state<number | null>(null);
  busy = $state(false);
  error = $state<string | null>(null);

  readonly canSubmit = $derived(this.parent !== this.#current && !this.busy);

  constructor(api: FolderEditApi) {
    this.#api = api;
  }

  /** Formulaire positionné sur l'emplacement actuel du dossier. */
  reset(folder: FolderDto, folders: FolderDto[]): void {
    this.#current = parentOf(folders, folder);
    this.parent = this.#current;
    this.error = null;
  }

  /** Déplace le dossier ; `null` si rien n'est fait (erreur affichée). */
  async submit(account: number, folder: FolderDto): Promise<RenamedFolder | null> {
    if (!this.canSubmit) return null;
    this.busy = true;
    this.error = null;
    try {
      const id = await this.#api.moveFolder(account, folder.id, this.parent);
      return { id, name: folder.leaf };
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
