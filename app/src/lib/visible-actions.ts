// Actions proposées à la souris (barre d'actions, survol des lignes, menu
// contextuel). Elles ne font qu'exécuter des commandes du registre
// (ADR-005) : mêmes conditions, mêmes effets que les raccourcis.

import type { Component } from "svelte";
import ArchiveIcon from "@lucide/svelte/icons/archive";
import TrashIcon from "@lucide/svelte/icons/trash-2";
import FolderInputIcon from "@lucide/svelte/icons/folder-input";
import FlagIcon from "@lucide/svelte/icons/flag";
import FlagOffIcon from "@lucide/svelte/icons/flag-off";
import MailOpenIcon from "@lucide/svelte/icons/mail-open";
import MailIcon from "@lucide/svelte/icons/mail";
import ReplyIcon from "@lucide/svelte/icons/reply";
import ReplyAllIcon from "@lucide/svelte/icons/reply-all";
import ForwardIcon from "@lucide/svelte/icons/forward";
import FileTextIcon from "@lucide/svelte/icons/file-text";
import ChevronUpIcon from "@lucide/svelte/icons/chevron-up";
import ChevronDownIcon from "@lucide/svelte/icons/chevron-down";
import EyeIcon from "@lucide/svelte/icons/eye";
import UserPenIcon from "@lucide/svelte/icons/user-pen";
import SquarePenIcon from "@lucide/svelte/icons/square-pen";
import StarIcon from "@lucide/svelte/icons/star";
import StarOffIcon from "@lucide/svelte/icons/star-off";
import UserPlusIcon from "@lucide/svelte/icons/user-plus";
import BookUserIcon from "@lucide/svelte/icons/book-user";
import { m } from "./paraglide/messages.js";

export interface VisibleAction {
  /** Identifiant de la commande du registre. */
  command: string;
  icon: Component<{ size?: number; "aria-hidden"?: "true" }>;
  /** Libellé selon l'état ; sinon, le titre de la commande. */
  label?: () => string;
  /** Rang de maintien quand la place manque (1 : gardé le plus
   * longtemps) ; les autres passent dans « Plus… ». */
  priority?: number;
  /** Groupe dans un menu : un séparateur entre deux groupes. */
  section?: number;
}

/** État d'une conversation qui choisit les bascules proposées. */
export interface ActionSubject {
  unread: number;
  flagged: boolean;
}

const archive: VisibleAction = { command: "action.archive", icon: ArchiveIcon };
const remove: VisibleAction = { command: "action.delete", icon: TrashIcon };
const move: VisibleAction = { command: "action.move", icon: FolderInputIcon };

/** Bascule lu / non lu : propose l'état inverse de la conversation. */
function readToggle(subject: ActionSubject | null): VisibleAction {
  return subject !== null && subject.unread > 0
    ? { command: "action.mark-read", icon: MailOpenIcon }
    : { command: "action.mark-unread", icon: MailIcon };
}

function flagToggle(subject: ActionSubject | null): VisibleAction {
  const flagged = subject?.flagged ?? false;
  return {
    command: "action.flag",
    icon: flagged ? FlagOffIcon : FlagIcon,
    label: flagged ? m.action_unfollow : m.action_follow,
  };
}

/** Personne d'un message pour les commandes du carnet : déjà dans le
 * carnet, inconnue du carnet, ou aucune. */
export type PersonKind = "contact" | "stranger" | null;

/** Actions sur une personne d'un message (son menu) : l'ajouter au
 * carnet, ou voir sa fiche si elle y est déjà. */
export function personActions(person: PersonKind): VisibleAction[] {
  if (person === "stranger") return [{ command: "contacts.add-person", icon: UserPlusIcon }];
  if (person === "contact") return [{ command: "contacts.show-person", icon: BookUserIcon }];
  return [];
}

/** Actions sur une personne, en dernier groupe d'un menu. */
const personSection = (person: PersonKind, section: number): VisibleAction[] =>
  personActions(person).map((a) => ({ ...a, section }));

/** Barre du mail ouvert : actions principales, libellées, et actions
 * secondaires du menu « Plus… » (dont l'expéditeur du dernier message,
 * `person`). */
export function readerActions(
  subject: ActionSubject | null,
  person: PersonKind = null,
): {
  primary: VisibleAction[];
  more: VisibleAction[];
} {
  return {
    primary: [
      { command: "compose.reply", icon: ReplyIcon, priority: 1 },
      { command: "compose.reply-all", icon: ReplyAllIcon, priority: 5 },
      { command: "compose.forward", icon: ForwardIcon, priority: 4 },
      { ...archive, priority: 2 },
      { ...remove, priority: 3 },
    ],
    more: [
      { ...readToggle(subject), section: 1 },
      { ...flagToggle(subject), section: 1 },
      { ...move, section: 1 },
      { command: "reading.plain-text", icon: FileTextIcon, section: 2 },
      { command: "conversation.previous", icon: ChevronUpIcon, section: 3 },
      { command: "conversation.next", icon: ChevronDownIcon, section: 3 },
      ...personSection(person, 4),
    ],
  };
}

