// Libellés de la file d'envoi (états, causes d'échec), dans la langue
// active.

import type { OutboxStateDto, SendFailureDto } from "./ipc/api";
import { m } from "./paraglide/messages.js";

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

export function stateLabel(state: OutboxStateDto): string {
  switch (state.state) {
    case "queued":
      return m.outbox_state_queued();
    case "sending":
      return m.outbox_state_sending();
    case "sent":
      return m.outbox_state_sent();
    case "failed":
      return failureLabel(state.failure);
  }
}
