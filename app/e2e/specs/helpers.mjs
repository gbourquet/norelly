// Outils partagés des scénarios end-to-end : ajout de compte par
// l'interface, dépôt de messages sur Dovecot (APPEND), compteur du
// serveur canari, choix d'un fichier dans le sélecteur natif.
import { execFile } from "node:child_process";
import tls from "node:tls";
import { promisify } from "node:util";
import { $, browser, expect } from "@wdio/globals";

export const password = process.env.NORELLY_E2E_PASSWORD ?? "";
export const address = "norelly@norelly.test";
const canary = process.env.NORELLY_E2E_CANARY ?? "http://canary.norelly.test:8080";

/** Ajoute le compte de test (saisie manuelle, confiance dans les certificats si demandée). */
export async function addTestAccount() {
  // Bouton du panneau de navigation, toujours visible (l'accueil sans
  // compte en propose un second).
  await $("nav").$("button=Ajouter un compte").click();
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

/**
 * Ouvre un menu déroulant (« Plus… », menu d'un compte). Les menus
 * s'ouvrent sur `pointerdown`, que la souris émet ; WebKitWebDriver ne
 * synthétise que `mousedown`, `mouseup` et `click` : le `pointerdown` est
 * donc envoyé tel que la souris le produit.
 */
export async function openMenu(trigger) {
  const element = await trigger;
  await element.waitForDisplayed({ timeout: 10000 });
  await browser.execute((el) => {
    el.dispatchEvent(
      new PointerEvent("pointerdown", { bubbles: true, button: 0, pointerType: "mouse" }),
    );
  }, element);
}

/**
 * Clic droit sur un élément : seul l'événement `contextmenu` est envoyé,
 * au centre de l'élément, comme le produit la souris (le clic droit de
 * WebKitWebDriver émet aussi un clic, qui ouvrirait la conversation).
 */
export async function openContextMenu(target) {
  const element = await target;
  await element.waitForDisplayed({ timeout: 10000 });
  await browser.execute((el) => {
    const r = el.getBoundingClientRect();
    el.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        button: 2,
        clientX: r.left + r.width / 2,
        clientY: r.top + r.height / 2,
      }),
    );
  }, element);
}

/** Ouvre le menu « ⋯ » d'un compte et choisit une entrée ; `menu` : nom
 * du bouton du menu (français par défaut, celui des scénarios). */
export async function accountMenu(account, entry, menu = `Actions du compte ${account}`) {
  await openMenu($(`aria/${menu}`));
  const item = $(`//*[@role="menuitem"][normalize-space()="${entry}"]`);
  await item.waitForDisplayed({ timeout: 10000 });
  await item.click();
}

/** Supprime le compte de test. */
export async function removeTestAccount() {
  await accountMenu(address, "Supprimer ce compte…");
  const confirm = $("button=Supprimer le compte");
  await confirm.waitForDisplayed();
  await confirm.click();
  await $("p=Aucun compte pour l'instant.").waitForDisplayed({ timeout: 20000 });
}

/** Dépose des messages bruts dans la boîte de réception d'un utilisateur
 * Dovecot (IMAP APPEND, TLS). Dovecot accepte tout utilisateur muni du mot
 * de passe de test : chaque utilisateur a sa propre boîte. */
export async function appendMessages(messages, user = "norelly") {
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
  socket.write(`a LOGIN ${user} "${password}"\r\n`);
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

/** Commande IMAP brève sur une session authentifiée ; renvoie la réponse. */
export async function imapCommand(command) {
  const socket = tls.connect({
    host: "imap.norelly.test",
    port: 31993,
    servername: "imap.norelly.test",
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
  };
  await waitFor(/^\* OK/m);
  socket.write(`a LOGIN norelly "${password}"\r\nb ${command}\r\n`);
  await waitFor(/^b (OK|NO|BAD)/m);
  const out = buffer;
  socket.write("z LOGOUT\r\n");
  socket.end();
  return out;
}

/** Nombre de messages d'une boîte du serveur IMAP (STATUS). */
export async function imapMessages(mailbox) {
  const reply = await imapCommand(`STATUS "${mailbox}" (MESSAGES)`);
  const match = /MESSAGES (\d+)/.exec(reply);
  return match ? Number(match[1]) : 0;
}

const mailpit = "http://smtp.norelly.test:8025";

/** Messages reçus par Mailpit (résumés, du plus récent au plus ancien). */
export async function mailpitMessages() {
  const response = await fetch(`${mailpit}/api/v1/messages`);
  const body = await response.json();
  return body.messages ?? [];
}

/** Message reçu par Mailpit (corps texte et HTML décodés). */
export async function mailpitMessage(id) {
  const response = await fetch(`${mailpit}/api/v1/message/${id}`);
  return response.json();
}

/** En-têtes transmis d'un message reçu par Mailpit (nom → valeurs). */
export async function mailpitHeaders(id) {
  const response = await fetch(`${mailpit}/api/v1/message/${id}/headers`);
  return response.json();
}

const xdotool = (...args) => promisify(execFile)("xdotool", args, { timeout: 30000 });

/**
 * Choisit `file` (chemin absolu) dans le sélecteur de fichiers du système
 * (GTK) titré `title`, que WebDriver ne voit pas : xdotool donne le focus
 * à sa fenêtre, ouvre la saisie d'un emplacement (Ctrl+L), tape le chemin
 * et valide, comme le ferait l'utilisateur au clavier.
 */
export async function pickFile(title, file) {
  // La fenêtre peut être trouvée avant d'être affichée (focus refusé) :
  // fenêtres visibles seulement, quelques essais.
  for (let attempt = 1; ; attempt += 1) {
    try {
      await xdotool("search", "--sync", "--onlyvisible", "--name", title, "windowfocus", "--sync");
      break;
    } catch (e) {
      if (attempt >= 10) throw e;
      await browser.pause(500);
    }
  }
  await browser.pause(500);
  await xdotool("key", "--clearmodifiers", "ctrl+l");
  await browser.pause(300);
  await xdotool("type", "--delay", "20", file);
  await browser.pause(300);
  await xdotool("key", "Return");
}

/**
 * Enregistre sous `file` (chemin absolu) dans le sélecteur d'enregistrement
 * du système (GTK) titré `title` : xdotool lui donne le focus, remplace le
 * nom proposé par le chemin complet et valide.
 */
export async function saveFile(title, file) {
  for (let attempt = 1; ; attempt += 1) {
    try {
      await xdotool("search", "--sync", "--onlyvisible", "--name", title, "windowfocus", "--sync");
      break;
    } catch (e) {
      if (attempt >= 10) throw e;
      await browser.pause(500);
    }
  }
  await browser.pause(500);
  await xdotool("key", "--clearmodifiers", "ctrl+a");
  await browser.pause(200);
  await xdotool("type", "--delay", "20", file);
  await browser.pause(300);
  await xdotool("key", "Return");
}
