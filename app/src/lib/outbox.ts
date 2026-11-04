// Libellés de la file d'envoi (états, causes d'échec), dans la langue
// active.

import type { OutboxStateDto, SendFailureDto } from "./ipc/api";
import { m } from "./paraglide/messages.js";
import { longDate } from "./format";

export function failureLabel(failure: SendFailureDto): string {
  switch (failure) {
    case "unreachable":
      return m.send_failure_unreachable();
    case "temporary":
      return m.send_failure_temporary();
    case "rejected":
      return m.send_failure_rejected();
    case "authentication":
      return m.send_failure_authentication();
    case "security":
      return m.send_failure_security();
    case "uncertain":
      return m.send_failure_uncertain();
    case "too_many_attempts":
      return m.send_failure_too_many_attempts();
  }
}

/** Seuil au-delà duquel un message en attente est « programmé » (une
 * attente de nouvel essai reste « en attente »). */
const SCHEDULED_AFTER_SECS = 60;

/** Message en attente d'une échéance choisie (envoi différé). */
export function isScheduled(state: OutboxStateDto, now = Date.now() / 1000): boolean {
  return state.state === "queued" && state.not_before > now + SCHEDULED_AFTER_SECS;
}

/** Date et heure locales saisies (`datetime-local`) en secondes Unix. */
export function unixOfLocal(value: string): number | null {
  if (value === "") return null;
  const ms = new Date(value).getTime();
  return Number.isFinite(ms) ? Math.floor(ms / 1000) : null;
}

export function stateLabel(state: OutboxStateDto, now = Date.now() / 1000): string {
  switch (state.state) {
    case "queued":
      return isScheduled(state, now)
        ? m.outbox_state_scheduled({ date: longDate(state.not_before) })
        : m.outbox_state_queued();
    case "sending":
      return m.outbox_state_sending();
    case "sent":
      return m.outbox_state_sent();
    case "failed":
      return failureLabel(state.failure);
  }
}
