// Recherche plein texte de bout en bout (phase 8), contre Dovecot :
// - « / » ouvre la barre ; un mot présent seulement dans le texte du
//   message le trouve (texte indexé à la synchronisation), surligné dans
//   l'extrait ;
// - Entrée ouvre la conversation du résultat, Échap revient aux résultats
//   puis à la liste ;
// - un sujet qui contient du HTML est affiché comme du texte ;
// - le message est daté avant ceux des autres scénarios (même serveur)
//   pour ne pas changer leur ordre dans la liste.
import { $, browser, expect } from "@wdio/globals";
import { addTestAccount, address, appendMessages, removeTestAccount } from "./helpers.mjs";

const SUBJECT = "Commande de printemps <b>gras</b>";
/** Sujet tel qu'il doit apparaître dans le HTML : échappé, jamais une balise. */
const ESCAPED = "Commande de printemps &lt;b&gt;gras&lt;/b&gt;";

const message = `From: Bruno <bruno@norelly.test>
To: ${address}
Subject: ${SUBJECT}
Date: Thu, 1 Oct 2026 08:00:00 +0000
Message-ID: <printemps@norelly.test>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Bonjour, le cerisier du jardin est en fleurs depuis lundi.
`;

describe("recherche", () => {
  before(async () => {
    await appendMessages([message]);
    await addTestAccount();
  });

  after(async () => {
    await browser.keys("Escape");
    await browser.keys("Escape");
    await removeTestAccount();
  });

  it("trouve un mot du texte, surligné, et ouvre le résultat", async () => {
    // Sujet affiché littéralement dans la liste (jamais interprété).
    const row = $("span*=Commande de printemps");
    await row.waitForDisplayed({ timeout: 90000 });
    expect(await row.getHTML()).toContain(ESCAPED);
    await expect(row.$("b")).not.toBeExisting();
    await browser.keys("/");
    const box = $('input[type="search"]');
    await box.waitForDisplayed({ timeout: 10000 });
    await expect(box).toBeFocused();

    // Le texte du corps est indexé quand il est téléchargé : on relance
    // la recherche jusqu'à ce qu'il le soit.
    const results = $('[role="listbox"][aria-label="Résultats de la recherche"]');
    const highlighted = results.$("mark=cerisier");
    await browser.waitUntil(
      async () => {
        await box.click();
        await box.clearValue();
        await box.setValue("CERISIER");
        await browser.keys("Enter");
        return highlighted.isDisplayed();
      },
      { timeout: 60000, interval: 1000, timeoutMsg: "mot du texte introuvable" },
    );
    await expect(results.$$('[role="option"]')).toBeElementsArrayOfSize(1);
    await expect(results.$("b")).not.toBeExisting();

    // Entrée dans la barre a placé le focus sur les résultats.
    await expect(results).toBeFocused();
    await browser.keys("Enter");
    const title = $("h1*=Commande de printemps");
    await expect(title).toBeDisplayed({ wait: 30000 });
    expect(await title.getHTML()).toContain(ESCAPED);
    await browser.keys("Escape");
    await expect(results).toBeDisplayed();
    await browser.keys("Escape");
    await box.waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("filtre par expéditeur et signale l'absence de résultat", async () => {
    await browser.keys("/");
    const box = $('input[type="search"]');
    await box.waitForDisplayed({ timeout: 10000 });
    await box.setValue("from:bruno printemps");
    await browser.keys("Enter");
    const results = $('[role="listbox"][aria-label="Résultats de la recherche"]');
    await expect(results.$("mark=printemps")).toBeDisplayed({ wait: 10000 });
    await box.click();
    await box.clearValue();
    await box.setValue("from:alice printemps");
    await browser.keys("Enter");
    await expect($("p=Aucun message trouvé.")).toBeDisplayed({ wait: 10000 });
    await browser.keys("Escape");
    await box.waitForDisplayed({ reverse: true, timeout: 10000 });
  });
});
