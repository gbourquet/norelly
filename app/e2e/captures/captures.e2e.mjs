// Captures d'écran de la documentation utilisateur, produites à
// chaque `pnpm -C app e2e`, une fois par langue (in-container.sh) :
//   NORELLY_E2E_LANG      fr ou en (langue attendue de l'interface) ;
//   NORELLY_E2E_CAPTURES  dossier de sortie (monté en écriture par run.sh,
//                         recopié ensuite dans docs/utilisateur/<langue>/captures).
//
// Profil neuf à chaque exécution. La question des mises à jour du premier
// lancement n'existe que dans l'installateur Windows (module `updater`,
// ADR-006) : elle n'est pas capturée. Boîte dédiée sur Dovecot (utilisateur distinct de celui des autres
// scénarios), remplie de messages lisibles dans la langue de la capture.
//
// Les textes ciblés viennent des catalogues de l'interface : le scénario
// suit les traductions sans les recopier.
import { readFileSync } from "node:fs";
import path from "node:path";
import { $, browser, expect } from "@wdio/globals";
import { accountMenu, appendMessages, password } from "../specs/helpers.mjs";

const lang = process.env.NORELLY_E2E_LANG ?? "fr";
const out = process.env.NORELLY_E2E_CAPTURES;
if (!out) throw new Error("NORELLY_E2E_CAPTURES non défini (lancer via pnpm -C app e2e)");

const t = JSON.parse(readFileSync(path.resolve("messages", `${lang}.json`), "utf8"));

/** Message du catalogue, paramètres remplacés. */
function msg(key, params = {}) {
  const text = t[key];
  if (typeof text !== "string") throw new Error(`clé absente du catalogue ${lang} : ${key}`);
  return text.replace(/\{(\w+)\}/g, (_, name) => String(params[name] ?? ""));
}

/** Compte de la capture : un utilisateur Dovecot par langue. */
const user = lang === "fr" ? "marie" : "mary";
const address = `${user}@norelly.test`;

const content = {
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
    search: "tarte",
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
    search: "pie",
  },
}[lang];

