// Traduction des erreurs IPC (ADR-011) : Rust envoie un code, le champ
// concerné et un détail technique brut facultatif ; l'interface compose le
// message dans la langue active. Le détail est cité tel quel, jamais
// traduit. Aucune décision de sécurité ici.

import type { ErrorCodeDto, FailureKindDto, FieldDto } from "./ipc/bindings";
import { m } from "./paraglide/messages.js";

/** Erreur IPC structurée (voir `IpcError` côté Rust). */
export class IpcFailure extends Error {
  readonly code: ErrorCodeDto;
  readonly field: FieldDto | null;
  readonly detail: string | null;

  constructor(code: ErrorCodeDto, field: FieldDto | null = null, detail: string | null = null) {
    // Message technique (journaux, tests) ; l'affichage passe par errorText.
    super(`${code}${field ? ` (${field})` : ""}${detail ? ` : ${detail}` : ""}`);
    this.code = code;
    this.field = field;
    this.detail = detail;
  }
}

/** Ajoute un détail technique brut, cité tel quel. */
export function withDetail(message: string, detail: string | null | undefined): string {
  return detail ? m.err_detail({ message, detail }) : message;
}

export function fieldLabel(field: FieldDto | null): string {
  switch (field) {
    case "address":
      return m.field_address();
    case "display_name":
      return m.field_display_name();
    case "host":
      return m.field_host();
    case "port":
      return m.field_port();
    case "username":
      return m.field_username();
    case "servers":
      return m.field_servers();
    case "account":
      return m.field_account();
    case "message":
      return m.field_message();
    case "part":
      return m.field_part();
    case "mailbox":
      return m.field_mailbox();
    case "role":
      return m.field_role();
    case "cursor":
      return m.field_cursor();
    case "conversation":
      return m.field_conversation();
    case "folder":
      return m.field_folder();
    case "certificate":
      return m.field_certificate();
    case "draft":
      return m.field_draft();
    case "attachment":
      return m.field_attachment();
    case "outbox":
      return m.field_outbox();
    case "query":
      return m.field_query();
    case "settings":
      return m.field_settings();
    case "update":
      return m.field_update();
    case null:
      return "?";
  }
}

function codeText(code: ErrorCodeDto, field: FieldDto | null): string {
  switch (code) {
    case "invalid_input":
      return m.err_invalid_input({ field: fieldLabel(field) });
    case "not_found":
      return m.err_not_found({ field: fieldLabel(field) });
    case "no_valid_sender":
      return m.err_no_valid_sender();
    case "download_failed":
      return m.err_download_failed();
    case "storage":
      return m.err_storage();
    case "secret_store":
      return m.err_secret_store();
    case "link_refused":
      return m.err_link_refused();
    case "executable_attachment":
      return m.err_executable_attachment();
    case "write_failed":
      return m.err_write_failed();
    case "open_failed":
      return m.err_open_failed();
    case "no_trash":
      return m.err_no_trash();
    case "no_archive":
      return m.err_no_archive();
    case "same_folder":
      return m.err_same_folder();
    case "folder_not_selectable":
      return m.err_folder_not_selectable();
    case "too_many_messages":
      return m.err_too_many_messages();
    case "pending_queue_full":
      return m.err_pending_queue_full();
    case "no_recipient":
      return m.err_no_recipient();
    case "too_many_recipients":
      return m.err_too_many_recipients();
    case "invalid_subject":
      return m.err_invalid_subject();
    case "body_too_large":
      return m.err_body_too_large();
    case "attachments_too_large":
      return m.err_attachments_too_large();
    case "too_many_attachments":
      return m.err_too_many_attachments();
    case "schedule_too_far":
      return m.err_schedule_too_far();
    case "outbox_busy":
      return m.err_outbox_busy();
    case "folder_name_empty":
      return m.err_folder_name_empty();
    case "folder_name_too_long":
      return m.err_folder_name_too_long();
    case "folder_name_invalid":
      return m.err_folder_name_invalid();
    case "folder_name_delimiter":
      return m.err_folder_name_delimiter();
    case "folder_exists":
      return m.err_folder_exists();
    case "no_subfolders":
      return m.err_no_subfolders();
    case "folder_create_failed":
      return m.err_folder_create_failed();
    case "folder_protected":
      return m.err_folder_protected();
    case "folder_has_subfolders":
      return m.err_folder_has_subfolders();
    case "folder_rename_failed":
      return m.err_folder_rename_failed();
    case "folder_delete_failed":
      return m.err_folder_delete_failed();
    case "update_failed":
      return m.err_update_failed();
    case "internal":
      return m.err_internal();
  }
}

/** Message affichable d'une erreur quelconque (IPC ou inattendue). */
export function errorText(e: unknown): string {
  if (e instanceof IpcFailure) return withDetail(codeText(e.code, e.field), e.detail);
  return m.err_unexpected();
}

/** Libellé court d'une catégorie d'échec de connexion. */
export function failureText(failure: FailureKindDto): string {
  switch (failure) {
    case "network":
      return m.failure_network();
    case "authentication":
      return m.failure_authentication();
    case "security":
      return m.failure_security();
    case "protocol":
      return m.failure_protocol();
    case "invalid_input":
      return m.failure_invalid_input();
    case "secret_store":
      return m.failure_secret_store();
    case "storage":
      return m.failure_storage();
    case "internal":
      return m.failure_internal();
  }
}