/** Barre des conversations cochées : les deux bascules lu / non lu sont
 * proposées (sélection mêlée). */
export function bulkActions(): { primary: VisibleAction[]; more: VisibleAction[] } {
  return {
    primary: [
      { ...archive, priority: 1 },
      { ...remove, priority: 2 },
      { command: "action.mark-read", icon: MailOpenIcon, priority: 3 },
      { command: "action.mark-unread", icon: MailIcon, priority: 4 },
      { ...move, priority: 5 },
    ],
    more: [{ command: "action.flag", icon: FlagIcon }],
  };
}

/** État résumé des conversations visées, tel que les commandes le
 * traitent : suivies si toutes le sont, non lues si une l'est. */
export function subjectOf(targets: ActionSubject[]): ActionSubject | null {
  if (targets.length === 0) return null;
  return {
    unread: targets.some((t) => t.unread > 0) ? 1 : 0,
    flagged: targets.every((t) => t.flagged),
  };
}

/** Menu contextuel d'une ligne : ouvrir, puis les actions de la barre du
 * mail ouvert, en groupes, puis le dernier expéditeur (`person`). */
export function menuActions(
  subject: ActionSubject | null,
  person: PersonKind = null,
): VisibleAction[] {
  return [
    { command: "conversation.open", icon: EyeIcon, section: 0 },
    { command: "compose.reply", icon: ReplyIcon, section: 1 },
    { command: "compose.reply-all", icon: ReplyAllIcon, section: 1 },
    { command: "compose.forward", icon: ForwardIcon, section: 1 },
    { ...archive, section: 2 },
    { ...remove, section: 2 },
    { ...readToggle(subject), section: 3 },
    { ...flagToggle(subject), section: 3 },
    { ...move, section: 3 },
    ...personSection(person, 4),
  ];
}

/** État d'un contact qui choisit la bascule de favori. */
export interface ContactSubject {
  favorite: boolean;
}

const writeContact: VisibleAction = { command: "contacts.write", icon: SquarePenIcon };
const editContact: VisibleAction = { command: "contacts.edit", icon: UserPenIcon };
const deleteContact: VisibleAction = { command: "contacts.delete", icon: TrashIcon };

/** Bascule de favori : propose l'état inverse du contact. */
function favoriteToggle(subject: ContactSubject | null): VisibleAction {
  const favorite = subject?.favorite ?? false;
  return {
    command: "contacts.favorite",
    icon: favorite ? StarOffIcon : StarIcon,
    label: favorite ? m.contacts_unfavorite : m.contacts_favorite,
  };
}

/** Barre de la fiche ouverte et son menu « Plus… ». */
export function contactsToolbar(subject: ContactSubject | null): {
  primary: VisibleAction[];
  more: VisibleAction[];
} {
  return {
    primary: [
      { ...writeContact, priority: 1 },
      { ...editContact, priority: 2 },
      { ...deleteContact, priority: 3 },
    ],
    more: [favoriteToggle(subject)],
  };
}

/** Menu contextuel d'une ligne de la liste des contacts. */
export function contactMenuActions(subject: ContactSubject | null): VisibleAction[] {
  return [
    { ...editContact, section: 0 },
    { ...writeContact, section: 0 },
    { ...favoriteToggle(subject), section: 0 },
    { ...deleteContact, section: 1 },
  ];
}

/** Barre des contacts cochés. */
export function contactBulkActions(): { primary: VisibleAction[]; more: VisibleAction[] } {
  return {
    primary: [
      { ...writeContact, priority: 1 },
      { ...deleteContact, priority: 2 },
    ],
    more: [],
  };
}

/** Actions rapides au survol d'une ligne de la liste. */
export function rowActions(subject: ActionSubject | null): VisibleAction[] {
  return [archive, remove, readToggle(subject)];
}

/**
 * Boutons gardés visibles dans une largeur donnée : par priorité
 * croissante, tant qu'ils tiennent avec le bouton « Plus… » (`moreWidth`)
 * et les espaces (`gap`) ; le plus prioritaire reste toujours. Largeur
 * disponible nulle (pas encore mesurée) : tout reste visible.
 */
export function fitByPriority(
  items: { width: number; priority: number }[],
  available: number,
  moreWidth: number,
  gap: number,
): boolean[] {
  if (available <= 0) return items.map(() => true);
  const order = items.map((item, index) => ({ ...item, index }));
  order.sort((a, b) => a.priority - b.priority);
  const visible = items.map(() => false);
  let used = moreWidth;
  for (const [rank, item] of order.entries()) {
    const next = used + gap + item.width;
    if (rank > 0 && next > available) break;
    visible[item.index] = true;
    used = next;
  }
  return visible;
}
