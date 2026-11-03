// Actions proposées à la souris (barre d'outils, survol des lignes, menu
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

export interface VisibleAction {
  /** Identifiant de la commande du registre. */
  command: string;
  icon: Component<{ size?: number; "aria-hidden"?: "true" }>;
}

/** État d'une conversation qui choisit les bascules proposées. */
export interface ActionSubject {
  unread: number;
  flagged: boolean;
}

/** Actions d'une conversation : archiver, supprimer, déplacer, puis les
 * bascules suivi et lu / non lu selon son état. */
export function conversationActions(subject: ActionSubject | null): VisibleAction[] {
  return [
    { command: "action.archive", icon: ArchiveIcon },
    { command: "action.delete", icon: TrashIcon },
    { command: "action.move", icon: FolderInputIcon },
    { command: "action.flag", icon: subject?.flagged ? FlagOffIcon : FlagIcon },
    subject !== null && subject.unread > 0
      ? { command: "action.mark-read", icon: MailOpenIcon }
      : { command: "action.mark-unread", icon: MailIcon },
  ];
}
