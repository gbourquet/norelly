// Outils partagés des scénarios end-to-end : ajout de compte par
// l'interface, dépôt de messages sur Dovecot (APPEND), compteur du
// serveur canari.
import tls from "node:tls";
import { $, browser, expect } from "@wdio/globals";

export const password = process.env.NORELLY_E2E_PASSWORD ?? "";
export const address = "norelly@norelly.test";
const canary = process.env.NORELLY_E2E_CANARY ?? "http://canary.norelly.test:8080";

/** Ajoute le compte de test (saisie manuelle, confiance dans les certificats si demandée). */
export async function addTestAccount() {
  // Sans compte, le bouton de l'accueil (l'icône du panneau porte le même nom).
  await $("button=Ajouter un compte").click();
  await $("#account-address").setValue(address);
  await $("#account-password").setValue(password);
  await $("button=Continuer").click();
  const imapHost = $("#imap-host");
  await imapHost.waitForDisplayed({ timeout: 90000 });
  await $("#imap-port").setValue("31993");
  await $("#imap-username").setValue("norelly");
  await $("#smtp-port").setValue("1025");
  await $("#smtp-username").setValue("norelly");
  await $("button=Tester et ajouter").click();
  // Certificats autosignés : confiance explicite si l'exception n'existe pas déjà.
  const entry = $(`span=${address}`);
  for (let i = 0; i < 2; i++) {
    const trust = $("button=Faire confiance à ce serveur");
    await browser.waitUntil(
      async () => (await trust.isDisplayed()) || (await entry.isDisplayed()),
      {
        timeout: 60000,
      },
    );
    if (await entry.isDisplayed()) break;
    await trust.click();
  }
  await entry.waitForDisplayed({ timeout: 60000 });
  await expect($("#imap-host")).not.toBeDisplayed();
}

/** Supprime le compte de test. */
export async function removeTestAccount() {
  await $(`aria/Supprimer le compte ${address}`).click();
  const confirm = $("button=Supprimer le compte");
  await confirm.waitForDisplayed();
  await confirm.click();
  await $("p=Aucun compte pour l'instant.").waitForDisplayed({ timeout: 20000 });
}

/** Dépose des messages bruts dans la boîte de réception (IMAP APPEND, TLS). */
export async function appendMessages(messages) {
  const socket = tls.connect({
    host: "imap.norelly.test",
    port: 31993,
    servername: "imap.norelly.test",
    // Certificat autosigné du réseau de test.
    rejectUnauthorized: false,
  });
  let buffer = "";
  socket.setEncoding("utf8");
  socket.on("data", (d) => (buffer += d));
  const waitFor = async (pattern) => {
    const deadline = Date.now() + 20000;
    while (!pattern.test(buffer)) {
      if (Date.now() > deadline) throw new Error(`IMAP : ${pattern} non reçu : ${buffer}`);
      await new Promise((r) => setTimeout(r, 50));
    }
    const out = buffer;
    buffer = "";
    return out;
  };
  await waitFor(/^\* OK/m);
  socket.write(`a LOGIN norelly "${password}"\r\n`);
  await waitFor(/^a OK/m);
  for (const [i, raw] of messages.entries()) {
    const bytes = Buffer.from(raw.replace(/\r?\n/g, "\r\n"), "utf8");
    socket.write(`m${i} APPEND INBOX {${bytes.length}}\r\n`);
    await waitFor(/^\+/m);
    socket.write(Buffer.concat([bytes, Buffer.from("\r\n")]));
    await waitFor(new RegExp(`^m${i} OK`, "m"));
  }
  socket.write("z LOGOUT\r\n");
  socket.end();
}

/** Requêtes reçues par le serveur canari. */
export async function canaryHits() {
  const response = await fetch(`${canary}/compte`);
  const body = await response.json();
  return body.hits;
}
