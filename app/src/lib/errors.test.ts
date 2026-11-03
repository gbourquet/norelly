import { afterEach, describe, expect, it } from "vitest";
import { IpcFailure, errorText, failureText } from "./errors";
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
      "Valeur refusée : pièce jointe.",
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
});
