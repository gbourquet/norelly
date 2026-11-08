import { afterEach, describe, expect, it } from "vitest";
import type { ErrorCodeDto, FailureKindDto, FieldDto } from "./ipc/bindings";
import { IpcFailure, errorText, failureText, fieldLabel } from "./errors";
import { setLocale } from "./paraglide/runtime.js";

describe("traduction des erreurs IPC", () => {
  afterEach(() => setLocale("fr", { reload: false }));

  it("compose le message à partir du code et du champ", () => {
    expect(errorText(new IpcFailure("invalid_input", "address"))).toBe(
      "Valeur refusée : adresse électronique.",
    );
    expect(errorText(new IpcFailure("not_found", "part"))).toBe("Introuvable : pièce jointe.");
    expect(errorText(new IpcFailure("link_refused"))).toMatch(/http, https et mailto/);
  });

  it("explique une file d'actions pleine", () => {
    expect(errorText(new IpcFailure("pending_queue_full"))).toBe(
      "Trop d'actions attendent le serveur sur ce compte : réessayez une fois reconnecté.",
    );
  });

  it("explique les refus de la rédaction", () => {
    expect(errorText(new IpcFailure("no_recipient"))).toBe("Ajoutez au moins un destinataire.");
    expect(errorText(new IpcFailure("too_many_recipients"))).toBe("Trop de destinataires.");
    expect(errorText(new IpcFailure("invalid_subject"))).toBe(
      "Le sujet contient un caractère interdit ou est trop long.",
    );
    expect(errorText(new IpcFailure("body_too_large"))).toBe("Le message est trop long.");
    expect(errorText(new IpcFailure("attachments_too_large"))).toBe(
      "Les pièces jointes dépassent 25 Mio.",
    );
    expect(errorText(new IpcFailure("outbox_busy"))).toBe(
      "Ce message est en cours d'envoi : réessayez dans un instant.",
    );
    expect(errorText(new IpcFailure("not_found", "draft"))).toBe("Introuvable : brouillon.");
    expect(errorText(new IpcFailure("invalid_input", "attachment"))).toBe(
      "Valeur refusée : fichier joint.",
    );
    expect(errorText(new IpcFailure("not_found", "outbox"))).toBe(
      "Introuvable : message en attente d'envoi.",
    );
  });

  it("explique les refus du carnet d'adresses", () => {
    expect(errorText(new IpcFailure("contact_no_address"))).toBe(
      "Indiquez au moins une adresse électronique.",
    );
    expect(errorText(new IpcFailure("contact_invalid", "contact_email", "bob@"))).toBe(
      "Valeur refusée dans la fiche : adresse électronique du contact. (« bob@ »)",
    );
    expect(errorText(new IpcFailure("contact_too_long", "given_name"))).toBe(
      "Texte trop long : prénom.",
    );
    expect(errorText(new IpcFailure("contact_duplicate_address", null, "bob@example.com"))).toBe(
      "Cette adresse appartient déjà à un autre contact. (« bob@example.com »)",
    );
    expect(errorText(new IpcFailure("contact_not_found"))).toBe("Ce contact n'existe plus.");
  });

  it("explique les refus des listes de diffusion", () => {
    expect(errorText(new IpcFailure("contact_list_no_name", "list_name"))).toBe(
      "Donnez un nom à la liste.",
    );
    expect(errorText(new IpcFailure("contact_list_duplicate_name", "list_name", "Famille"))).toBe(
      "Une autre liste porte déjà ce nom. (« Famille »)",
    );
    expect(errorText(new IpcFailure("contact_list_too_long", "list_description"))).toBe(
      "Texte trop long dans la liste : description de la liste.",
    );
    expect(errorText(new IpcFailure("contact_list_invalid", "list_name"))).toBe(
      "Valeur refusée dans la liste : nom de la liste.",
    );
    expect(errorText(new IpcFailure("contact_list_too_many_members"))).toBe(
      "Une liste compte au plus 100 membres.",
    );
    expect(errorText(new IpcFailure("contact_list_empty"))).toBe(
      "Cette liste n'a aucun membre : rien à ajouter.",
    );
    expect(errorText(new IpcFailure("contact_list_not_found"))).toBe("Cette liste n'existe plus.");
  });

  it("cite le détail technique tel quel, sans le traduire", () => {
    expect(errorText(new IpcFailure("open_failed", null, "No such file"))).toBe(
      "Ouverture impossible. (« No such file »)",
    );
  });

  it("suit la langue active", () => {
    setLocale("en", { reload: false });
    expect(errorText(new IpcFailure("invalid_input", "port"))).toBe("Value rejected: port.");
    expect(failureText("network")).toBe("server unreachable");
  });

  it("une erreur inattendue n'expose pas son texte", () => {
    expect(errorText(new Error("pile interne"))).toBe("Erreur inattendue.");
  });

  it("traduit chaque code, champ et catégorie d'échec (liste vérifiée par le typage)", () => {
    const codes = {
      invalid_input: true,
      not_found: true,
      no_valid_sender: true,
      download_failed: true,
      storage: true,
      secret_store: true,
      link_refused: true,
      executable_attachment: true,
      write_failed: true,
      open_failed: true,
      no_trash: true,
      no_archive: true,
      same_folder: true,
      folder_not_selectable: true,
      too_many_messages: true,
      pending_queue_full: true,
      no_recipient: true,
      too_many_recipients: true,
      invalid_subject: true,
      body_too_large: true,
      attachments_too_large: true,
      too_many_attachments: true,
      schedule_too_far: true,
      outbox_busy: true,
      folder_name_empty: true,
      folder_name_too_long: true,
      folder_name_invalid: true,
      folder_name_delimiter: true,
      folder_exists: true,
      no_subfolders: true,
      folder_create_failed: true,
      folder_protected: true,
      folder_has_subfolders: true,
      folder_into_itself: true,
      folder_rename_failed: true,
      folder_move_failed: true,
      folder_delete_failed: true,
      update_failed: true,
      contact_no_address: true,
      contact_invalid: true,
      contact_too_long: true,
      contact_repeated_address: true,
      contact_duplicate_address: true,
      contact_not_found: true,
      contact_list_no_name: true,
      contact_list_invalid: true,
      contact_list_too_long: true,
      contact_list_duplicate_name: true,
      contact_list_too_many_members: true,
      contact_list_empty: true,
      contact_list_not_found: true,
      contact_import_too_large: true,
      contact_import_too_many_cards: true,
      contact_import_unknown_format: true,
      contact_import_read_failed: true,
      contact_import_expired: true,
      internal: true,
    } satisfies Record<ErrorCodeDto, true>;
    const fields = {
      address: true,
      display_name: true,
      host: true,
      port: true,
      username: true,
      servers: true,
      account: true,
      message: true,
      part: true,
      mailbox: true,
      role: true,
      cursor: true,
      conversation: true,
      folder: true,
      certificate: true,
      draft: true,
      attachment: true,
      outbox: true,
      query: true,
      settings: true,
      update: true,
      contact: true,
      contact_email: true,
      contact_phone: true,
      given_name: true,
      family_name: true,
      nickname: true,
      organization: true,
      notes: true,
      contact_list: true,
      list_name: true,
      list_description: true,
    } satisfies Record<FieldDto, true>;
    const kinds = {
      network: true,
      authentication: true,
      security: true,
      protocol: true,
      invalid_input: true,
      secret_store: true,
      storage: true,
      internal: true,
    } satisfies Record<FailureKindDto, true>;
    const texts = (Object.keys(codes) as ErrorCodeDto[]).map((c) =>
      errorText(new IpcFailure(c, "address")),
    );
    for (const t of texts) expect(t).not.toBe("Erreur inattendue.");
    expect(new Set(texts).size).toBe(texts.length);
    const labels = (Object.keys(fields) as FieldDto[]).map(fieldLabel);
    expect(new Set(labels).size).toBe(labels.length);
    expect(fieldLabel(null)).toBe("?");
    const failures = (Object.keys(kinds) as FailureKindDto[]).map(failureText);
    expect(new Set(failures).size).toBe(failures.length);
  });
});
