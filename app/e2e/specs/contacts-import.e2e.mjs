// Import d'un carnet de bout en bout, à la souris :
// - « Importer… » de la vue Contacts ouvre le sélecteur du système, le
//   fichier est lu en Rust ; un export vCard de Google (accents, note
//   repliée) donne un aperçu chiffré, puis deux contacts ;
// - un export CSV de Google Contacts : les fiches déjà importées sont
//   reconnues par leur adresse (doublons, choix de leur sort), la fiche
//   sans adresse est rejetée avec sa raison, la nouvelle est ajoutée ;
// - la fiche importée garde ses accents et sa note ;
// - nettoyage : les contacts importés sont supprimés (sélection multiple).
import path from "node:path";
import { fileURLToPath } from "node:url";
import { $, $$, browser, expect } from "@wdio/globals";
import { addTestAccount, openMenu, pickFile, removeTestAccount } from "./helpers.mjs";

const data = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../crates/adapters/contacts-io/tests/data",
);

const dialog = (title) => $(`//*[@role="dialog"][.//*[normalize-space()="${title}"]]`);

const row = (name) => $(`//*[@role="option"][.//*[normalize-space()="${name}"]]`);

const pickTitle = "Choisir le fichier de contacts à importer";

/** « Importer… » de l'en-tête, fichier choisi dans le sélecteur ; renvoie
 * la fenêtre d'aperçu. */
async function importFile(name) {
  // Celui de l'en-tête (le carnet vide en montre un second).
  await $('[role="toolbar"][aria-label="Créer ou importer"]').$("button=Importer…").click();
  await pickFile(pickTitle, path.join(data, name));
  const preview = dialog("Importer des contacts");
  await preview.waitForDisplayed({ timeout: 20000 });
  return preview;
}

describe("import du carnet", () => {
  before(async () => {
    await addTestAccount();
  });

  after(async () => {
    await browser.keys("Escape");
    await $("nav").$("button*=Boîte de réception unifiée").click();
    await removeTestAccount();
  });

  it("importe un export vCard de Google", async () => {
    await $("nav").$("button=Contacts").click();
    await $("p*=Aucun contact pour l'instant").waitForDisplayed({ timeout: 10000 });
    const preview = await importFile("gmail.vcf");
    await expect(preview.$("p*=« gmail.vcf » (vCard) : 2 fiche(s)")).toBeDisplayed();
    await expect(preview.$("span=2 nouveau(x) contact(s) ajouté(s)")).toBeDisplayed();
    // Ni doublon ni rejet : rien à choisir.
    await expect(preview.$("legend=Contacts déjà dans le carnet")).not.toBeExisting();
    await preview.$("button=Importer").click();
    await preview.waitForDisplayed({ reverse: true, timeout: 20000 });
    await expect(
      $("span*=Import terminé : 2 contact(s) ajouté(s), 0 complété(s)."),
    ).toBeDisplayed();
    await expect(row("Élodie Cézanne")).toBeDisplayed();
    await expect(row("Marc Durand")).toBeDisplayed();

    await row("Élodie Cézanne").click();
    const card = $('article[aria-label="Fiche de Élodie Cézanne"]');
    await expect(card).toBeDisplayed();
    await expect(card.$("span=elodie.cezanne@example.com")).toBeDisplayed();
    await expect(card.$("p*=Rencontrée à Lyon, France.")).toBeDisplayed();
  });

  it("importe un export CSV de Google : doublons reconnus, fiche sans adresse rejetée", async () => {
    const preview = await importFile("gmail.csv");
    await expect(preview.$("p*=CSV de Google Contacts")).toBeDisplayed();
    await expect(preview.$("span=1 nouveau(x) contact(s) ajouté(s)")).toBeDisplayed();
    await expect(
      preview.$("span=2 fiche(s) déjà dans le carnet, laissée(s) de côté"),
    ).toBeDisplayed();
    await expect(preview.$("span=1 fiche(s) impossible(s) à importer")).toBeDisplayed();
    await expect(preview.$("legend=Contacts déjà dans le carnet")).toBeDisplayed();
    await preview.$("summary=Voir les fiches impossibles à importer").click();
    await expect(
      preview.$("li=Fiche n° 4 (Jean Sansadresse) : aucune adresse électronique"),
    ).toBeDisplayed();
    // Compléter : rien n'est effacé ; l'aperçu suit le choix.
    await preview.$("span=Les compléter avec le fichier (rien n'est effacé)").click();
    await expect(preview.$("span=1 nouveau(x) contact(s) ajouté(s)")).toBeDisplayed();
    await preview.$("button=Importer").click();
    await preview.waitForDisplayed({ reverse: true, timeout: 20000 });
    await expect($("span*=Import terminé : 1 contact(s) ajouté(s)")).toBeDisplayed();
    await expect(row("contact@plomberie.example")).toBeDisplayed();
    // Pas de doublon créé.
    await expect($$('//*[@role="option"]')).toBeElementsArrayOfSize(3);
  });

  it("supprime les contacts importés", async () => {
    for (const name of ["Élodie Cézanne", "Marc Durand", "contact@plomberie.example"]) {
      await row(name).$('input[type="checkbox"]').click();
    }
    // Trop étroite, la barre range la suppression dans « Plus… ».
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
