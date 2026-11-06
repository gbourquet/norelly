// Ajout d'un compte de bout en bout contre Dovecot (IMAP) et Mailpit (SMTP)
// du réseau de test : découverte sans résultat, saisie manuelle,
// avertissements de certificat (autosignés), confiance explicite, compte
// enregistré (base SQLite et trousseau GNOME Keyring), puis suppression.
import { $, browser, expect } from "@wdio/globals";
import { accountMenu } from "./helpers.mjs";

const password = process.env.NORELLY_E2E_PASSWORD ?? "";
const address = "norelly@norelly.test";
const FINGERPRINT = /^([0-9A-F]{2}:){31}[0-9A-F]{2}$/;

describe("compte de messagerie", () => {
  it("démarre en français (locale imposée par run.sh, ADR-011)", async () => {
    expect(await browser.execute(() => document.documentElement.lang)).toBe("fr");
  });

  it("démarre sans compte", async () => {
    await expect($("h1=Norelly")).toBeDisplayed();
    await expect($("p=Aucun compte pour l'instant.")).toBeDisplayed();
  });

  it("ajoute un compte Dovecot après confiance explicite dans les certificats", async () => {
    await $("main").$("button=Ajouter un compte").click();
    await $("#account-address").setValue(address);
    await $("#account-password").setValue(password);
    await $("button=Continuer").click();

    // Aucune source automatique ne connaît norelly.test : saisie manuelle
    // pré-remplie.
    const imapHost = $("#imap-host");
    await imapHost.waitForDisplayed({ timeout: 90000 });
    await expect(imapHost).toHaveValue("imap.norelly.test");
    await expect($("#smtp-host")).toHaveValue("smtp.norelly.test");
    // Ports de l'image Dovecot (mode confiné) et de Mailpit.
    await $("#imap-port").setValue("31993");
    await $("#imap-username").setValue("norelly");
    await $("#smtp-port").setValue("1025");
    await $("#smtp-username").setValue("norelly");
    await $("button=Tester et ajouter").click();

    // Deux certificats autosignés (IMAP puis SMTP) : avertissement, empreinte
    // affichée, confiance explicite.
    for (const host of ["imap.norelly.test", "smtp.norelly.test"]) {
      const trust = $("button=Faire confiance à ce serveur");
      await trust.waitForDisplayed({ timeout: 60000 });
      await expect($(".warning")).toHaveText(expect.stringContaining(host));
      await expect($(".fingerprint")).toHaveText(FINGERPRINT);
      await trust.click();
    }

    // Compte ajouté : la fenêtre se ferme et le compte apparaît.
    const entry = $(`span=${address}`);
    await entry.waitForDisplayed({ timeout: 60000 });
    await expect($("#imap-host")).not.toBeDisplayed();
  });

  it("conserve le compte après rechargement", async () => {
    await browser.refresh();
    await $(`span=${address}`).waitForDisplayed({ timeout: 20000 });
  });

  it("refuse un second compte pour la même adresse", async () => {
    await $("nav").$("button=Ajouter un compte").click();
    await $("#account-address").setValue(address);
    await $("#account-password").setValue(password);
    await $("button=Continuer").click();
    await $("#imap-host").waitForDisplayed({ timeout: 90000 });
    await $("#imap-port").setValue("31993");
    await $("#imap-username").setValue("norelly");
    await $("#smtp-port").setValue("1025");
    await $("#smtp-username").setValue("norelly");
    await $("button=Tester et ajouter").click();
    await expect($("p*=existe déjà")).toBeDisplayed({ wait: 30000 });
    await browser.keys("Escape");
    await $("#account-address").waitForDisplayed({ reverse: true });
  });

  it("supprime le compte après confirmation", async () => {
    await accountMenu(address, "Supprimer ce compte…");
    const confirm = $("button=Supprimer le compte");
    await confirm.waitForDisplayed();
    await confirm.click();
    await $("p=Aucun compte pour l'instant.").waitForDisplayed({ timeout: 20000 });
    await expect($(`span=${address}`)).not.toBeExisting();
  });
});
