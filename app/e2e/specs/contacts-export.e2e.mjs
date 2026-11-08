// Export du carnet de bout en bout, à la souris, puis réimport :
// - « Exporter… » de l'en-tête de la vue Contacts : fenêtre du format,
//   puis sélecteur d'enregistrement du système ; le fichier vCard 4.0 est
//   écrit par Rust (lignes CRLF repliées à 75 octets, accents) ;
// - « Exporter… » de la barre des contacts cochés : le seul contact coché,
//   en CSV (BOM UTF-8, téléphone « +33 » neutralisé contre les formules) ;
// - le carnet vidé, l'export vCard réimporté redonne les mêmes contacts
//   (accents, note sur plusieurs lignes, adresse préférée).
import { mkdtempSync, readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { $, $$, browser, expect } from "@wdio/globals";
import { addTestAccount, barAction, pickFile, removeTestAccount, saveFile } from "./helpers.mjs";

const data = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../crates/adapters/contacts-io/tests/data",
);
const out = mkdtempSync(path.join(os.tmpdir(), "norelly-export-"));

const dialog = (title) => $(`//*[@role="dialog"][.//*[normalize-space()="${title}"]]`);
const row = (name) => $(`//*[@role="option"][.//*[normalize-space()="${name}"]]`);
const header = () => $('[role="toolbar"][aria-label="Créer, importer ou exporter"]');

/** Fenêtre d'export : format `format`, enregistré sous `file`. */
async function exportAs(format, file) {
  const window = dialog("Exporter des contacts");
  await window.waitForDisplayed({ timeout: 10000 });
  await window.$(`span=${format}`).click();
  await window.$("button=Exporter…").click();
  await saveFile("Enregistrer les contacts exportés", file);
  await window.waitForDisplayed({ reverse: true, timeout: 20000 });
}

async function importFile(file) {
  await barAction(header(), "Importer…");
  await pickFile("Choisir le fichier de contacts à importer", file);
  const preview = dialog("Importer des contacts");
  await preview.waitForDisplayed({ timeout: 20000 });
  return preview;
}

async function deleteAll(names) {
  for (const name of names) {
    await row(name).$('input[type="checkbox"]').click();
  }
  await barAction($('section[aria-label="Contacts sélectionnés"]'), "Supprimer le contact");
  const confirm = dialog("Supprimer les contacts");
  await confirm.waitForDisplayed();
  await confirm.$("button=Supprimer").click();
  await confirm.waitForDisplayed({ reverse: true, timeout: 10000 });
  await $("p*=Aucun contact pour l'instant").waitForDisplayed({ timeout: 10000 });
}

describe("export du carnet", () => {
  before(async () => {
    await addTestAccount();
    await $("nav").$("button=Contacts").click();
    await $("p*=Aucun contact pour l'instant").waitForDisplayed({ timeout: 10000 });
    const preview = await importFile(path.join(data, "gmail.vcf"));
    await preview.$("button=Importer").click();
    await preview.waitForDisplayed({ reverse: true, timeout: 20000 });
    await expect(row("Élodie Cézanne")).toBeDisplayed();
  });

  after(async () => {
    await browser.keys("Escape");
    await $("nav").$("button*=Boîte de réception unifiée").click();
    await removeTestAccount();
  });

  it("exporte tout le carnet en vCard 4.0 depuis l'en-tête", async () => {
    const file = path.join(out, "carnet.vcf");
    await barAction(header(), "Exporter…");
    await expect(
      dialog("Exporter des contacts").$("p=Tout le carnet sera exporté."),
    ).toBeDisplayed();
    await exportAs("vCard 4.0 (.vcf)", file);
    await expect($("span*=2 contact(s) exporté(s) dans « carnet.vcf »")).toBeDisplayed();

    const bytes = readFileSync(file);
    const text = bytes.toString("utf8");
    expect(text.startsWith("BEGIN:VCARD\r\nVERSION:4.0\r\n")).toBe(true);
    expect(text).toContain("FN:Élodie Cézanne\r\n");
    expect(text).toContain("FN:Marc Durand\r\n");
    // CRLF partout, aucune ligne de plus de 75 octets.
    expect(text.replace(/\r\n/g, "")).not.toMatch(/[\r\n]/);
    for (const line of text.split("\r\n")) {
      expect(Buffer.byteLength(line, "utf8")).toBeLessThanOrEqual(75);
    }
  });

  it("exporte le contact coché en CSV depuis la barre de sélection", async () => {
    const file = path.join(out, "choisis.csv");
    await row("Élodie Cézanne").$('input[type="checkbox"]').click();
    await barAction($('section[aria-label="Contacts sélectionnés"]'), "Exporter…");
    await expect(
      dialog("Exporter des contacts").$("p=1 contact(s) choisi(s) ser(a/ont) exporté(s)."),
    ).toBeDisplayed();
    await exportAs("CSV (.csv)", file);
    await expect($("span*=1 contact(s) exporté(s) dans « choisis.csv »")).toBeDisplayed();
    await row("Élodie Cézanne").$('input[type="checkbox"]').click();

    const bytes = readFileSync(file);
    expect([...bytes.subarray(0, 3)]).toEqual([0xef, 0xbb, 0xbf]);
    const lines = bytes.subarray(3).toString("utf8").split("\r\n");
    expect(lines[0].startsWith("Name,First Name,Last Name,Nickname,Organization Name,Notes,")).toBe(
      true,
    );
    expect(lines.join("\n")).toContain("Élodie");
    expect(lines.join("\n")).not.toContain("Marc");
    // Téléphone « +33 » neutralisé contre les formules des tableurs.
    expect(lines.join("\n")).toContain(",'+33 6 12 34 56 78");
  });

  it("le carnet exporté se réimporte à l'identique", async () => {
    await deleteAll(["Élodie Cézanne", "Marc Durand"]);
    const preview = await importFile(path.join(out, "carnet.vcf"));
    await expect(preview.$("p*=« carnet.vcf » (vCard) : 2 fiche(s)")).toBeDisplayed();
    await expect(preview.$("span=2 nouveau(x) contact(s) ajouté(s)")).toBeDisplayed();
    await preview.$("button=Importer").click();
    await preview.waitForDisplayed({ reverse: true, timeout: 20000 });
    await expect($$('//*[@role="option"]')).toBeElementsArrayOfSize(2);

    await row("Élodie Cézanne").click();
    const card = $('article[aria-label="Fiche de Élodie Cézanne"]');
    await expect(card).toBeDisplayed();
    await expect(card.$("span=elodie.cezanne@example.com")).toBeDisplayed();
    await expect(card.$("span=elodie@example.org")).toBeDisplayed();
    await expect(card.$("p*=Rencontrée à Lyon, France.")).toBeDisplayed();
    await expect(card.$("p*=Préfère le téléphone.")).toBeDisplayed();

    await deleteAll(["Élodie Cézanne", "Marc Durand"]);
  });
});
