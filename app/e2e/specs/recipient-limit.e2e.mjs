// Limite de destinataires d'un message, prévenue dès l'insertion :
// - deux listes de 60 membres chacune (contacts importés, cochés, puis
//   « Ajouter à une liste… ») ;
// - dans la rédaction, la première liste se déplie en puces ; la seconde
//   ferait dépasser la limite : elle n'est pas insérée et un avertissement
//   donne le nombre et la limite, tout de suite, sous les destinataires ;
// - retirer une puce efface l'avertissement ;
// - nettoyage : listes et contacts supprimés.
import { mkdtempSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { $, browser, expect } from "@wdio/globals";
import {
  addTestAccount,
  barAction,
  openContextMenu,
  pickFile,
  removeTestAccount,
} from "./helpers.mjs";

const PER_LIST = 60;

const names = (group) =>
  Array.from({ length: PER_LIST }, (_, i) => `Limite ${group}${String(i + 1).padStart(2, "0")}`);
const groupA = names("A");
const groupB = names("B");

/** Carnet de 120 contacts (vCard), une adresse chacun. */
function vcardFile() {
  const dir = mkdtempSync(path.join(os.tmpdir(), "norelly-limite-"));
  const file = path.join(dir, "limite.vcf");
  const cards = [...groupA, ...groupB].map((name) => {
    const address = `${name.replace(" ", "-").toLowerCase()}@norelly.test`;
    return ["BEGIN:VCARD", "VERSION:3.0", `FN:${name}`, `EMAIL:${address}`, "END:VCARD"].join(
      "\r\n",
    );
  });
  writeFileSync(file, `${cards.join("\r\n")}\r\n`);
  return file;
}

const button = (name, scope = null) => {
  const xpath = `.//button[@aria-label="${name}" or normalize-space()="${name}"][not(ancestor::*[@aria-hidden="true"])]`;
  return scope === null ? $(xpath.slice(1)) : scope.$(xpath);
};

const dialog = (title) => $(`//*[@role="dialog"][.//*[normalize-space()="${title}"]]`);

const menuItem = (label) =>
  $(`//*[@role="menuitem"][.//*[contains(@class, "menu-label")][normalize-space()="${label}"]]`);

const row = (name) => $(`//*[@role="option"][.//*[normalize-space()="${name}"]]`);

const field = (label) =>
  $(`//label[span[normalize-space()="${label}"]]//*[self::input or self::textarea]`);

/** Puces de destinataires (classe `chip` exactement, pas `chips`). */
const CHIP = './/*[contains(concat(" ", normalize-space(@class), " "), " chip ")]';
const chips = (scope) => scope.$$(CHIP);

/** Coche chaque contact de `group` : la liste est virtualisée, elle
 * défile jusqu'à la ligne avant de cocher sa case. */
async function markAll(group) {
  const scrollTo = (top) =>
    browser.execute((y) => {
      const scroller = document.querySelector('.scroller[role="listbox"]');
      if (scroller) scroller.scrollTop = y;
    }, top);
  await button("Tous").click();
  const search = $('input[aria-label="Rechercher un contact"]');
  await search.setValue(group[0].slice(0, -2));
  // La liste a pu rester défilée par un passage précédent.
  await scrollTo(0);
  await row(group[0]).waitForDisplayed({ timeout: 10000 });
  for (const [index, name] of group.entries()) {
    const box = $(`input[aria-label="Sélectionner ${name}"]`);
    if (!(await box.isExisting())) {
      await scrollTo(index * 52);
      await box.waitForExist({ timeout: 10000 });
    }
    await box.scrollIntoView({ block: "center" });
    if (!(await box.isSelected())) await box.click();
  }
}

/** Barre des contacts cochés. */
const bulkBar = () => $('section[aria-label="Contacts sélectionnés"]');

/** Ajoute les contacts cochés à la liste `list` (« Plus… » de la barre de
 * sélection). */
async function addMarkedTo(list) {
  await barAction(bulkBar(), "Ajouter à une liste…");
  const add = dialog("Ajouter à une liste");
  await add.waitForDisplayed();
  await add.$(`.//label[.//span[normalize-space()="${list}"]]`).click();
  await button("Ajouter", add).click();
  await add.waitForDisplayed({ reverse: true, timeout: 10000 });
  const clear = button("Désélectionner les contacts");
  if (await clear.isDisplayed().catch(() => false)) await clear.click();
}

async function createList(name) {
  await button("Nouvelle liste").click();
  const form = dialog("Nouvelle liste");
  await form.waitForDisplayed();
  await field("Nom de la liste").setValue(name);
  await button("Enregistrer", form).click();
  await form.waitForDisplayed({ reverse: true, timeout: 10000 });
}

/** Choisit la liste `name` dans les propositions du champ « À ». */
async function chooseList(compose, typed, name) {
  const to = compose.$('[aria-label="À"]');
  await to.click();
  // WebKitWebDriver perd les lettres doublées d'une frappe.
  await browser.keys(typed);
  const option = compose.$(`//*[@role="option"][.//*[normalize-space()="${name}"]]`);
  await option.waitForDisplayed({ timeout: 10000 });
  await option.$("button").click();
}

describe("limite de destinataires", () => {
  before(async () => {
    await addTestAccount();
    await $("nav").$("button=Contacts").click();
    await barAction($('[role="toolbar"][aria-label="Créer, importer ou exporter"]'), "Importer…");
    await pickFile("Choisir le fichier de contacts à importer", vcardFile());
    const preview = dialog("Importer des contacts");
    await preview.waitForDisplayed({ timeout: 20000 });
    await preview.$("button=Importer").click();
    await preview.waitForDisplayed({ reverse: true, timeout: 20000 });
    await button("Listes").click();
    await createList("Grande A");
    await createList("Grande B");
    await button("Tous").click();
    await markAll(groupA);
    await addMarkedTo("Grande A");
    await markAll(groupB);
    await addMarkedTo("Grande B");
  });

  after(async () => {
    // Fenêtre ou menu resté ouvert : refermés, puis nettoyage ; le compte
    // est retiré quoi qu'il arrive, pour les scénarios suivants.
    await browser.keys("Escape");
    await browser.keys("Escape");
    try {
      await cleanUp();
    } finally {
      await browser.keys("Escape");
      await $("nav").$("button*=Boîte de réception unifiée").click();
      await removeTestAccount();
    }
  });

  async function cleanUp() {
    await $("nav").$("button=Contacts").click();
    await button("Listes").click();
    for (const name of ["Grande A", "Grande B"]) {
      if (!(await row(name).isExisting())) continue;
      await openContextMenu(row(name));
      await menuItem("Supprimer la liste").click();
      const confirm = dialog("Supprimer la liste ?");
      await confirm.waitForDisplayed();
      await confirm.$("button=Supprimer").click();
      await confirm.waitForDisplayed({ reverse: true, timeout: 10000 });
    }
    await button("Tous").click();
    for (const group of [groupA, groupB]) {
      await markAll(group);
      await barAction(bulkBar(), "Supprimer le contact");
      const confirm = dialog("Supprimer les contacts");
      await confirm.waitForDisplayed();
      await confirm.$("button=Supprimer").click();
      await confirm.waitForDisplayed({ reverse: true, timeout: 10000 });
    }
    await $('input[aria-label="Rechercher un contact"]').clearValue();
  }

  it("prévient dès l'insertion d'une liste qui ferait dépasser la limite", async () => {
    await $("nav").$("button*=Boîte de réception unifiée").click();
    await button("Nouveau message").click();
    const compose = dialog("Nouveau message");
    await compose.waitForDisplayed({ timeout: 20000 });

    await chooseList(compose, "grande a", "Grande A");
    await browser.waitUntil(async () => (await chips(compose).length) === PER_LIST, {
      timeout: 10000,
      timeoutMsg: "première liste non dépliée",
    });
    await expect(compose.$(".recipient-warning")).not.toBeExisting();

    await chooseList(compose, "grande b", "Grande B");
    const warning = compose.$(".recipient-warning");
    await warning.waitForDisplayed({ timeout: 10000 });
    await expect(warning).toHaveAttribute("role", "alert");
    await expect(warning).toHaveText(
      "Destinataires non ajoutés : le message en compterait 120, au-delà de la limite de 100.",
    );
    // Rien d'inséré de la seconde liste.
    await expect(chips(compose)).toBeElementsArrayOfSize(PER_LIST);

    // Retirer une puce efface l'avertissement.
    await (await chips(compose))[0].$(".//button").click();
    await warning.waitForDisplayed({ reverse: true, timeout: 10000 });

    await button("Supprimer le brouillon", compose).click();
    await compose.waitForDisplayed({ reverse: true, timeout: 20000 });
  });
});
