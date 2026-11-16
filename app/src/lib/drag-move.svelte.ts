// Glisser-déposer dans le panneau de gauche (runes Svelte 5), à la souris
// ou au doigt (appui long) : des conversations (liste, résultats de
// recherche, message ouvert) sur un dossier, ou un dossier dans un autre
// (ou au premier niveau, sur la ligne du compte). Événements pointeur
// plutôt que le glisser-déposer HTML, que l'interception native des
// fichiers déposés (pièces jointes) rend peu fiable dans la webview et que
// les webviews ne déclenchent pas au toucher. Le dépôt exécute le même
// déplacement que « Déplacer vers… » ou « Déplacer le dossier… » ; Rust
// revalide la destination.

import type { ConversationDto, FolderDto, ScopeDto, SearchHit } from "./ipc/api";
import { isEditableFolder, isSameOrUnder, parentName } from "./folder-edit.svelte";

/** Distance (px) à parcourir bouton enfoncé avant que le glisser
 * commence : en deçà, c'est un clic. */
export const DRAG_THRESHOLD = 5;

/** Durée (ms) d'un appui long au doigt avant que le glisser commence ;
 * plus courte que celle qui ouvre le menu contextuel (700 ms). */
export const LONG_PRESS_MS = 400;

/** Hauteur (px) de la bande, près d'un bord du panneau, qui le fait
 * défiler pendant un glisser. */
export const EDGE_SCROLL_BAND = 48;

/** Défilement maximal (px par image), au bord même ou au-delà. */
export const EDGE_SCROLL_MAX = 16;

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
  /** Portée dans laquelle agir (résultat de recherche, message ouvert) ;
   * absente : celle de la liste affichée. */
  scope?: ScopeDto;
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

/** Conversation d'un résultat de recherche, agie dans le dossier du
 * résultat (comme une fois ouverte) ; `null` si le message n'est pas encore
 * rattaché à une conversation. */
export function hitPayload(hit: SearchHit): ConversationsPayload | null {
  if (hit.conversation === null) return null;
  return {
    kind: "conversations",
    ids: [hit.conversation],
    account: hit.account,
    mailboxes: [hit.mailbox],
    label: hit.subject.map((s) => s.text).join(""),
    scope: { kind: "mailbox", id: hit.mailbox },
  };
}

/** Conversation ouverte, emportée depuis son titre, dans la portée où elle
 * est ouverte. */
export function openedPayload(
  conversation: ConversationDto,
  scope: ScopeDto,
): ConversationsPayload {
  return {
    kind: "conversations",
    ids: [conversation.id],
    account: conversation.account,
    mailboxes: [conversation.mailbox],
    label: conversation.subject,
    scope,
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

/** Défilement (px par image, négatif vers le haut) du panneau `rect` pour
 * un pointeur en (`x`, `y`) pendant un glisser : nul hors de sa largeur ou
 * loin de ses bords, plus rapide au plus près d'un bord, maximal au-delà ;
 * au moins un pixel dans la bande (un pas fractionnaire serait perdu). */
export function edgeScroll(
  x: number,
  y: number,
  rect: { left: number; right: number; top: number; bottom: number },
): number {
  if (x < rect.left || x > rect.right) return 0;
  const step = (distance: number): number =>
    Math.max(1, Math.round(EDGE_SCROLL_MAX * (1 - Math.max(distance, 0) / EDGE_SCROLL_BAND)));
  const top = y - rect.top;
  if (top < EDGE_SCROLL_BAND) return -step(top);
  const bottom = rect.bottom - y;
  if (bottom < EDGE_SCROLL_BAND) return step(bottom);
  return 0;
}

export class DragMove {
  /** Ce qui est en cours de glisser (`null` : pas de glisser). */
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
  #pressed: { payload: DragPayload; x: number; y: number; touch: boolean } | null = null;
  #suppress = false;
  #timer: ReturnType<typeof setTimeout> | null = null;
  /** Le pointeur s'est éloigné du point d'appui depuis le début du glisser
   * (toujours à la souris : le glisser y naît d'un déplacement). */
  #moved = false;

  /** Bouton enfoncé sur une conversation ou un dossier. */
  press(payload: DragPayload, x: number, y: number): void {
    this.#reset();
    this.#pressed = { payload, x, y, touch: false };
    this.#suppress = false;
  }

  /** Doigt posé : le glisser commence après un appui long, sur place ; un
   * doigt qui bouge avant fait défiler, comme d'habitude. */
  pressTouch(payload: DragPayload, x: number, y: number): void {
    this.#reset();
    this.#pressed = { payload, x, y, touch: true };
    this.#suppress = false;
    this.#timer = setTimeout(() => {
      this.#timer = null;
      this.payload = payload;
      this.x = x;
      this.y = y;
    }, LONG_PRESS_MS);
  }

  /** Pointeur déplacé ; `target` : cible sous le pointeur. */
  move(x: number, y: number, target: DropTarget | null): void {
    const pressed = this.#pressed;
    if (pressed === null) return;
    const far = Math.hypot(x - pressed.x, y - pressed.y) >= DRAG_THRESHOLD;
    if (this.payload === null) {
      if (!far) return;
      if (pressed.touch) {
        // Doigt parti avant l'appui long : c'est un défilement.
        this.#reset();
        return;
      }
      this.payload = pressed.payload;
    }
    if (far) this.#moved = true;
    this.x = x;
    this.y = y;
    const payload = this.payload;
    this.#target = target !== null && canDrop(payload, target) ? target : null;
    this.over = this.#target?.folder?.id ?? null;
    this.overRoot =
      this.#target !== null && this.#target.folder === null ? this.#target.account : null;
  }

  /** Menu contextuel demandé (clic droit, appui long) : vrai s'il doit être
   * retenu, parce qu'un glisser est en cours et que le pointeur est parti.
   * Sinon le menu l'emporte et l'appui en cours est abandonné. */
  contextMenu(): boolean {
    if (this.payload !== null && this.#moved) return true;
    if (this.#pressed !== null) this.cancel();
    return false;
  }

  /** Bouton relâché ou doigt levé : le dépôt à exécuter, s'il y en a un. */
  release(): Drop | null {
    const payload = this.payload;
    const target = this.#target;
    this.#suppress = payload !== null;
    this.#reset();
    return payload !== null && target !== null ? { ...target, payload } : null;
  }

  /** Glisser abandonné (Échap, défilement natif, menu contextuel). */
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
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
    this.#moved = false;
    this.payload = null;
    this.over = null;
    this.overRoot = null;
    this.#target = null;
    this.#pressed = null;
  }
}
