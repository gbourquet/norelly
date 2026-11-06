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

/** Actions d'une conversation : archiver, supprimer, déplacer, puis les
 * bascules suivi et lu / non lu selon son état. */
export function conversationActions(subject: ActionSubject | null): VisibleAction[] {
  return [archive, remove, move, flagToggle(subject), readToggle(subject)];
}

/** Barre du mail ouvert : actions principales, libellées, et actions
 * secondaires du menu « Plus… ». */
export function readerActions(subject: ActionSubject | null): {
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
    ],
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
