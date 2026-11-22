// Données et gestes communs aux scénarios de captures d'écran
// (captures.e2e.mjs, inbox-dark.e2e.mjs) : même langue, même compte, mêmes
// messages, même taille de fenêtre.
import { readFileSync } from "node:fs";
import path from "node:path";
import { $, browser } from "@wdio/globals";
import { password } from "../specs/helpers.mjs";

export const lang = process.env.NORELLY_E2E_LANG ?? "fr";
export const out = process.env.NORELLY_E2E_CAPTURES;
if (!out) throw new Error("NORELLY_E2E_CAPTURES non défini (lancer via pnpm -C app e2e)");

const t = JSON.parse(readFileSync(path.resolve("messages", `${lang}.json`), "utf8"));

/** Message du catalogue, paramètres remplacés. */
export function msg(key, params = {}) {
  const text = t[key];
  if (typeof text !== "string") throw new Error(`clé absente du catalogue ${lang} : ${key}`);
  return text.replace(/\{(\w+)\}/g, (_, name) => String(params[name] ?? ""));
}

/** Compte de la capture : un utilisateur Dovecot par langue. */
export const user = lang === "fr" ? "marie" : "mary";
export const address = `${user}@norelly.test`;

export const content = {
  fr: {
    name: "Marie Durand",
    messages: [
      [
        "Alice Martin <alice@norelly.test>",
        "Pique-nique de dimanche",
        "Bonjour Marie,\n\nOn se retrouve dimanche à midi au parc ? J'apporte la tarte aux pommes.\n\nAlice",
      ],
      [
        "Paul Bernard <paul@norelly.test>",
        "Photos des vacances",
        "Coucou,\n\nVoici enfin les photos de l'été. Dis-moi lesquelles tu préfères !\n\nPaul",
      ],
      [
        "Mairie de Valfleuri <contact@norelly.test>",
        "Inscriptions à la bibliothèque",
        "Madame,\n\nLes inscriptions pour la nouvelle saison sont ouvertes jusqu'au 30 novembre.\n\nLe service culturel",
      ],
      [
        "Docteur Leroy <cabinet@norelly.test>",
        "Rappel de rendez-vous",
        "Bonjour,\n\nNous vous rappelons votre rendez-vous de mardi à 10 h 30.\n\nLe secrétariat",
      ],
    ],
    reply: "Avec plaisir, à dimanche ! Je prends les jus de fruits.",
    folder: "Jardin",
    renamed: "Potager",
    subfolder: "Cuisine",
    newFolder: "Santé",
    search: "tarte",
    suggest: "ler",
    labels: ["Associations", "Famille"],
    contacts: [
      ["Alice", "Martin", "alice@norelly.test", "Association des parents d'élèves"],
      ["Paul", "Bernard", "paul@norelly.test", ""],
      ["Jeanne", "Leroy", "jeanne@norelly.test", "Cabinet médical Leroy"],
    ],
    contactDetails: {
      title: "Présidente",
      street: "12 rue des Lilas",
      code: "69003",
      city: "Lyon",
      country: "France",
      label: "maison",
      website: "www.ape-valfleuri.example",
    },
    list: {
      name: "Famille",
      description: "Pour organiser les repas du dimanche",
      members: [
        ["ali", "Alice Martin"],
        ["pau", "Paul Bernard"],
      ],
      suggest: "fam",
    },
  },
  en: {
    name: "Mary Smith",
    messages: [
      [
        "Alice Martin <alice@norelly.test>",
        "Sunday picnic",
        "Hi Mary,\n\nShall we meet at the park on Sunday at noon? I'll bring the apple pie.\n\nAlice",
      ],
      [
        "Paul Brown <paul@norelly.test>",
        "Holiday photos",
        "Hello,\n\nHere are the summer photos at last. Tell me which ones you like best!\n\nPaul",
      ],
      [
        "Greenfield Town Hall <contact@norelly.test>",
        "Library registration",
        "Dear Ms Smith,\n\nRegistration for the new season is open until 30 November.\n\nThe culture department",
      ],
      [
        "Dr Lee's surgery <surgery@norelly.test>",
        "Appointment reminder",
        "Hello,\n\nThis is a reminder of your appointment on Tuesday at 10:30.\n\nThe front desk",
      ],
    ],
    reply: "Great, until Sunday! I can bring drinks.",
    folder: "Garden",
    renamed: "Vegetables",
    subfolder: "Recipes",
    newFolder: "Health",
    search: "pie",
    suggest: "lee",
    labels: ["Clubs", "Family"],
    contacts: [
      ["Alice", "Martin", "alice@norelly.test", "Parents' association"],
      ["Paul", "Brown", "paul@norelly.test", ""],
      ["Jane", "Lee", "jane@norelly.test", "Dr Lee's surgery"],
    ],
    contactDetails: {
      title: "Chair",
      street: "12 Lilac Street",
      code: "GL1 1AA",
      city: "Greenfield",
      country: "United Kingdom",
      label: "home",
      website: "www.greenfield-pta.example",
    },
    list: {
      name: "Family",
      description: "To organise Sunday lunches",
      members: [
        ["ali", "Alice Martin"],
        ["pau", "Paul Brown"],
      ],
      suggest: "fam",
    },
  },
}[lang];

