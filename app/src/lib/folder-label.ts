// Libellé affiché d'un dossier : traduit pour un dossier spécial, d'après
// son rôle (SPECIAL-USE ou nom reconnu par Rust) ; nom du serveur sinon.

import type { FolderDto } from "./ipc/api";
import { m } from "./paraglide/messages.js";

type Labelled = Pick<FolderDto, "role" | "leaf">;

/** Libellé traduit par rôle, lu à chaque appel (langue courante). */
const SPECIAL = new Map<string, () => string>([
  ["inbox", () => m.mailbox_inbox()],
  ["drafts", () => m.mailbox_drafts()],
  ["sent", () => m.mailbox_sent()],
  ["archive", () => m.mailbox_archive()],
  ["junk", () => m.mailbox_junk()],
  ["trash", () => m.mailbox_trash()],
  ["all", () => m.mailbox_all()],
  ["flagged", () => m.mailbox_flagged()],
]);

/** Libellé d'un dossier dans l'interface. */
export function folderLabel(folder: Labelled): string {
  return SPECIAL.get(folder.role)?.() ?? folder.leaf;
}

/** Nom du serveur, en info-bulle, quand le libellé affiché en diffère
 * (`INBOX` excepté : nom imposé par le protocole). */
export function folderServerName(folder: Labelled): string | undefined {
  if (folder.role === "inbox") return undefined;
  return folderLabel(folder) === folder.leaf ? undefined : folder.leaf;
}
