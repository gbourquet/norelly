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
