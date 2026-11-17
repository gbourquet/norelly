// Étiquettes de bout en bout, contre Dovecot, à la souris :
// - « Plus… » → « Étiqueter… » sur la conversation ouverte : création d'une
//   étiquette au nom non ASCII, posée tout de suite ;
// - pastille sous le sujet et dans la ligne de la liste ; mot-clé encodé
//   (atom ASCII) posé sur le serveur après la fenêtre d'annulation ;
// - clic sur la pastille de la ligne : recherche `étiquette:"…"` ;
// - l'étiquette décochée disparaît, en local puis sur le serveur ;
// - le message est daté avant ceux des autres scénarios (même serveur).
import { $, browser, expect } from "@wdio/globals";
import {
  addTestAccount,
  address,
  appendMessages,
  barAction,
  imapCommands,
  removeTestAccount,
} from "./helpers.mjs";

const SUBJECT = "Inventaire du grenier";
const LABEL = "Été 2026";
/** Mot-clé IMAP du nom (UTF-7 modifié étendu : voir docs/technique). */
const KEYWORD = "&AMk-t&AOkAIA-2026";

const message = `From: Chloé <chloe@norelly.test>
To: ${address}
Subject: ${SUBJECT}
Date: Wed, 30 Sep 2026 08:00:00 +0000
Message-ID: <grenier@norelly.test>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Il reste trois cartons de livres et la vieille lampe.
`;

/** UID de la boîte de réception qui portent le mot-clé, côté serveur. */
async function serverHas() {
  const reply = await imapCommands(["EXAMINE INBOX", `UID SEARCH KEYWORD ${KEYWORD}`]);
  return /^\* SEARCH \d+/m.test(reply);
}

const chipName = `Rechercher les messages étiquetés « ${LABEL} »`;

describe("étiquettes", () => {
  before(async () => {
    await appendMessages([message]);
    await addTestAccount();
  });

  after(async () => {
    await browser.keys("Escape");
    await browser.keys("Escape");
    await removeTestAccount();
  });

  it("étiquette une conversation depuis « Plus… », l'affiche, la cherche et la retire", async () => {
    const row = $(`span=${SUBJECT}`);
    await row.waitForDisplayed({ timeout: 90000 });
    await row.click();
    const reader = $('article[aria-label="Conversation"]');
    await expect(reader.$(`h1=${SUBJECT}`)).toBeDisplayed({ wait: 30000 });

    // « Plus… » → « Étiqueter… » : nouvelle étiquette, posée d'un coup.
    await barAction(reader.$('[role="toolbar"]'), "Étiqueter…");
    const dialog = $('//*[@role="dialog"][.//*[normalize-space()="Étiqueter"]]');
    await dialog.waitForDisplayed({ timeout: 10000 });
    await dialog.$('input[aria-label="Nouvelle étiquette"]').setValue(LABEL);
    await dialog.$("button=Créer et poser").click();
    const box = dialog.$('input[type="checkbox"]');
    await box.waitForDisplayed({ timeout: 10000 });
    await expect(box).toBeChecked();
    await dialog.$("button=Terminé").click();
    await dialog.waitForDisplayed({ reverse: true, timeout: 10000 });

    // Pastille sous le sujet, puis sur le serveur (mot-clé encodé).
    const labels = reader.$('ul[aria-label="Étiquettes"]');
    await expect(labels.$(`button[aria-label="${chipName}"]`)).toBeDisplayed({ wait: 10000 });
    await browser.waitUntil(serverHas, {
      timeout: 60000,
      interval: 1000,
      timeoutMsg: "mot-clé absent du serveur",
    });

    // Pastille dans la ligne ; un clic lance la recherche de l'étiquette.
    await browser.keys("Escape");
    const listRow = $(`//*[@role="option"][.//span[normalize-space()="${SUBJECT}"]]`);
    const chip = listRow.$(`button[aria-label="${chipName}"]`);
    await chip.waitForDisplayed({ timeout: 10000 });
    expect(await chip.getProperty("textContent")).toBe(LABEL);
    // Pastille réellement visible (pas écrasée par le sujet).
    expect((await chip.getSize()).width).toBeGreaterThan(30);
    await chip.click();
    const search = $('input[type="search"]');
    await search.waitForDisplayed({ timeout: 10000 });
    await expect(search).toHaveValue(`étiquette:"${LABEL}"`);
    const results = $('[role="listbox"][aria-label="Résultats de la recherche"]');
    await expect(results.$$('[role="option"]')).toBeElementsArrayOfSize(1, { wait: 20000 });
    await browser.keys("Escape");
    await search.waitForDisplayed({ reverse: true, timeout: 10000 });

    // Retirer l'étiquette : décochée dans le menu.
    await $(`span=${SUBJECT}`).click();
    await expect(reader.$(`h1=${SUBJECT}`)).toBeDisplayed({ wait: 30000 });
    await barAction(reader.$('[role="toolbar"]'), "Étiqueter…");
    await dialog.waitForDisplayed({ timeout: 10000 });
    await expect(box).toBeChecked({ wait: 10000 });
    await box.click();
    await expect(box).not.toBeChecked({ wait: 10000 });
    await dialog.$("button=Terminé").click();
    await dialog.waitForDisplayed({ reverse: true, timeout: 10000 });
    await expect(labels).not.toBeDisplayed({ wait: 10000 });
    await browser.waitUntil(async () => !(await serverHas()), {
      timeout: 60000,
      interval: 1000,
      timeoutMsg: "mot-clé encore sur le serveur",
    });
  });
});
