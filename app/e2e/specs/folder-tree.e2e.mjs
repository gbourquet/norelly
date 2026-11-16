// Arbre des dossiers du panneau de gauche, contre Dovecot :
// - un dossier spécial rangé dans la boîte de réception (`INBOX/Archive`,
//   reconnu par son nom) est remonté en tête, sans retrait, comme la boîte
//   de réception ;
// - un sous-dossier ordinaire reste en retrait sous son parent.
// Les dossiers créés sont supprimés avant (restes d'une exécution
// interrompue) et après : les autres scénarios et les captures de la
// documentation partagent le même serveur.
import { $, expect } from "@wdio/globals";
import { addTestAccount, address, imapCommand, removeTestAccount } from "./helpers.mjs";

const CREATED = ["INBOX/Archive", "INBOX/Perso/Banque"];
/** Dossiers à supprimer, parents implicites de Dovecot compris. */
const CLEANUP = ["INBOX/Archive", "INBOX/Perso/Banque", "INBOX/Perso"];

/** Bouton d'un dossier du compte de test, d'après son libellé affiché. */
function folder(label) {
  return $(
    `//ul[@aria-label="Dossiers de ${address}"]//button[contains(@class,"folder")]` +
      `[.//span[contains(@class,"label") and normalize-space()="${label}"]]`,
  );
}

async function indent(button) {
  return Number.parseFloat((await button.getCSSProperty("padding-left")).value);
}

async function cleanup() {
  for (const name of CLEANUP) await imapCommand(`DELETE "${name}"`);
}

describe("arbre des dossiers", () => {
  before(async () => {
    await cleanup();
    for (const name of CREATED) {
      expect(await imapCommand(`CREATE "${name}"`)).toMatch(/^b OK/m);
    }
    await addTestAccount();
  });

  after(async () => {
    await removeTestAccount();
    await cleanup();
  });

  it("affiche un dossier spécial imbriqué au premier niveau, sans retrait", async () => {
    const archive = folder("Archives");
    await archive.waitForDisplayed({ timeout: 90000 });
    const inbox = folder("Boîte de réception");
    expect(await indent(archive)).toBe(await indent(inbox));

    // Sous-dossier ordinaire : un cran sous son parent.
    const banque = folder("Banque");
    await banque.waitForDisplayed({ timeout: 30000 });
    const perso = folder("Perso");
    expect(await indent(perso)).toBeGreaterThan(await indent(inbox));
    expect(await indent(banque)).toBeGreaterThan(await indent(perso));
  });
});
