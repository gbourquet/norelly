// Glisser-déposer de conversations sur un dossier (runes Svelte 5), à la
// souris : événements pointeur plutôt que le glisser-déposer HTML, que
// l'interception native des fichiers déposés (pièces jointes) rend peu
// fiable dans la webview. Le dépôt exécute le même déplacement que
// « Déplacer vers… » ; Rust revalide la destination.

import type { ConversationDto, FolderDto } from "./ipc/api";

/** Distance (px) à parcourir bouton enfoncé avant que le glisser
 * commence : en deçà, c'est un clic. */
export const DRAG_THRESHOLD = 5;

/** Conversations emportées. */
export interface DragPayload {
  ids: number[];
  account: number;
  /** Dossiers d'origine. */
  mailboxes: number[];
  /** Sujet de la conversation, si elle est seule ; sinon `null` (le nombre
   * est affiché). */
  label: string | null;
}

/** Dossier sous le pointeur. */
export interface DropTarget {
  account: number;
  folder: FolderDto;
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
): DragPayload | null {
  const taken = marked.includes(row.id)
    ? conversations.filter((c) => marked.includes(c.id))
    : [row];
  if (taken.some((c) => c.account !== row.account)) return null;
  return {
    ids: taken.map((c) => c.id),
    account: row.account,
    mailboxes: taken.map((c) => c.mailbox).filter((m, i, all) => all.indexOf(m) === i),
    label: taken.length === 1 ? row.subject : null,
  };
}

/** Le dossier `folder` du compte `account` accepte les conversations
 * emportées : même compte, sélectionnable, pas déjà leur seul dossier
 * (affichage seulement : Rust revalide). */
export function canDrop(payload: DragPayload, account: number, folder: FolderDto): boolean {
  return (
    account === payload.account &&
    folder.selectable &&
    payload.mailboxes.some((m) => m !== folder.id)
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
  #target: DropTarget | null = null;
  #pressed: { payload: DragPayload; x: number; y: number } | null = null;
  #suppress = false;

  /** Bouton enfoncé sur une conversation. */
  press(payload: DragPayload, x: number, y: number): void {
    this.#pressed = { payload, x, y };
    this.#suppress = false;
  }

  /** Pointeur déplacé ; `target` : dossier sous le pointeur. */
  move(x: number, y: number, target: DropTarget | null): void {
    const pressed = this.#pressed;
    if (this.payload === null) {
      if (pressed === null || Math.hypot(x - pressed.x, y - pressed.y) < DRAG_THRESHOLD) return;
      this.payload = pressed.payload;
    }
    this.x = x;
    this.y = y;
    const payload = this.payload;
    this.#target =
      target !== null && canDrop(payload, target.account, target.folder) ? target : null;
    this.over = this.#target?.folder.id ?? null;
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
   * doit pas ouvrir la conversation. */
  suppressClick(): boolean {
    const suppress = this.#suppress;
    this.#suppress = false;
    return suppress;
  }

  #reset(): void {
    this.payload = null;
    this.over = null;
    this.#target = null;
    this.#pressed = null;
  }
}
