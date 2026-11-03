// Texte des notifications de nouveaux messages, composé dans la langue de
// l'utilisateur (ADR-011) à partir de l'événement Rust ; Rust a déjà
// appliqué les paramètres (désactivées, confidentialité) et revalide ce
// texte avant de l'afficher.

import type { SyncEventDto } from "./ipc/api";
import { personName } from "./format";
import { m } from "./paraglide/messages.js";

export type NewMailEvent = Extract<SyncEventDto, { kind: "new_mail" }>;

/** Titre et texte d'une notification ; `account` : adresse du compte. */
export function notificationText(
  event: NewMailEvent,
  account: string,
): { title: string; body: string } {
  const from = event.from ?? null;
  const subject = event.subject ?? null;
  const title =
    event.count === 1
      ? from !== null
        ? personName(from)
        : m.notify_new_one()
      : m.notify_new_many({ count: event.count });
  if (from === null || subject === null) return { title, body: account };
  const shown = subject !== "" ? subject : m.common_no_subject();
  const body =
    event.count === 1 ? shown : m.notify_latest({ from: personName(from), subject: shown });
  return { title, body };
}
