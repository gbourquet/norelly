// Carnet d'adresses de bout en bout, à la souris :
// - « Contacts » du panneau de gauche ouvre la vue ; « Nouveau contact »
//   crée une fiche à deux adresses dont la seconde est préférée ;
// - une adresse déjà dans le carnet (à la casse près) est refusée ;
// - « Modifier le contact », le favori (« Plus… ») et le filtre des
//   favoris ;
// - « Écrire au contact » ouvre la rédaction avec l'adresse préférée ;
// - un texte saisi qui ressemble à du HTML est affiché comme du texte ;
// - suppression depuis le menu contextuel d'une ligne, avec confirmation.
import { $, browser, expect } from "@wdio/globals";
import { addTestAccount, openContextMenu, openMenu, removeTestAccount } from "./helpers.mjs";

/** Bouton visible dont le libellé (texte ou `aria-label`) est `name` ; la
 * copie invisible de mesure des barres d'actions est ignorée. */
const button = (name) =>
  $(
    `//button[@aria-label="${name}" or normalize-space()="${name}"][not(ancestor::*[@aria-hidden="true"])]`,
  );

/** Champ d'un formulaire par son libellé. */
const field = (label) =>
  $(`//label[span[normalize-space()="${label}"]]//*[self::input or self::textarea]`);

const dialog = (title) => $(`//*[@role="dialog"][.//*[normalize-space()="${title}"]]`);

const menuItem = (label) =>
  $(`//*[@role="menuitem"][.//*[contains(@class, "menu-label")][normalize-space()="${label}"]]`);

const row = (name) => $(`//*[@role="option"][.//*[normalize-space()="${name}"]]`);

describe("carnet d'adresses", () => {
  before(async () => {
    await addTestAccount();
  });

  after(async () => {
    // Retour au courrier (l'accueil sans compte s'y affiche), même après
    // un échec en cours de scénario.
    await browser.keys("Escape");
    await $("nav").$("button*=Boîte de réception unifiée").click();
    await removeTestAccount();
  });

  it("crée un contact à deux adresses, la seconde préférée", async () => {
    await $("nav").$("button=Contacts").click();
    await $("p*=Aucun contact pour l'instant").waitForDisplayed({ timeout: 10000 });
    await button("Nouveau contact").click();
    const form = dialog("Nouveau contact");
    await form.waitForDisplayed();
    await field("Prénom").setValue("Zoé");
    await field("Nom").setValue("Martin");
    await field("Organisation").setValue("<i>ACME</i>");
    await $('[aria-label="Adresse électronique 1"]').setValue("zoe@norelly.test");
    await button("Ajouter une adresse").click();
    await $('[aria-label="Adresse électronique 2"]').setValue("zoe.pro@norelly.test");
    await form.$$('input[type="radio"]')[1].click();
    await button("Enregistrer").click();
    await form.waitForDisplayed({ reverse: true, timeout: 10000 });

    await expect(row("Zoé Martin")).toBeDisplayed();
    const card = $('article[aria-label="Fiche de Zoé Martin"]');
    await expect(card).toBeDisplayed();
    await expect(card.$("span=zoe.pro@norelly.test")).toBeDisplayed();
    // Saisie affichée telle quelle, jamais interprétée.
    await expect(card.$('.//p[normalize-space()="<i>ACME</i>"]')).toBeDisplayed();
    await expect(card.$("i")).not.toBeExisting();
  });

  it("refuse une adresse déjà dans le carnet", async () => {
    await button("Nouveau contact").click();
    const form = dialog("Nouveau contact");
    await form.waitForDisplayed();
    await $('[aria-label="Adresse électronique 1"]').setValue("ZOE@norelly.test");
    await button("Enregistrer").click();
    await expect(form.$('[role="alert"]')).toHaveText(
      expect.stringContaining("appartient déjà à un autre contact"),
    );
    await button("Annuler").click();
    await form.waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("modifie la fiche, l'ajoute aux favoris et filtre", async () => {
    const card = $('article[aria-label="Fiche de Zoé Martin"]');
    await button("Modifier le contact").click();
    const form = dialog("Modifier le contact");
    await form.waitForDisplayed();
    await field("Surnom").setValue("Zozo");
    await button("Enregistrer").click();
    await form.waitForDisplayed({ reverse: true, timeout: 10000 });
    await expect(card.$("p=Surnom : Zozo")).toBeDisplayed();

    await openMenu(button("Plus…"));
    await menuItem("Ajouter aux favoris").click();
    await expect(row("Zoé Martin").$('[aria-label="Favori"]')).toBeDisplayed();
    await button("Favoris").click();
    await expect(row("Zoé Martin")).toBeDisplayed();
    await button("Tous").click();
  });

  it("écrit au contact depuis sa fiche", async () => {
    await button("Écrire au contact").click();
    const compose = $('//*[@role="dialog"][.//*[normalize-space()="Nouveau message"]]');
    await compose.waitForDisplayed({ timeout: 20000 });
    await expect(
      compose.$('.//*[contains(@class, "chip")][@title="zoe.pro@norelly.test"]'),
    ).toBeDisplayed();
    await browser.keys("Escape");
    await compose.waitForDisplayed({ reverse: true, timeout: 20000 });
  });

  it("supprime le contact depuis le menu contextuel", async () => {
    await openContextMenu(row("Zoé Martin"));
    await menuItem("Supprimer le contact").click();
    const confirm = dialog("Supprimer le contact");
    await confirm.waitForDisplayed();
    await expect(confirm.$("p*=Zoé Martin")).toBeDisplayed();
    await confirm.$("button=Supprimer").click();
    await confirm.waitForDisplayed({ reverse: true, timeout: 10000 });
    await $("p*=Aucun contact pour l'instant").waitForDisplayed({ timeout: 10000 });
    // Retour au courrier par le panneau de gauche.
    await $("nav").$("button*=Boîte de réception unifiée").click();
  });
});