/** Messages bruts, du plus récent au plus ancien (le premier est en tête de liste). */
function rawMessages() {
  const base = Date.UTC(2026, 10, 4, 9, 0, 0);
  return content.messages.map(([from, subject, body], i) => {
    const date = new Date(base - i * 5 * 3600 * 1000).toUTCString().replace("GMT", "+0000");
    return `From: ${from}
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
async function capture(name) {
  // Laisse le temps aux transitions (focus, ouverture des fenêtres).
  await browser.pause(400);
  await browser.saveScreenshot(path.join(out, `${name}.png`));
}

/** Bouton dont le nom accessible est `name`. */
const button = (name) => $(`aria/${name}`);

/** Entrée de menu dont le texte commence par `label` (raccourci à droite). */
const menuItem = (label) => $(`//*[@role="menuitem"][starts-with(normalize-space(), "${label}")]`);

/** Fenêtre modale dont le titre est `title`. */
const dialogTitled = (title) => $(`//*[@role="dialog"][.//*[normalize-space()="${title}"]]`);

describe(`captures d'écran (${lang})`, () => {
  before(async () => {
    try {
      await browser.setWindowSize(1200, 780);
    } catch {
      // Taille de l'écran virtuel (Xvfb) conservée.
    }
    await appendMessages(rawMessages(), user);
  });

  it("accueil", async () => {
    await $(`p=${msg("welcome_no_account")}`).waitForDisplayed({ timeout: 20000 });
    // L'interface est bien dans la langue attendue.
    expect(await browser.execute(() => document.documentElement.lang)).toBe(lang);
    await capture("welcome");
  });

  it("ajout du compte", async () => {
    await $(`button=${msg("welcome_add_account")}`).click();
    await $("#account-address").setValue(address);
    await $("#account-password").setValue(password);
    await $("#account-name").setValue(content.name);
    await capture("add-account");
    await $(`button=${msg("common_continue")}`).click();
    await $("#imap-host").waitForDisplayed({ timeout: 90000 });
    await $("#imap-port").setValue("31993");
    await $("#imap-username").setValue(user);
    await $("#smtp-port").setValue("1025");
    await $("#smtp-username").setValue("norelly");
    await capture("servers");
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
      if (i === 0) await capture("certificate");
      await trust.click();
    }
    await entry.waitForDisplayed({ timeout: 60000 });
  });

  it("liste et lecture", async () => {
    const first = $(`span=${content.messages[0][1]}`);
    await first.waitForDisplayed({ timeout: 90000 });
    await $(`span=${content.messages.at(-1)[1]}`).waitForDisplayed({ timeout: 60000 });
    await capture("inbox");

    // Sélection multiple à la souris : cases à cocher et barre groupée.
    for (const [, subject] of content.messages.slice(0, 3)) {
      await button(msg("list_select_row", { subject })).click();
    }
    await $(`[aria-label="${msg("list_selection")}"]`).waitForDisplayed({ timeout: 10000 });
    await capture("selection");
    await button(msg("cmd_clear_selection")).click();

    // Menu du clic droit sur une ligne.
    await $(`span=${content.messages[2][1]}`).click({ button: "right" });
    await menuItem(msg("cmd_reply")).waitForDisplayed({ timeout: 10000 });
    await capture("context-menu");
    await browser.keys("Escape");

    await first.click();
    await expect($(`h1=${content.messages[0][1]}`)).toBeDisplayed({ wait: 30000 });
    await capture("reading");
  });

  it("réponse, puis brouillon gardé", async () => {
    await button(msg("cmd_reply")).click();
    const dialog = dialogTitled(msg("compose_title_reply"));
    await dialog.waitForDisplayed({ timeout: 20000 });
    await dialog.$(`[aria-label="${msg("compose_body")}"]`).click();
    // Curseur en tête, au-dessus de la citation. WebKitWebDriver perd les
    // lettres doublées d'une frappe : les textes de réponse n'en ont pas.
    await browser.keys(["Control", "Home"]);
    await browser.keys(content.reply);
    await capture("compose");
    // Échap ferme l'éditeur en gardant le brouillon.
    await browser.keys("Escape");
    await dialog.waitForDisplayed({ reverse: true, timeout: 20000 });
    await browser.keys("Escape");
  });

  it("brouillons, raccourcis, palette, paramètres (à la souris)", async () => {
    await accountMenu(address, msg("cmd_drafts"));
    const drafts = dialogTitled(msg("drafts_title"));
    await drafts.waitForDisplayed({ timeout: 20000 });
    await capture("drafts");
    await drafts.$(`aria/${msg("common_close")}`).click();
    await drafts.waitForDisplayed({ reverse: true, timeout: 10000 });

    await button(msg("cmd_help")).click();
    const help = dialogTitled(msg("help_title"));
    await help.waitForDisplayed({ timeout: 10000 });
    await capture("shortcuts");
    // La palette s'ouvre depuis l'aide.
    await help.$(`button=${msg("cmd_palette")}`).click();
    const palette = $(`[placeholder="${msg("palette_placeholder")}"]`);
    await palette.waitForDisplayed({ timeout: 10000 });
    await capture("palette");
    await browser.keys("Escape");
    await palette.waitForDisplayed({ reverse: true, timeout: 10000 });

    await button(msg("sidebar_settings")).click();
    const settings = dialogTitled(msg("settings_title"));
    await settings.waitForDisplayed({ timeout: 10000 });
    await capture("settings");
    await settings.$(`aria/${msg("common_close")}`).click();
    await settings.waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("déplacement vers un dossier", async () => {
    await $(`span=${content.messages[1][1]}`).click();
    await expect($(`h1=${content.messages[1][1]}`)).toBeDisplayed({ wait: 30000 });
    await button(msg("action_more")).click();
    await menuItem(msg("cmd_move")).click();
    const picker = $(`[placeholder="${msg("picker_placeholder")}"]`);
    await picker.waitForDisplayed({ timeout: 10000 });
    await capture("move");
    await browser.keys("Escape");
    await picker.waitForDisplayed({ reverse: true, timeout: 10000 });
    await browser.keys("Escape");
  });

  it("recherche", async () => {
    await button(msg("search_label")).click();
    const box = $('input[type="search"]');
    await box.waitForDisplayed({ timeout: 10000 });
    const results = $(`[role="listbox"][aria-label="${msg("search_results")}"]`);
    const highlighted = results.$(`mark=${content.search}`);
    await browser.waitUntil(
      async () => {
        await box.click();
        await box.clearValue();
        await box.setValue(content.search);
        await browser.keys("Enter");
        return highlighted.isDisplayed();
      },
      { timeout: 60000, interval: 1000, timeoutMsg: "mot recherché introuvable" },
    );
    await box.click();
    await capture("search");
    await browser.keys("Escape");
  });
});
