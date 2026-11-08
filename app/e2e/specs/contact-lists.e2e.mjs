// Listes de diffusion de bout en bout, à la souris :
// - section « Listes » de la vue Contacts ; « Nouvelle liste » avec des
//   membres cherchés parmi les contacts et l'envoi en Cci ;
// - un nom déjà pris (à la casse près) est refusé ;
// - « Ajouter à une liste… » depuis « Plus… » de la fiche d'un contact ;
// - « Écrire à la liste » ouvre la rédaction avec ses membres ;
// - pendant la saisie d'un destinataire, la liste est proposée ; choisie,
//   elle se déplie en puces (adresse préférée de chaque membre), en Cci
//   pour une liste « en Cci » ; le message part vers Mailpit avec les
//   membres en copie cachée seulement ;
// - suppression d'une liste depuis le menu contextuel : ses contacts
//   restent.
import { $, browser, expect } from "@wdio/globals";
import {
  addTestAccount,
  mailpitMessages,
  openContextMenu,
  openMenu,
  removeTestAccount,
} from "./helpers.mjs";

/** Bouton visible dont le libellé (texte ou `aria-label`) est `name` ; la
 * copie invisible de mesure des barres d'actions est ignorée. */
const button = (name, scope = null) => {
  const xpath = `.//button[@aria-label="${name}" or normalize-space()="${name}"][not(ancestor::*[@aria-hidden="true"])]`;
  return scope === null ? $(xpath.slice(1)) : scope.$(xpath);
};

/** Champ d'un formulaire par son libellé. */
const field = (label) =>
  $(`//label[span[normalize-space()="${label}"]]//*[self::input or self::textarea]`);

const dialog = (title) => $(`//*[@role="dialog"][.//*[normalize-space()="${title}"]]`);

const menuItem = (label) =>
  $(`//*[@role="menuitem"][.//*[contains(@class, "menu-label")][normalize-space()="${label}"]]`);

const row = (name) => $(`//*[@role="option"][.//*[normalize-space()="${name}"]]`);

const chip = (scope, address) => scope.$(`.//*[contains(@class, "chip")][@title="${address}"]`);

async function waitFor(check, message, timeout = 60000) {
  await browser.waitUntil(check, { timeout, interval: 500, timeoutMsg: message });
}

/** Crée un contact à la souris (première adresse ; seconde préférée si
 * donnée). */
async function createContact(given, family, first, preferred = null) {
  await button("Nouveau contact").click();
  const form = dialog("Nouveau contact");
  await form.waitForDisplayed();
  await field("Prénom").setValue(given);
  await field("Nom").setValue(family);
  await $('[aria-label="Adresse électronique 1"]').setValue(first);
  if (preferred !== null) {
    await button("Ajouter une adresse").click();
    await $('[aria-label="Adresse électronique 2"]').setValue(preferred);
    await form.$$('input[type="radio"]')[1].click();
  }
  await button("Enregistrer").click();
  await form.waitForDisplayed({ reverse: true, timeout: 10000 });
}

/** Ajoute un membre dans le formulaire d'une liste, par la recherche. */
async function addMember(form, query, name) {
  const search = form.$('input[aria-label="Ajouter un contact"]');
  await search.setValue(query);
  const add = button(`Ajouter ${name}`, form);
  await add.waitForDisplayed({ timeout: 10000 });
  await add.click();
}

