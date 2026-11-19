// Photos de contact de bout en bout, à la souris :
// - un export vCard dont une fiche porte une photo PNG intégrée, une autre
//   une photo donnée par adresse (le serveur canari), une troisième une
//   « photo » SVG piégée annoncée comme PNG ;
// - la fiche importée montre la photo, servie par `contact-photo://` (image
//   chargée, ronde, à côté du nom) ;
// - la photo par adresse n'est jamais demandée (aucune requête au canari) et
//   la fiche montre l'initiale ; la photo piégée est écartée, initiale aussi ;
// - nettoyage : les contacts importés sont supprimés.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { $, browser, expect } from "@wdio/globals";
import {
  addTestAccount,
  barAction,
  canaryHits,
  openMenu,
  pickFile,
  removeTestAccount,
} from "./helpers.mjs";

const data = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../crates/adapters/contacts-io/tests/data",
);

const dialog = (title) => $(`//*[@role="dialog"][.//*[normalize-space()="${title}"]]`);

const row = (name) => $(`//*[@role="option"][.//*[normalize-space()="${name}"]]`);

const card = (name) => $(`article[aria-label="Fiche de ${name}"]`);

/** Taille naturelle de l'image de l'avatar de la fiche ouverte (0 si elle
 * n'est pas chargée), et son adresse. */
async function avatarImage() {
  return browser.execute(() => {
    const img = document.querySelector("article .avatar img");
    return img instanceof HTMLImageElement
      ? { src: img.getAttribute("src"), width: img.naturalWidth, complete: img.complete }
      : null;
  });
}

describe("photos de contact", () => {
  before(async () => {
    await addTestAccount();
  });

  after(async () => {
    await browser.keys("Escape");
    await $("nav").$("button*=Boîte de réception unifiée").click();
    await removeTestAccount();
  });

  it("importe une vCard avec photo et l'affiche sur la fiche", async () => {
    await $("nav").$("button=Contacts").click();
    await $("p*=Aucun contact pour l'instant").waitForDisplayed({ timeout: 10000 });
    await barAction($('[role="toolbar"][aria-label="Créer, importer ou exporter"]'), "Importer…");
    await pickFile("Choisir le fichier de contacts à importer", path.join(data, "photo.vcf"));
    const preview = dialog("Importer des contacts");
    await preview.waitForDisplayed({ timeout: 20000 });
    await expect(preview.$("span=3 nouveau(x) contact(s) ajouté(s)")).toBeDisplayed();
    await preview.$("button=Importer").click();
    await preview.waitForDisplayed({ reverse: true, timeout: 20000 });
    await expect($("span*=Import terminé : 3 contact(s) ajouté(s)")).toBeDisplayed();

    await row("Camille Rousseau").click();
    await expect(card("Camille Rousseau")).toBeDisplayed();
    await browser.waitUntil(async () => (await avatarImage())?.complete === true, {
      timeout: 10000,
      timeoutMsg: "photo non chargée",
    });
    const photo = await avatarImage();
    expect(photo?.src).toMatch(/^contact-photo:\/\/localhost\/\d+$/);
    // Réencodée en 256 px au plus (l'originale en fait 400).
    expect(photo?.width).toBe(256);
  });

  it("ne charge jamais une photo donnée par adresse, ni une photo piégée", async () => {
    for (const [name, initial] of [
      ["Hugo Lemaire", "H"],
      ["Inès Faure", "I"],
    ]) {
      await row(name).click();
      const opened = card(name);
      await expect(opened).toBeDisplayed();
      await expect(opened.$(".avatar img")).not.toBeExisting();
      // Avatar décoratif (`aria-hidden`) : son texte est lu dans la page.
      const shown = await browser.execute(() =>
        document.querySelector("article .avatar")?.textContent?.trim(),
      );
      expect(shown).toBe(initial);
    }
    expect(await canaryHits()).toEqual([]);
  });

  it("supprime les contacts importés", async () => {
    for (const name of ["Camille Rousseau", "Hugo Lemaire", "Inès Faure"]) {
      await row(name).$('input[type="checkbox"]').click();
    }
    await openMenu(
      $('section[aria-label="Contacts sélectionnés"]').$('.//button[normalize-space()="Plus…"]'),
    );
    await $(
      '//*[@role="menuitem"][.//*[contains(@class, "menu-label")][normalize-space()="Supprimer le contact"]]',
    ).click();
    const confirm = dialog("Supprimer les contacts");
    await confirm.waitForDisplayed();
    await confirm.$("button=Supprimer").click();
    await confirm.waitForDisplayed({ reverse: true, timeout: 10000 });
    await $("p*=Aucun contact pour l'instant").waitForDisplayed({ timeout: 10000 });
  });
});
