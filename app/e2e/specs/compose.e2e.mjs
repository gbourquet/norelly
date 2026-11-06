// Rédaction et envoi de bout en bout, contre Dovecot et Mailpit :
// - « r » prépare une réponse (destinataire, sujet Re:, citation) ;
// - Ctrl+Entrée envoie : Mailpit reçoit le message (In-Reply-To, version
//   texte et HTML), et la copie arrive dans Envoyés sur Dovecot ;
// - « c » rédige un nouveau message avec un Cci : présent dans
//   l'enveloppe SMTP, ni en « À » ni en « Cc » (Mailpit reconstruit
//   lui-même un en-tête Bcc depuis l'enveloppe : l'absence de Bcc dans les
//   octets transmis est vérifiée par les tests de crates/adapters/render) ;
// - le message déposé est daté avant ceux de reading-security.e2e.mjs
//   (même serveur) pour ne pas changer leur ordre dans la liste ;
// - le HTML saisi n'est jamais interprété (texte littéral).
import { $, browser, expect } from "@wdio/globals";
import {
  addTestAccount,
  address,
  appendMessages,
  imapMessages,
  mailpitHeaders,
  mailpitMessage,
  mailpitMessages,
  removeTestAccount,
} from "./helpers.mjs";

const received = `From: Alice <alice@norelly.test>
To: ${address}
Subject: Préparation de la réunion
Date: Mon, 2 Nov 2026 08:00:00 +0000
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
    // Aucune fenêtre ouverte ne doit bloquer le retrait du compte.
    await browser.keys("Escape");
    await browser.keys("Escape");
    await removeTestAccount();
  });

  it("répond avec r et envoie avec Ctrl+Entrée", async () => {
    const row = $("span=Préparation de la réunion");
    await row.waitForDisplayed({ timeout: 90000 });
    const sentBefore = await imapMessages("Sent");
    await row.click();
    await expect($("h1=Préparation de la réunion")).toBeDisplayed({ wait: 30000 });
    await browser.keys("r");
    const dialog = $('[role="dialog"]');
    await dialog.waitForDisplayed({ timeout: 20000 });
    await expect(dialog.$('span[title="alice@norelly.test"]')).toBeDisplayed();
    await expect($("#compose-subject")).toHaveValue("Re: Préparation de la réunion");
    await dialog.$('[aria-label="Corps du message"]').click();
    await browser.keys("Oui, je serai là. <b>pas du HTML</b>");
    await browser.keys(["Control", "Enter"]);
    await dialog.waitForDisplayed({ reverse: true, timeout: 20000 });

    await waitFor(
      async () =>
        (await mailpitMessages()).some((m) => m.Subject === "Re: Préparation de la réunion"),
      "réponse absente de Mailpit",
    );
    const reply = (await mailpitMessages()).find(
      (m) => m.Subject === "Re: Préparation de la réunion",
    );
    expect(reply.To.map((t) => t.Address)).toEqual(["alice@norelly.test"]);
    const headers = await mailpitHeaders(reply.ID);
    expect(headers["In-Reply-To"]).toEqual(["<reunion@norelly.test>"]);
    const message = await mailpitMessage(reply.ID);
    expect(message.Text).toContain("<b>pas du HTML</b>");
    expect(message.Text).toContain("> Bonjour, peux-tu confirmer ta présence ?");
    expect(message.HTML).toContain("&lt;b&gt;pas du HTML&lt;/b&gt;");
    expect(message.HTML).not.toContain("<b>pas du HTML");
    expect(message.HTML).toContain("<blockquote>");
    await waitFor(
      async () => (await imapMessages("Sent")) === sentBefore + 1,
      "copie absente d'Envoyés",
    );
  });

  it("rédige avec c ; le Cci reste hors des en-têtes", async () => {
    await browser.keys("c");
    const dialog = $('[role="dialog"]');
    await dialog.waitForDisplayed({ timeout: 20000 });
    await dialog.$('[aria-label="À"]').setValue("bob@norelly.test");
    await browser.keys("Enter");
    await dialog.$("button=Cc et Cci").click();
    await dialog.$('[aria-label="Cci"]').setValue("cache@norelly.test");
    await browser.keys("Enter");
    await $("#compose-subject").setValue("Ordre du jour");
    await dialog.$('[aria-label="Corps du message"]').click();
    await browser.keys("Voici l'ordre du jour.");
    await dialog.$("button*=Envoyer").click();
    await dialog.waitForDisplayed({ reverse: true, timeout: 20000 });
    await waitFor(
      async () => (await mailpitMessages()).some((m) => m.Subject === "Ordre du jour"),
      "message absent de Mailpit",
    );
    const sent = (await mailpitMessages()).find((m) => m.Subject === "Ordre du jour");
    expect(sent.Bcc.map((b) => b.Address)).toEqual(["cache@norelly.test"]);
    expect(sent.To.map((t) => t.Address)).toEqual(["bob@norelly.test"]);
    expect(sent.Cc ?? []).toEqual([]);
    const headers = await mailpitHeaders(sent.ID);
    expect(headers["Message-Id"]?.[0]).toMatch(/^<[0-9a-f]{32}@norelly\.test>$/);
  });
});