describe("listes de diffusion", () => {
  before(async () => {
    await addTestAccount();
    await $("nav").$("button=Contacts").click();
    await button("Nouveau contact").waitForDisplayed({ timeout: 10000 });
    await createContact("Ana", "Lima", "ana@norelly.test", "ana.perso@norelly.test");
    await createContact("Bruno", "Roy", "bruno@norelly.test");
  });

  after(async () => {
    await browser.keys("Escape");
    // Nettoyage : le carnet est commun à tous les comptes.
    await $("nav").$("button=Contacts").click();
    await button("Tous").click();
    for (const name of ["Ana Lima", "Bruno Roy"]) {
      if (!(await row(name).isExisting())) continue;
      await openContextMenu(row(name));
      await menuItem("Supprimer le contact").click();
      const confirm = dialog("Supprimer le contact");
      await confirm.waitForDisplayed();
      await confirm.$("button=Supprimer").click();
      await confirm.waitForDisplayed({ reverse: true, timeout: 10000 });
    }
    await $("nav").$("button*=Boîte de réception unifiée").click();
    await removeTestAccount();
  });

  it("crée une liste en Cci avec des membres cherchés parmi les contacts", async () => {
    await button("Listes").click();
    await $("p*=Aucune liste pour l'instant").waitForDisplayed({ timeout: 10000 });
    await button("Nouvelle liste").click();
    const form = dialog("Nouvelle liste");
    await form.waitForDisplayed();
    await field("Nom de la liste").setValue("Famille");
    await field("Description").setValue("<b>Cousins</b> compris");
    await form.$('input[type="checkbox"]').click();
    await addMember(form, "ana", "Ana Lima");
    await addMember(form, "bru", "Bruno Roy");
    await expect(form.$("legend")).toHaveText("Membres (2)");
    await button("Enregistrer", form).click();
    await form.waitForDisplayed({ reverse: true, timeout: 10000 });

    await expect(row("Famille")).toBeDisplayed();
    const card = $('article[aria-label="Liste Famille"]');
    await expect(card).toBeDisplayed();
    await expect(card.$("p=2 membre(s)")).toBeDisplayed();
    await expect(card.$("span*=copie cachée")).toBeDisplayed();
    // Adresse préférée de chaque membre.
    await expect(card.$("span=ana.perso@norelly.test")).toBeDisplayed();
    // Saisie affichée telle quelle, jamais interprétée.
    await expect(card.$('.//p[normalize-space()="<b>Cousins</b> compris"]')).toBeDisplayed();
    await expect(card.$("b")).not.toBeExisting();
  });

  it("refuse un nom déjà pris, à la casse près", async () => {
    await button("Nouvelle liste").click();
    const form = dialog("Nouvelle liste");
    await form.waitForDisplayed();
    await field("Nom de la liste").setValue("FAMILLE");
    await button("Enregistrer", form).click();
    await expect(form.$('[role="alert"]')).toHaveText(
      expect.stringContaining("Une autre liste porte déjà ce nom"),
    );
    // Autre nom : accepté (liste vide).
    await field("Nom de la liste").setValue("Club");
    await button("Enregistrer", form).click();
    await form.waitForDisplayed({ reverse: true, timeout: 10000 });
    await expect($('article[aria-label="Liste Club"]').$("p=0 membre(s)")).toBeDisplayed();
  });

  it("ajoute un contact à une liste depuis sa fiche (« Plus… »)", async () => {
    await button("Tous").click();
    await row("Bruno Roy").click();
    await $('article[aria-label="Fiche de Bruno Roy"]').waitForDisplayed();
    await openMenu(button("Plus…"));
    await menuItem("Ajouter à une liste…").click();
    const add = dialog("Ajouter à une liste");
    await add.waitForDisplayed();
    await expect(add.$("p*=Bruno Roy")).toBeDisplayed();
    await add.$('.//label[.//span[normalize-space()="Club"]]').click();
    await button("Ajouter", add).click();
    await add.waitForDisplayed({ reverse: true, timeout: 10000 });
    await button("Listes").click();
    await expect(row("Club").$("span=1 membre(s)")).toBeDisplayed();
  });

  it("écrit à la liste depuis sa fiche", async () => {
    await row("Club").click();
    await $('article[aria-label="Liste Club"]').waitForDisplayed();
    await button("Écrire à la liste").click();
    const compose = dialog("Nouveau message");
    await compose.waitForDisplayed({ timeout: 20000 });
    await expect(chip(compose, "bruno@norelly.test")).toHaveText("Bruno Roy <bruno@norelly.test>");
    await button("Supprimer le brouillon", compose).click();
    await compose.waitForDisplayed({ reverse: true, timeout: 20000 });
  });

  it("propose la liste à la saisie et l'envoie en Cci vers Mailpit", async () => {
    await $("nav").$("button*=Boîte de réception unifiée").click();
    await button("Nouveau message").click();
    const compose = dialog("Nouveau message");
    await compose.waitForDisplayed({ timeout: 20000 });
    const to = compose.$('[aria-label="À"]');
    await to.click();
    // WebKitWebDriver perd les lettres doublées d'une frappe : « fam ».
    await browser.keys("fam");
    const option = compose.$('//*[@role="option"][.//*[normalize-space()="Famille"]]');
    await option.waitForDisplayed({ timeout: 10000 });
    await expect(option).toHaveText(expect.stringContaining("2 membre(s)"));
    await expect(option).toHaveText(expect.stringContaining("Cci"));
    await option.$("button").click();
    // Liste « en Cci » : les puces vont dans Cci, l'adresse préférée de
    // chaque membre.
    const bcc = compose.$('[aria-label="Cci"]');
    await bcc.waitForDisplayed({ timeout: 10000 });
    await expect(chip(compose, "ana.perso@norelly.test")).toHaveText(
      "Ana Lima <ana.perso@norelly.test>",
    );
    await expect(chip(compose, "bruno@norelly.test")).toBeDisplayed();
    await expect(chip(compose, "ana@norelly.test")).not.toBeExisting();
    await expect(to).toHaveValue("");
    await compose.$("#compose-subject").setValue("Nouvelles de la famille");
    await compose.$("button*=Envoyer").click();
    await compose.waitForDisplayed({ reverse: true, timeout: 20000 });

    await waitFor(
      async () => (await mailpitMessages()).some((m) => m.Subject === "Nouvelles de la famille"),
      "message absent de Mailpit",
    );
    const sent = (await mailpitMessages()).find((m) => m.Subject === "Nouvelles de la famille");
    expect(sent.Bcc.map((b) => b.Address).sort()).toEqual([
      "ana.perso@norelly.test",
      "bruno@norelly.test",
    ]);
    expect(sent.To ?? []).toEqual([]);
    expect(sent.Cc ?? []).toEqual([]);
  });

  it("supprime une liste depuis le menu contextuel ; ses contacts restent", async () => {
    await $("nav").$("button=Contacts").click();
    await button("Listes").click();
    for (const name of ["Club", "Famille"]) {
      await openContextMenu(row(name));
      await menuItem("Supprimer la liste").click();
      const confirm = dialog("Supprimer la liste ?");
      await confirm.waitForDisplayed();
      await expect(confirm.$(`p*=${name}`)).toBeDisplayed();
      await confirm.$("button=Supprimer").click();
      await confirm.waitForDisplayed({ reverse: true, timeout: 10000 });
    }
    await $("p*=Aucune liste pour l'instant").waitForDisplayed({ timeout: 10000 });
    await button("Tous").click();
    await expect(row("Ana Lima")).toBeDisplayed();
    await expect(row("Bruno Roy")).toBeDisplayed();
  });
});
