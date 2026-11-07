// Carnet d'adresses de bout en bout, à la souris :
// - « Contacts » du panneau de gauche ouvre la vue ; « Nouveau contact »
//   crée une fiche à deux adresses dont la seconde est préférée ;
// - une adresse déjà dans le carnet (à la casse près) est refusée ;
// - « Modifier le contact », le favori (« Plus… ») et le filtre des
//   favoris ;
// - « Écrire au contact » ouvre la rédaction avec l'adresse préférée ;
// - un texte saisi qui ressemble à du HTML est affiché comme du texte ;
// - le nom du carnet remplace celui de l'en-tête dans la liste et le
//   message ; un nom d'en-tête qui imite une adresse n'est pas affiché ;
//   « Voir le contact » depuis le nom de l'expéditeur ;
// - « Ajouter au carnet d'adresses » depuis le clic droit sur une
//   conversation, fiche pré-remplie ;
// - suppression depuis le menu contextuel d'une ligne, avec confirmation.
import { $, browser, expect } from "@wdio/globals";
import {
  addTestAccount,
  address,
  appendMessages,
  openContextMenu,
  openMenu,
  removeTestAccount,
} from "./helpers.mjs";

/** Message brut reçu de `from`. */
const mail = (from, subject, id) => `From: ${from}
To: ${address}
Subject: ${subject}
Date: Mon, 2 Nov 2026 08:00:00 +0000
Message-ID: <${id}@norelly.test>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Bonjour.
`;

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

/** Retour au courrier par le panneau de gauche. */
const inbox = () => $("nav").$("button*=Boîte de réception unifiée").click();

describe("carnet d'adresses", () => {
  before(async () => {
    await appendMessages([
      mail("Z. M. <Zoe@norelly.test>", "Réunion du carnet", "carnet-zoe"),
      mail("Hugo Petit <hugo@norelly.test>", "Prêt du carnet de chant", "carnet-hugo"),
      mail('"service@banque.test" <pirate@norelly.test>', "Votre carnet bancaire", "carnet-faux"),
    ]);
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

  it("affiche le nom du carnet dans la liste et le message, puis la fiche", async () => {
    await inbox();
    const known = row("Réunion du carnet");
    await known.waitForDisplayed({ timeout: 90000 });
    // Adresse de l'en-tête à la casse près : le nom du carnet l'emporte.
    await expect(known.$('.//*[normalize-space()="Zoé Martin"]')).toBeDisplayed();
    // Nom d'en-tête qui imite une adresse : seule l'adresse réelle.
    const fake = row("Votre carnet bancaire");
    await expect(fake.$('.//*[normalize-space()="pirate@norelly.test"]')).toBeDisplayed();
    await expect(
      fake.$('.//*[contains(normalize-space(), "service@banque.test")]'),
    ).not.toBeExisting();

    await $("span=Réunion du carnet").click();
    await openMenu(button("Actions pour Zoé Martin (Dans vos contacts)"));
    await menuItem("Voir le contact").click();
    await expect($('article[aria-label="Fiche de Zoé Martin"]')).toBeDisplayed({ wait: 10000 });
  });

  it("ajoute au carnet l'expéditeur d'une conversation (clic droit)", async () => {
    await inbox();
    await openContextMenu(row("Prêt du carnet de chant"));
    await menuItem("Ajouter au carnet d'adresses").click();
    const form = dialog("Nouveau contact");
    await form.waitForDisplayed();
    await expect(field("Nom affiché")).toHaveValue("Hugo Petit");
    await expect($('[aria-label="Adresse électronique 1"]')).toHaveValue("hugo@norelly.test");
    await button("Enregistrer").click();
    await form.waitForDisplayed({ reverse: true, timeout: 10000 });

    // La conversation le signale désormais comme contact.
    await $("span=Prêt du carnet de chant").click();
    await expect(button("Actions pour Hugo Petit (Dans vos contacts)")).toBeDisplayed({
      wait: 10000,
    });

    // Nettoyage : le carnet est commun à tous les comptes.
    await $("nav").$("button=Contacts").click();
    await openContextMenu(row("Hugo Petit"));
    await menuItem("Supprimer le contact").click();
    const confirm = dialog("Supprimer le contact");
    await confirm.waitForDisplayed();
    await confirm.$("button=Supprimer").click();
    await confirm.waitForDisplayed({ reverse: true, timeout: 10000 });
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
    await inbox();
  });
});
