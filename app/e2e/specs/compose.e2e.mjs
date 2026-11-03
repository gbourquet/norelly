// Rédaction et envoi de bout en bout (phase 7), contre Dovecot et Mailpit :
// - « r » prépare une réponse (destinataire, sujet Re:, citation) ;
// - Ctrl+Entrée envoie : Mailpit reçoit le message (In-Reply-To, version
//   texte et HTML), et la copie arrive dans Envoyés sur Dovecot ;
// - « c » rédige un nouveau message avec un Cci : présent dans
//   l'enveloppe SMTP, absent des en-têtes transmis ;
// - le HTML saisi n'est jamais interprété (texte littéral).
import { $, browser, expect } from "@wdio/globals";
import {
  addTestAccount,
  address,
  appendMessages,
  imapMessages,
  mailpitMessages,
  mailpitRaw,
  removeTestAccount,
} from "./helpers.mjs";

const received = `From: Alice <alice@norelly.test>
To: ${address}
Subject: Préparation de la réunion
Date: Mon, 2 Nov 2026 10:00:00 +0000
Message-ID: <reunion@norelly.test>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Bonjour, peux-tu confirmer ta présence ?
`;

async function waitFor(check, message, timeout = 60000) {
  await browser.waitUntil(check, { timeout, interval: 500, timeoutMsg: message });
}

describe("rédaction et envoi", () => {
  before(async () => {
    await appendMessages([received]);
    await addTestAccount();
  });

  after(async () => {
    await removeTestAccount();
  });

  it("répond avec r et envoie avec Ctrl+Entrée", async () => {
    const row = $("*=Préparation de la réunion");
    await row.waitForDisplayed({ timeout: 90000 });
    await row.click();
    await browser.keys("Enter");
    await $("*=peux-tu confirmer").waitForExist({ timeout: 30000 });
    const sentBefore = await imapMessages("Sent");
    await browser.keys("r");
    const dialog = $("aria/Répondre");
    await dialog.waitForDisplayed({ timeout: 20000 });
    await expect($("*=alice@norelly.test")).toBeDisplayed();
    await expect($("#compose-subject")).toHaveValue("Re: Préparation de la réunion");
    const body = $('[aria-label="Corps du message"]');
    await body.click();
    await browser.keys("Oui, je serai là. <b>pas du HTML</b>");
    await browser.keys(["Control", "Enter"]);
    await $("*=Message en cours d'envoi.").waitForDisplayed({ timeout: 20000 });

    await waitFor(
      async () =>
        (await mailpitMessages()).some((m) => m.Subject === "Re: Préparation de la réunion"),
      "réponse absente de Mailpit",
    );
    const reply = (await mailpitMessages()).find(
      (m) => m.Subject === "Re: Préparation de la réunion",
    );
    expect(reply.To.map((t) => t.Address)).toEqual(["alice@norelly.test"]);
    const raw = await mailpitRaw(reply.ID);
    expect(raw).toContain("In-Reply-To: <reunion@norelly.test>");
    expect(raw).toMatch(/multipart\/alternative/);
    expect(raw).toContain("&lt;b&gt;pas du HTML&lt;/b&gt;");
    expect(raw).not.toMatch(/<b>pas du HTML<\/b>/);
    await waitFor(
      async () => (await imapMessages("Sent")) === sentBefore + 1,
      "copie absente d'Envoyés",
    );
  });

  it("rédige avec c ; le Cci reste hors des en-têtes", async () => {
    await browser.keys("Escape");
    await browser.keys("c");
    const dialog = $("aria/Nouveau message");
    await dialog.waitForDisplayed({ timeout: 20000 });
    const to = $('[aria-label="À"]');
    await to.setValue("bob@norelly.test");
    await browser.keys("Enter");
    await $("button=Cc et Cci").click();
    await $('[aria-label="Cci"]').setValue("cache@norelly.test");
    await browser.keys("Enter");
    await $("#compose-subject").setValue("Ordre du jour");
    await $('[aria-label="Corps du message"]').click();
    await browser.keys("Voici l'ordre du jour.");
    await $("button=Envoyer").click();
    await waitFor(
      async () => (await mailpitMessages()).some((m) => m.Subject === "Ordre du jour"),
      "message absent de Mailpit",
    );
    const sent = (await mailpitMessages()).find((m) => m.Subject === "Ordre du jour");
    expect(sent.Bcc.map((b) => b.Address)).toEqual(["cache@norelly.test"]);
    const raw = await mailpitRaw(sent.ID);
    expect(raw).not.toContain("cache@norelly.test");
  });
});
