// Doublons du carnet de bout en bout, à la souris :
// - deux fiches « Alice Martin » (l'une saisie « Martin Alice ») et une
//   fiche « Bob » qui partage le téléphone d'Alice ;
// - « Rechercher les doublons… » dans « Plus… » de l'en-tête de la vue
//   Contacts : un groupe, rapproché par le nom et le téléphone, la fiche
//   la plus complète proposée à garder ;
// - Bob décoché, « Fusionner » : une seule Alice, aux deux adresses ; Bob
//   reste ; une nouvelle recherche ne trouve plus rien.
import { $, $$, browser, expect } from "@wdio/globals";
import { addTestAccount, barAction, removeTestAccount } from "./helpers.mjs";

const button = (name) =>
  $(
    `//button[@aria-label="${name}" or normalize-space()="${name}"][not(ancestor::*[@aria-hidden="true"])]`,
  );
const field = (label) =>
  $(`//label[span[normalize-space()="${label}"]]//*[self::input or self::textarea]`);
const dialog = (title) => $(`//*[@role="dialog"][.//*[normalize-space()="${title}"]]`);
const rows = (name) => $$(`//*[@role="option"][.//*[normalize-space()="${name}"]]`);
const header = () => $('[role="toolbar"][aria-label="Créer, importer ou exporter"]');

/** Nouvelle fiche saisie à la souris. */
async function create(given, family, address, phone = "") {
  await button("Nouveau contact").click();
  const form = dialog("Nouveau contact");
  await form.waitForDisplayed();
  await field("Prénom").setValue(given);
  await field("Nom").setValue(family);
  await $('[aria-label="Adresse électronique 1"]').setValue(address);
  if (phone !== "") {
    await button("Ajouter un téléphone").click();
    await $('[aria-label="Téléphone 1"]').setValue(phone);
  }
  await button("Enregistrer").click();
  await form.waitForDisplayed({ reverse: true, timeout: 10000 });
}

async function findDuplicates() {
  await barAction(header(), "Rechercher les doublons…");
  const window = dialog("Doublons possibles");
  await window.waitForDisplayed({ timeout: 10000 });
  return window;
}

describe("doublons du carnet", () => {
  before(async () => {
    await addTestAccount();
    await $("nav").$("button=Contacts").click();
    await $("p*=Aucun contact pour l'instant").waitForDisplayed({ timeout: 10000 });
    await create("Alice", "Martin", "alice@norelly.test", "06 12 34 56 78");
    await create("Martin", "Alice", "alice.pro@norelly.test");
    await create("Bob", "", "bob@norelly.test", "0612345678");
  });

  after(async () => {
    await browser.keys("Escape");
    await $("nav").$("button*=Boîte de réception unifiée").click();
    await removeTestAccount();
  });

  it("présente le groupe de doublons, la fiche la plus complète gardée", async () => {
    const window = await findDuplicates();
    await expect(window.$("p=Groupe 1 sur 1")).toBeDisplayed();
    await expect(window.$("li=Même nom")).toBeDisplayed();
    await expect(window.$("li=Même téléphone")).toBeDisplayed();
    const cards = window.$$("li.card");
    await expect(cards).toBeElementsArrayOfSize(3);
    // Fiche au téléphone d'abord, gardée.
    await expect(cards[0].$("span=alice@norelly.test")).toBeDisplayed();
    await expect(cards[0].$('input[type="radio"]')).toBeSelected();
    await expect(cards[0].$('input[type="checkbox"]')).toBeDisabled();
  });

  it("fusionne les fiches cochées et laisse la fiche décochée", async () => {
    const window = dialog("Doublons possibles");
    await window.$('input[aria-label="Réunir Bob"]').click();
    await window.$("button=Fusionner").click();
    await expect(window.$("p=Tous les doublons ont été examinés.")).toBeDisplayed();
    await window.$("button=Fermer").click();
    await window.waitForDisplayed({ reverse: true, timeout: 10000 });
    await expect($("span*=1 groupe(s) de doublons fusionné(s).")).toBeDisplayed();

    await expect(rows("Alice Martin")).toBeElementsArrayOfSize(1);
    await expect(rows("Bob")).toBeElementsArrayOfSize(1);
    await rows("Alice Martin")[0].click();
    const card = $('article[aria-label="Fiche de Alice Martin"]');
    await expect(card.$("span=alice@norelly.test")).toBeDisplayed();
    await expect(card.$("span=alice.pro@norelly.test")).toBeDisplayed();
  });

  it("une nouvelle recherche ne propose plus que Bob et Alice, rapprochés par le téléphone", async () => {
    const window = await findDuplicates();
    await expect(window.$("li=Même téléphone")).toBeDisplayed();
    await expect(window.$("li=Même nom")).not.toBeExisting();
    await window.$("button=Passer").click();
    await expect(window.$("p=Tous les doublons ont été examinés.")).toBeDisplayed();
    await window.$("button=Fermer").click();
    await window.waitForDisplayed({ reverse: true, timeout: 10000 });
  });
});
