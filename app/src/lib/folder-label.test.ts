import { afterEach, describe, expect, it } from "vitest";
import { folderLabel, folderServerName } from "./folder-label";
import { setLocale } from "./paraglide/runtime.js";

const special = (role: string, leaf: string) => ({ role, leaf });

describe("libellé d'un dossier", () => {
  afterEach(() => setLocale("fr", { reload: false }));

  it("traduit les dossiers spéciaux d'après leur rôle, pas leur nom", () => {
    setLocale("fr", { reload: false });
    expect(folderLabel(special("inbox", "INBOX"))).toBe("Boîte de réception");
    expect(folderLabel(special("drafts", "Drafts"))).toBe("Brouillons");
    expect(folderLabel(special("sent", "Sent Mail"))).toBe("Envoyés");
    expect(folderLabel(special("archive", "Archive"))).toBe("Archives");
    expect(folderLabel(special("junk", "Spam"))).toBe("Indésirables");
    expect(folderLabel(special("trash", "Bin"))).toBe("Corbeille");
    expect(folderLabel(special("all", "All Mail"))).toBe("Tous les messages");
    expect(folderLabel(special("flagged", "Starred"))).toBe("Messages suivis");
  });

  it("suit la langue de l'interface", () => {
    setLocale("en", { reload: false });
    expect(folderLabel(special("sent", "Envoyés"))).toBe("Sent");
    expect(folderLabel(special("trash", "Corbeille"))).toBe("Trash");
  });

  it("garde le nom du serveur pour un dossier ordinaire ou de rôle inconnu", () => {
    expect(folderLabel(special("normal", "Potager"))).toBe("Potager");
    expect(folderLabel(special("autre", "Potager"))).toBe("Potager");
    expect(folderLabel(special("constructor", "Potager"))).toBe("Potager");
  });

  it("donne le nom du serveur en info-bulle quand il diffère du libellé", () => {
    setLocale("fr", { reload: false });
    expect(folderServerName(special("sent", "Sent Mail"))).toBe("Sent Mail");
    expect(folderServerName(special("trash", "Corbeille"))).toBeUndefined();
    expect(folderServerName(special("inbox", "INBOX"))).toBeUndefined();
    expect(folderServerName(special("normal", "Potager"))).toBeUndefined();
  });
});
