// Synchronisation CardDAV du carnet de bout en bout, à la souris, contre le
// faux serveur CardDAV du réseau de test (e2e/carddav.mjs, HTTPS) :
// - « Synchroniser avec un serveur… » dans « Plus… » de l'en-tête de la
//   vue Contacts ;
// - nom du serveur seul saisi : Rust trouve le carnet (adresse bien
//   connue, principal, dossier des carnets) ; mauvais mot de passe refusé ;
// - « Synchroniser ce carnet » : la fiche du serveur arrive, un contact
//   saisi ici part sur le serveur sans effacer l'anniversaire d'une fiche
//   du serveur ;
// - « Déconnecter » : les contacts restent.
import { $, $$, browser, expect } from "@wdio/globals";
import { addTestAccount, barAction, password, removeTestAccount } from "./helpers.mjs";

const control = process.env.NORELLY_E2E_CARDDAV ?? "";

const button = (name) =>
  $(
    `//button[@aria-label="${name}" or normalize-space()="${name}"][not(ancestor::*[@aria-hidden="true"])]`,
  );
const field = (label) =>
  $(`//label[span[normalize-space()="${label}"]]//*[self::input or self::textarea]`);
const dialog = (title) => $(`//*[@role="dialog"][.//*[normalize-space()="${title}"]]`);
const rows = (name) => $$(`//*[@role="option"][.//*[normalize-space()="${name}"]]`);
const header = () => $('[role="toolbar"][aria-label="Créer, importer ou exporter"]');

/** Fiches du faux serveur (port de contrôle). */
async function remoteCards() {
  const response = await fetch(`${control}/`);
  return response.json();
}

async function openSync() {
  await barAction(header(), "Synchroniser avec un serveur…");
  const window = dialog("Synchroniser le carnet");
  await window.waitForDisplayed({ timeout: 10000 });
  return window;
}

describe("synchronisation CardDAV du carnet", () => {
  before(async () => {
    await addTestAccount();
    await $("nav").$("button=Contacts").click();
    await $("p*=Aucun contact pour l'instant").waitForDisplayed({ timeout: 10000 });
  });

  after(async () => {
    // Carnet distant déconnecté puis carnet vidé, même après un échec : il
    // est commun aux scénarios (déconnecté d'abord, pour ne rien supprimer
    // sur le serveur).
    await browser.keys("Escape");
    const window = await openSync().catch(() => null);
    if (window !== null) {
      const disconnect = window.$("button=Déconnecter");
      if (await disconnect.isDisplayed().catch(() => false)) await disconnect.click();
      await browser.keys("Escape");
    }
    const boxes = await $$('[role="option"] input[type="checkbox"]');
    if (boxes.length > 0) {
      for (const box of boxes) await box.click();
      await barAction($('section[aria-label="Contacts sélectionnés"]'), "Supprimer le contact");
      const confirm = dialog("Supprimer les contacts");
      await confirm.waitForDisplayed();
      await confirm.$("button=Supprimer").click();
      await confirm.waitForDisplayed({ reverse: true, timeout: 10000 });
    }
    await $("nav").$("button*=Boîte de réception unifiée").click();
    await removeTestAccount();
  });

  it("refuse un mauvais mot de passe sans perdre la saisie", async () => {
    const window = await openSync();
    await field("Adresse du serveur").setValue("carddav.norelly.test");
    await field("Identifiant").setValue("norelly");
    await field("Mot de passe d'application").setValue("mauvais");
    await window.$("button=Rechercher les carnets").click();
    await expect(
      window.$("p=Identifiant ou mot de passe refusé par le serveur de contacts."),
    ).toBeDisplayed();
    await expect(field("Adresse du serveur")).toHaveValue("carddav.norelly.test");
  });

  it("trouve le carnet depuis le nom du serveur et le synchronise", async () => {
    const window = dialog("Synchroniser le carnet");
    await field("Mot de passe d'application").setValue(password);
    await window.$("button=Rechercher les carnets").click();
    const book = window.$('input[type="radio"][name="carddav-book"]');
    await book.waitForDisplayed({ timeout: 15000 });
    await expect(book).toBeSelected();
    await expect(window.$("span=Contacts")).toBeDisplayed();
    await window.$("button=Synchroniser ce carnet").click();
    await window
      .$("p*=Relié au carnet « Contacts » de norelly")
      .waitForDisplayed({ timeout: 20000 });
    await window.$("p*=1 fiche(s) reçue(s)").waitForDisplayed({ timeout: 20000 });
    await window.$("button=Fermer").click();
    await window.waitForDisplayed({ reverse: true, timeout: 10000 });
    await expect(rows("Camille Durand")).toBeElementsArrayOfSize(1);
  });

  it("envoie sur le serveur un contact saisi ici et garde l'anniversaire d'une fiche modifiée", async () => {
    await button("Nouveau contact").click();
    const form = dialog("Nouveau contact");
    await form.waitForDisplayed();
    await field("Prénom").setValue("Dominique");
    await field("Nom").setValue("Lefèvre");
    await $('[aria-label="Adresse électronique 1"]').setValue("dominique@norelly.test");
    await button("Enregistrer").click();
    await form.waitForDisplayed({ reverse: true, timeout: 10000 });

    // Fiche du serveur modifiée ici.
    await rows("Camille Durand")[0].click();
    await button("Modifier le contact").click();
    const edit = dialog("Modifier le contact");
    await edit.waitForDisplayed();
    await field("Organisation").setValue("Atelier Durand");
    await button("Enregistrer").click();
    await edit.waitForDisplayed({ reverse: true, timeout: 10000 });

    const window = await openSync();
    await window.$("button=Synchroniser maintenant").click();
    await window.$("p*=2 envoyée(s)").waitForDisplayed({ timeout: 20000 });
    await window.$("button=Fermer").click();
    await window.waitForDisplayed({ reverse: true, timeout: 10000 });

    const cards = await remoteCards();
    expect(cards).toHaveLength(2);
    const dominique = cards.find((c) => c.includes("dominique@norelly.test"));
    expect(dominique).toContain("UID:");
    const camille = cards.find((c) => c.includes("camille.durand@norelly.test"));
    expect(camille).toContain("ORG:Atelier Durand");
    // Propriété que le carnet ne connaît pas : gardée.
    expect(camille).toContain("BDAY:19850412");
  });

  it("déconnecte le carnet distant sans toucher aux contacts", async () => {
    const window = await openSync();
    await window.$("button=Déconnecter").click();
    await window.$("button=Rechercher les carnets").waitForDisplayed({ timeout: 10000 });
    await browser.keys("Escape");
    await window.waitForDisplayed({ reverse: true, timeout: 10000 });
    await expect(rows("Camille Durand")).toBeElementsArrayOfSize(1);
    await expect(rows("Dominique Lefèvre")).toBeElementsArrayOfSize(1);
    expect(await remoteCards()).toHaveLength(2);
  });
});
