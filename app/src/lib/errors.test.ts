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
      outbox_busy: true,
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