/** Messages bruts, du plus récent au plus ancien (le premier est en tête de liste). */
export function rawMessages() {
  const base = Date.UTC(2026, 10, 4, 9, 0, 0);
  return content.messages.map(([from, subject, body], i) => {
    const date = new Date(base - i * 5 * 3600 * 1000).toUTCString().replace("GMT", "+0000");
    // Expéditeurs authentifiés par le serveur du compte (`authserv-id` du
    // domaine de l'hôte IMAP, imap.norelly.test).
    const domain = from.replace(/^.*@([^>]+)>?$/, "$1");
    return `Authentication-Results: mx.norelly.test; dmarc=pass header.from=${domain}
From: ${from}
To: ${content.name} <${address}>
Subject: ${subject}
Date: ${date}
Message-ID: <capture-${lang}-${i}@norelly.test>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

${body}
`;
  });
}

/** Enregistre une capture de la fenêtre. */
export async function capture(name) {
  // Laisse le temps aux transitions (focus, ouverture des fenêtres).
  await browser.pause(400);
  await browser.saveScreenshot(path.join(out, `${name}.png`));
}

/** Taille de la fenêtre de toutes les captures. */
export async function captureWindowSize() {
  try {
    await browser.setWindowSize(1200, 780);
  } catch {
    // Taille de l'écran virtuel (Xvfb) conservée.
  }
}

/**
 * Ajoute le compte de la capture à la souris, depuis l'écran d'accueil,
 * jusqu'à son apparition dans le panneau de gauche. `shot(nom)` est appelé
 * aux étapes capturées (sans effet pour un scénario qui ne les veut pas).
 */
export async function addCaptureAccount(shot) {
  await $("main")
    .$(`button=${msg("welcome_add_account")}`)
    .click();
  await $("#account-address").setValue(address);
  await $("#account-password").setValue(password);
  // Nom affiché qui imite une adresse : message sous le champ.
  await $("#account-name").setValue("support@banque.fr");
  await $("#account-name-error").waitForDisplayed({ timeout: 10000 });
  await shot("display-name-refused");
  await $("#account-name").setValue(content.name);
  await $("#account-name-error").waitForDisplayed({ reverse: true, timeout: 10000 });
  await shot("add-account");
  await $(`button=${msg("common_continue")}`).click();
  await $("#imap-host").waitForDisplayed({ timeout: 90000 });
  await $("#imap-port").setValue("31993");
  await $("#imap-username").setValue(user);
  await $("#smtp-port").setValue("1025");
  await $("#smtp-username").setValue("norelly");
  await shot("servers");
  await $(`button=${msg("add_test_and_add")}`).click();
  // Certificats autosignés du réseau de test : l'avertissement est capturé
  // une fois, puis la confiance est accordée (IMAP, puis SMTP).
  const entry = $(`span=${address}`);
  const trust = $(`button=${msg("add_cert_trust")}`);
  for (let i = 0; i < 2; i++) {
    await browser.waitUntil(
      async () => (await trust.isDisplayed()) || (await entry.isDisplayed()),
      {
        timeout: 60000,
      },
    );
    if (await entry.isDisplayed()) break;
    if (i === 0) await shot("certificate");
    await trust.click();
  }
  await entry.waitForDisplayed({ timeout: 60000 });
}
