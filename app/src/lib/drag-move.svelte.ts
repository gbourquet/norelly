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
import { folderLabel } from "./folder-label";

/** Distance (px) à parcourir bouton enfoncé avant que le glisser
 * commence : en deçà, c'est un clic. */
export const DRAG_THRESHOLD = 5;

/** Durée (ms) d'un appui long au doigt avant que le glisser commence ;
 * plus courte que celle qui ouvre le menu contextuel (700 ms). */
export const LONG_PRESS_MS = 400;

/** Délai (ms) après le lever du doigt pendant lequel le menu de l'appui
 * long, envoyé par certaines webviews à ce moment-là, est retenu. */
const AFTER_TOUCH_MENU_MS = 500;

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
  return { kind: "folder", account, folder, label: folderLabel(folder) };
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

/** Appui d'un pointeur (champs d'un `PointerEvent`). */
export interface PointerGrab {
  pointerType: string;
  pointerId: number;
  clientX: number;
  clientY: number;
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
  /** Doigt posé sur un élément saisissable, jusqu'à son lever : la page
   * doit pouvoir empêcher le défilement natif si le glisser commence. */
  touching = $state(false);
  /** Le pointeur s'est éloigné du point de saisie d'au moins la bande de
   * défilement : le panneau peut défiler près de ses bords (il ne file pas
   * sous un élément saisi près d'un bord). */
  autoScroll = $state(false);
  #target: DropTarget | null = null;
  #pressed: {
    payload: DragPayload;
    x: number;
    y: number;
    touch: boolean;
    /** Un menu contextuel de l'application est derrière l'élément saisi. */
    menu: boolean;
  } | null = null;
  #pointer: number | null = null;
  #suppress = false;
  #suppressTimer: ReturnType<typeof setTimeout> | null = null;
  #timer: ReturnType<typeof setTimeout> | null = null;
  /** Le pointeur s'est éloigné du point d'appui depuis le début du glisser
   * (toujours à la souris : le glisser y naît d'un déplacement). */
  #moved = false;
  /** Glisser au doigt tout juste déposé : le menu de l'appui long, que
   * certaines webviews envoient au lever du doigt, est retenu une fois. */
  #afterTouch: ReturnType<typeof setTimeout> | null = null;

  /** Bouton enfoncé sur une conversation ou un dossier. */
  press(payload: DragPayload, x: number, y: number): void {
    this.#reset();
    this.#pressed = { payload, x, y, touch: false, menu: false };
    this.#setSuppress(false);
  }

  /** Doigt posé : le glisser commence après un appui long, sur place ; un
   * doigt qui bouge avant fait défiler, comme d'habitude. `menu` : l'appui
   * long ouvre aussi un menu de l'application, qui l'emporte si le doigt ne
   * bouge pas. */
  pressTouch(payload: DragPayload, x: number, y: number, menu: boolean): void {
    this.#reset();
    this.#pressed = { payload, x, y, touch: true, menu };
    this.touching = true;
    this.#setSuppress(false);
    this.#timer = setTimeout(() => {
      this.#timer = null;
      this.payload = payload;
      this.x = x;
      this.y = y;
    }, LONG_PRESS_MS);
  }

  /** Appui principal : à la souris ou au stylet, glisser au-delà du seuil ;
   * au doigt, après un appui long. `menu` : voir [`pressTouch`]. */
  grab(payload: DragPayload, pointer: PointerGrab, menu = false): void {
    const { pointerType, clientX: x, clientY: y } = pointer;
    if (pointerType === "mouse" || pointerType === "pen") this.press(payload, x, y);
    else if (pointerType === "touch") this.pressTouch(payload, x, y, menu);
    else return;
    this.#pointer = pointer.pointerId;
  }

  /** Le pointeur `pointerId` mène l'appui en cours (ou il n'y en a pas) :
   * un autre doigt ne déplace, ne dépose ni n'annule le glisser. */
  owns(pointerId: number): boolean {
    return this.#pointer === null || this.#pointer === pointerId;
  }

  /** Pointeur déplacé ; `target` : cible sous le pointeur. */
  move(x: number, y: number, target: DropTarget | null): void {
    const pressed = this.#pressed;
    if (pressed === null) return;
    const distance = Math.hypot(x - pressed.x, y - pressed.y);
    const far = distance >= DRAG_THRESHOLD;
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
    if (distance >= EDGE_SCROLL_BAND) this.autoScroll = true;
    this.x = x;
    this.y = y;
    const payload = this.payload;
    this.#target = target !== null && canDrop(payload, target) ? target : null;
    this.over = this.#target?.folder?.id ?? null;
    this.overRoot =
      this.#target !== null && this.#target.folder === null ? this.#target.account : null;
  }

  /** Menu contextuel demandé (clic droit, appui long) : vrai s'il doit être
   * retenu (glisser engagé, ou au doigt sans menu de l'application
   * derrière : seul celui de la webview viendrait). Sinon le menu l'emporte
   * et l'appui en cours est abandonné. */
  contextMenu(): boolean {
    if (this.#afterTouch !== null) {
      this.#clearAfterTouch();
      return true;
    }
    const pressed = this.#pressed;
    if (this.payload !== null && (this.#moved || pressed?.menu === false)) return true;
    if (pressed !== null) this.cancel();
    return false;
  }

  /** Bouton relâché ou doigt levé : le dépôt à exécuter, s'il y en a un. */
  release(): Drop | null {
    const payload = this.payload;
    const target = this.#target;
    const touchDrag = payload !== null && this.#moved && this.#pressed?.touch === true;
    this.#setSuppress(payload !== null);
    this.#reset();
    if (touchDrag) {
      this.#afterTouch = setTimeout(() => {
        this.#afterTouch = null;
      }, AFTER_TOUCH_MENU_MS);
    }
    return payload !== null && target !== null ? { ...target, payload } : null;
  }

  /** Glisser abandonné (Échap, défilement natif, menu contextuel). */
  cancel(): void {
    this.#setSuppress(this.payload !== null);
    this.#reset();
  }

  /** Vrai une fois juste après un glisser : le clic qui suit le
   * relâchement ne doit ni ouvrir la conversation ni afficher le dossier. */
  suppressClick(): boolean {
    const suppress = this.#suppress;
    this.#setSuppress(false);
    return suppress;
  }

  /** Le clic qui suit le relâchement arrive dans la même suite
   * d'événements : passé ce moment (relâché ailleurs que sur l'élément
   * saisi, donc sans clic), plus rien n'est avalé. */
  #setSuppress(value: boolean): void {
    if (this.#suppressTimer !== null) clearTimeout(this.#suppressTimer);
    this.#suppressTimer = null;
    this.#suppress = value;
    if (value) {
      this.#suppressTimer = setTimeout(() => {
        this.#suppressTimer = null;
        this.#suppress = false;
      }, 0);
    }
  }

  #clearAfterTouch(): void {
    if (this.#afterTouch !== null) clearTimeout(this.#afterTouch);
    this.#afterTouch = null;
  }

  #reset(): void {
    this.#clearAfterTouch();
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
    this.#moved = false;
    this.#pointer = null;
    this.touching = false;
    this.autoScroll = false;
    this.payload = null;
    this.over = null;
    this.overRoot = null;
    this.#target = null;
    this.#pressed = null;
  }
}
