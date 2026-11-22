// Capture large de la boîte de réception en thème sombre (`inbox-dark`,
// page d'accueil du site et guide), une fois par langue, lancée par
// in-container.sh avant captures.e2e.mjs, sur un profil neuf et avec un
// thème GTK sombre (GTK_THEME=Adwaita:dark, que WebKitGTK traduit en
// `prefers-color-scheme: dark`). Même compte, mêmes messages et même
// taille de fenêtre que la capture `inbox` claire (shared.mjs).
//
// Les messages déposés dans la boîte Dovecot de la capture en sont retirés
// à la fin : captures.e2e.mjs la retrouve vide et la remplit à son tour.
import { $, browser, expect } from "@wdio/globals";
import { appendMessages, imapCommands } from "../specs/helpers.mjs";
import {
  addCaptureAccount,
  capture,
  captureWindowSize,
  content,
  lang,
  msg,
  rawMessages,
  user,
} from "./shared.mjs";

describe(`capture en thème sombre (${lang})`, () => {
  before(async () => {
    await captureWindowSize();
    await appendMessages(rawMessages(), user);
  });

  after(async () => {
    const out = await imapCommands(
      ["SELECT INBOX", "STORE 1:* +FLAGS.SILENT (\\Deleted)", "EXPUNGE"],
      user,
    );
    if (!/^c2 OK/m.test(out)) throw new Error(`boîte de la capture non vidée : ${out}`);
  });

  it("boîte de réception", async () => {
    await $(`p=${msg("welcome_no_account")}`).waitForDisplayed({ timeout: 20000 });
    expect(await browser.execute(() => document.documentElement.lang)).toBe(lang);
    // Thème sombre réellement appliqué : sinon la capture serait claire
    // sans que rien ne le signale.
    expect(
      await browser.execute(() => window.matchMedia("(prefers-color-scheme: dark)").matches),
    ).toBe(true);
    await addCaptureAccount(async () => {});
    await $(`span=${content.messages[0][1]}`).waitForDisplayed({ timeout: 90000 });
    await $(`span=${content.messages.at(-1)[1]}`).waitForDisplayed({ timeout: 60000 });
    await capture("inbox-dark");
  });
});
