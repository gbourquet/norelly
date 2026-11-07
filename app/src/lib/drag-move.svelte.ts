// Glisser-déposer dans le panneau de gauche (runes Svelte 5), à la souris :
// des conversations sur un dossier, ou un dossier dans un autre (ou au
// premier niveau, sur la ligne du compte). Événements pointeur plutôt que
// le glisser-déposer HTML, que l'interception native des fichiers déposés
// (pièces jointes) rend peu fiable dans la webview. Le dépôt exécute le
// même déplacement que « Déplacer vers… » ou « Déplacer le dossier… » ;
// Rust revalide la destination.

import type { ConversationDto, FolderDto } from "./ipc/api";
import { isEditableFolder, isSameOrUnder, parentName } from "./folder-edit.svelte";

/** Distance (px) à parcourir bouton enfoncé avant que le glisser
 * commence : en deçà, c'est un clic. */
export const DRAG_THRESHOLD = 5;

/** Conversations emportées. */
export interface ConversationsPayload {
  kind: "conversations";
  ids: number[];
  account: number;
  /** Dossiers d'origine. */
  mailboxes: number[];
  /** Sujet de la conversation, si elle est seule ; sinon `null` (le nombre
   * est affiché). */
  label: string | null;
}

/** Dossier emporté (avec ses sous-dossiers). */
export interface FolderPayload {
  kind: "folder";
  account: number;
  folder: FolderDto;
  label: string;
}

export type DragPayload = ConversationsPayload | FolderPayload;

/** Cible sous le pointeur : un dossier, ou le premier niveau du compte
 * (`folder` à `null`, ligne du compte). */
export interface DropTarget {
  account: number;
  folder: FolderDto | null;
}

/** Dépôt à exécuter. */
export interface Drop extends DropTarget {
  payload: DragPayload;
}

/** Conversations emportées en saisissant `row` : toutes les cochées si
 * elle en fait partie, sinon elle seule ; `null` si elles viennent de
 * plusieurs comptes (un dossier n'appartient qu'à un compte). */
export function dragPayload(
  row: ConversationDto,
  marked: number[],
  conversations: ConversationDto[],
): ConversationsPayload | null {
  const taken = marked.includes(row.id)
    ? conversations.filter((c) => marked.includes(c.id))
    : [row];
  if (taken.some((c) => c.account !== row.account)) return null;
  return {
    kind: "conversations",
    ids: taken.map((c) => c.id),
    account: row.account,
    mailboxes: taken.map((c) => c.mailbox).filter((m, i, all) => all.indexOf(m) === i),
    label: taken.length === 1 ? row.subject : null,
  };
}

/** Dossier emporté en saisissant `folder` : seulement un dossier ordinaire
 * (la boîte de réception et les dossiers spéciaux ne se déplacent pas). */
export function folderPayload(account: number, folder: FolderDto): FolderPayload | null {
  if (!isEditableFolder(folder)) return null;
  return { kind: "folder", account, folder, label: folder.leaf };
}

/** La cible accepte ce qui est emporté (affichage seulement : Rust
 * revalide).
 * - Conversations : un dossier du même compte, sélectionnable, pas déjà
 *   leur seul dossier.
 * - Dossier : même compte, ni lui-même ni un de ses sous-dossiers, pas un
 *   dossier virtuel, pas l'endroit où il est déjà. */
export function canDrop(payload: DragPayload, target: DropTarget): boolean {
  if (target.account !== payload.account) return false;
  const folder = target.folder;
  if (payload.kind === "conversations") {
    return folder !== null && folder.selectable && payload.mailboxes.some((m) => m !== folder.id);
  }
  const current = parentName(payload.folder);
  if (folder === null) return current !== null;
  return (
    folder.role !== "all" &&
    folder.role !== "flagged" &&
    !isSameOrUnder(folder, payload.folder) &&
    folder.name !== current
  );
}

export class DragMove {
  /** Conversations en cours de glisser (`null` : pas de glisser). */
  payload = $state<DragPayload | null>(null);
  /** Position du pointeur. */
  x = $state(0);
  y = $state(0);
  /** Dossier valide survolé. */
  over = $state<number | null>(null);
  /** Compte dont le premier niveau (ligne du compte) est une cible valide
   * survolée. */
  overRoot = $state<number | null>(null);
  #target: DropTarget | null = null;
  #pressed: { payload: DragPayload; x: number; y: number } | null = null;
  #suppress = false;

  /** Bouton enfoncé sur une conversation ou un dossier. */
  press(payload: DragPayload, x: number, y: number): void {
    this.#pressed = { payload, x, y };
    this.#suppress = false;
  }

  /** Pointeur déplacé ; `target` : cible sous le pointeur. */
  move(x: number, y: number, target: DropTarget | null): void {
    const pressed = this.#pressed;
    if (this.payload === null) {
      if (pressed === null || Math.hypot(x - pressed.x, y - pressed.y) < DRAG_THRESHOLD) return;
      this.payload = pressed.payload;
    }
    this.x = x;
    this.y = y;
    const payload = this.payload;
    this.#target = target !== null && canDrop(payload, target) ? target : null;
    this.over = this.#target?.folder?.id ?? null;
    this.overRoot =
      this.#target !== null && this.#target.folder === null ? this.#target.account : null;
  }

  /** Bouton relâché : le dépôt à exécuter, s'il y en a un. */
  release(): Drop | null {
    const payload = this.payload;
    const target = this.#target;
    this.#suppress = payload !== null;
    this.#reset();
    return payload !== null && target !== null ? { ...target, payload } : null;
  }

  /** Glisser abandonné (Échap). */
  cancel(): void {
    this.#suppress = this.payload !== null;
    this.#reset();
  }

  /** Vrai une fois après un glisser : le clic qui suit le relâchement ne
   * doit ni ouvrir la conversation ni afficher le dossier. */
  suppressClick(): boolean {
    const suppress = this.#suppress;
    this.#suppress = false;
    return suppress;
  }

  #reset(): void {
    this.payload = null;
    this.over = null;
    this.overRoot = null;
    this.#target = null;
    this.#pressed = null;
  }
}
