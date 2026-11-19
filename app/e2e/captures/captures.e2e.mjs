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
import { fileURLToPath } from "node:url";
import { $, $$, browser, expect } from "@wdio/globals";
import {
  accountMenu,
  appendMessages,
  openContextMenu,
  openMenu,
  password,
  pickFile,
} from "../specs/helpers.mjs";

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
function rawMessages() {
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
async function capture(name) {
  // Laisse le temps aux transitions (focus, ouverture des fenêtres).
  await browser.pause(400);
  await browser.saveScreenshot(path.join(out, `${name}.png`));
}

/** Bouton visible dont le libellé (texte ou `aria-label`) est `name` ; la
 * copie invisible de mesure des barres d'actions est ignorée. */
const button = (name) =>
  $(
    `//button[@aria-label="${name}" or normalize-space()="${name}"][not(ancestor::*[@aria-hidden="true"])]`,
  );

/** Entrée de menu dont le libellé est exactement `label` (raccourci à droite). */
const menuItem = (label) =>
  $(`//*[@role="menuitem"][.//*[contains(@class, "menu-label")][normalize-space()="${label}"]]`);

/** Action `label` de l'en-tête de la vue Contacts, directe ou rangée dans
 * « Plus… » faute de place. */
async function headerAction(label) {
  const bar = $(`[role="toolbar"][aria-label="${msg("contacts_header_actions")}"]`);
  const visible = `[not(ancestor::*[@aria-hidden="true"])]`;
  const direct = bar.$(`.//button[normalize-space()="${label}"]${visible}`);
  if (await direct.isDisplayed().catch(() => false)) {
    await direct.click();
    return;
  }
  await openMenu(bar.$(`.//button[normalize-space()="${msg("action_more")}"]${visible}`));
  await menuItem(label).click();
}

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
    await $("main")
      .$(`button=${msg("welcome_add_account")}`)
      .click();
    await $("#account-address").setValue(address);
    await $("#account-password").setValue(password);
    // Nom affiché qui imite une adresse : message sous le champ.
    await $("#account-name").setValue("support@banque.fr");
    await $("#account-name-error").waitForDisplayed({ timeout: 10000 });
    await capture("display-name-refused");
    await $("#account-name").setValue(content.name);
    await $("#account-name-error").waitForDisplayed({ reverse: true, timeout: 10000 });
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

  it("compte Google : connexion Google, puis mot de passe d'application refusé", async () => {
    // Adresse Gmail sur le faux serveur IMAP de Google (réseau sans
    // Internet : la découverte échoue ; certificat signé par l'autorité de
    // test, sans exception), mot de passe refusé par Dovecot : l'aide au mot
    // de passe d'application s'affiche. Le serveur d'envoi n'est pas joint.
    await $("nav")
      .$(`button=${msg("welcome_add_account")}`)
      .click();
    await $("#account-address").setValue(`${user}@gmail.com`);
    // Rust propose « Se connecter avec Google » : l'avertissement de Google
    // est expliqué avant d'ouvrir le navigateur. Le bouton n'est pas cliqué
    // (réseau de test sans Internet) ; le mot de passe d'application reste
    // proposé à côté.
    const signIn = $(`button=${msg("add_oauth_sign_in_google")}`);
    await signIn.waitForDisplayed({ timeout: 10000 });
    await expect($("#account-password")).not.toBeDisplayed();
    await expect($(`p=${msg("add_oauth_hint_google")}`)).toBeDisplayed();
    await capture("google-sign-in");
    await $(`button=${msg("add_use_app_password")}`).click();
    await $("#account-password").waitForDisplayed();
    await $("#account-password").setValue("mot-de-passe-du-compte");
    await $(`button=${msg("common_continue")}`).click();
    await $("#imap-host").waitForDisplayed({ timeout: 90000 });
    await $("#imap-host").setValue("imap.gmail.com");
    await $("#imap-port").setValue("31993");
    await $("#imap-username").setValue(user);
    await $("#smtp-host").setValue("smtp.gmail.com");
    await $("#smtp-username").setValue(user);
    await $(`button=${msg("add_test_and_add")}`).click();
    const help = $(`//section[.//h3[normalize-space()="${msg("add_app_password_title")}"]]`);
    await help.waitForDisplayed({ timeout: 60000 });
    await expect(help.$$("ol > li")).toBeElementsArrayOfSize(3);
    await expect($(`button=${msg("add_app_password_open")}`)).toBeDisplayed();
    await expect($("#manual-password")).toHaveValue("");
    await capture("google-app-password");
    await browser.keys("Escape");
    await $("#imap-host").waitForDisplayed({ reverse: true });
  });

  it("compte Microsoft : se connecter avec Microsoft", async () => {
    // Adresse Outlook.com : Rust la reconnaît, le formulaire propose la
    // connexion par Microsoft au lieu du mot de passe. Le bouton n'est pas
    // cliqué (réseau de test sans Internet).
    await $("nav")
      .$(`button=${msg("welcome_add_account")}`)
      .click();
    await $("#account-address").setValue(`${user}@outlook.com`);
    const signIn = $(`button=${msg("add_oauth_sign_in_microsoft")}`);
    await signIn.waitForDisplayed({ timeout: 10000 });
    await expect($("#account-password")).not.toBeDisplayed();
    await expect($(`p=${msg("add_oauth_hint_microsoft")}`)).toBeDisplayed();
    await capture("microsoft-sign-in");
    await browser.keys("Escape");
    await $("#account-address").waitForDisplayed({ reverse: true });
  });

  it("liste et lecture", async () => {
    const first = $(`span=${content.messages[0][1]}`);
    await first.waitForDisplayed({ timeout: 90000 });
    await $(`span=${content.messages.at(-1)[1]}`).waitForDisplayed({ timeout: 60000 });
    await capture("inbox");

    // Sélection multiple à la souris : cases à cocher et barre groupée.
    for (const [, subject] of content.messages.slice(0, 3)) {
      await $(
        `input[type="checkbox"][aria-label="${msg("list_select_row", { subject })}"]`,
      ).click();
    }
    await $(`[aria-label="${msg("list_selection")}"]`).waitForDisplayed({ timeout: 10000 });
    await capture("selection");
    await button(msg("cmd_clear_selection")).click();

    // Menu du clic droit sur une ligne.
    await openContextMenu($(`span=${content.messages[2][1]}`));
    await menuItem(msg("cmd_reply")).waitForDisplayed({ timeout: 10000 });
    await capture("context-menu");
    await browser.keys("Escape");

    await first.click();
    await expect($(`h1=${content.messages[0][1]}`)).toBeDisplayed({ wait: 30000 });
    await capture("reading");

    // Menu « Plus… » du message ouvert (dont « Imprimer »).
    await openMenu(button(msg("action_more")));
    await menuItem(msg("cmd_print")).waitForDisplayed({ timeout: 10000 });
    await capture("reading-more");
    await browser.keys("Escape");

    // Largeurs de panneaux par défaut, fenêtre de 1 280 px : les cinq
    // actions principales restent dans la barre, aucune n'est repliée
    // dans « Plus… ».
    try {
      await browser.setWindowSize(1280, 780);
      await browser.waitUntil(async () => (await $("body").getSize("width")) === 1280, {
        timeout: 5000,
      });
      // Plusieurs éléments peuvent porter ce nom de barre : seul compte un
      // bouton affiché, hors de la copie invisible de mesure.
      const toolbar = `//*[@role="toolbar"][@aria-label="${msg("conversation_toolbar")}"]`;
      for (const key of [
        "cmd_reply",
        "cmd_reply_all",
        "cmd_forward",
        "cmd_archive",
        "cmd_delete",
      ]) {
        const shown = async () => {
          const found = await $$(
            `${toolbar}//button[normalize-space()="${msg(key)}"][not(ancestor::*[@aria-hidden="true"])]`,
          );
          for (const el of found) if (await el.isDisplayed()) return true;
          return false;
        };
        await browser.waitUntil(shown, {
          timeout: 5000,
          timeoutMsg: `« ${msg(key)} » absent de la barre à 1 280 px`,
        });
      }
    } finally {
      // Taille des captures rétablie avant les scénarios suivants.
      await browser.setWindowSize(1200, 780);
      await browser.waitUntil(async () => (await $("body").getSize("width")) === 1200, {
        timeout: 5000,
      });
    }
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

  it("changer le mot de passe (à la souris)", async () => {
    await openMenu($(`aria/${msg("sidebar_account_menu", { address })}`));
    await menuItem(msg("sidebar_change_password")).waitForDisplayed({ timeout: 10000 });
    await capture("account-menu");
    await menuItem(msg("sidebar_change_password")).click();
    const dialog = dialogTitled(msg("password_title"));
    await dialog.waitForDisplayed({ timeout: 10000 });
    await $("#new-password").setValue("mot-de-passe-faux");
    await dialog.$(`button=${msg("password_save")}`).click();
    await dialog.$(`[role="alert"]`).waitForDisplayed({ timeout: 60000 });
    await capture("change-password");
    await dialog.$(`button=${msg("common_cancel")}`).click();
    await dialog.waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("brouillons, raccourcis, palette, paramètres (à la souris)", async () => {
    await accountMenu(address, msg("cmd_drafts"), msg("sidebar_account_menu", { address }));
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
    await openMenu(button(msg("action_more")));
    await menuItem(msg("cmd_move")).click();
    const picker = $(`[placeholder="${msg("picker_placeholder")}"]`);
    await picker.waitForDisplayed({ timeout: 10000 });
    await capture("move");
    await browser.keys("Escape");
    await picker.waitForDisplayed({ reverse: true, timeout: 10000 });
    await browser.keys("Escape");
  });

  it("étiquettes : menu « Étiqueter… », réglages, pastilles (à la souris)", async () => {
    const subject = content.messages[2][1];
    await $(`span=${subject}`).click();
    await expect($(`h1=${subject}`)).toBeDisplayed({ wait: 30000 });
    await openMenu(button(msg("action_more")));
    await menuItem(msg("cmd_label")).click();
    const dialog = dialogTitled(msg("label_dialog_title"));
    await dialog.waitForDisplayed({ timeout: 10000 });
    const name = dialog.$(`input[aria-label="${msg("label_dialog_new")}"]`);
    for (const [i, label] of content.labels.entries()) {
      await name.setValue(label);
      await dialog.$(`button=${msg("label_dialog_create")}`).click();
      await expect(dialog.$$('input[type="checkbox"]')).toBeElementsArrayOfSize(i + 1, {
        wait: 10000,
      });
    }
    await capture("label-menu");
    await dialog.$(`button=${msg("label_dialog_done")}`).click();
    await dialog.waitForDisplayed({ reverse: true, timeout: 10000 });

    // Réglages : couleur de la première étiquette (sélecteur de couleur du
    // système, valeur posée comme il la rend).
    await button(msg("sidebar_settings")).click();
    const settings = dialogTitled(msg("settings_title"));
    await settings.waitForDisplayed({ timeout: 10000 });
    const color = settings.$(
      `input[type="color"][aria-label="${msg("settings_label_color", { label: content.labels[0] })}"]`,
    );
    await color.waitForDisplayed({ timeout: 10000 });
    const picker = await color;
    await browser.execute((el) => {
      el.value = "#2e7d32";
      el.dispatchEvent(new Event("change", { bubbles: true }));
    }, picker);
    await expect(settings.$(`button=${msg("settings_label_no_color")}`)).toBeDisplayed({
      wait: 10000,
    });
    await settings.$("#labels-settings-title").scrollIntoView();
    await capture("label-settings");
    await settings.$(`aria/${msg("common_close")}`).click();
    await settings.waitForDisplayed({ reverse: true, timeout: 10000 });

    // Pastilles sous le sujet et dans la ligne.
    await capture("label-reading");
    await browser.keys("Escape");
    await $(
      `button[aria-label="${msg("label_search", { label: content.labels[0] })}"]`,
    ).waitForDisplayed({ timeout: 10000 });
    await capture("label-list");
  });

  it("nouveau dossier (à la souris)", async () => {
    await accountMenu(address, msg("cmd_new_folder"), msg("sidebar_account_menu", { address }));
    const dialog = dialogTitled(msg("new_folder_title"));
    await dialog.waitForDisplayed({ timeout: 10000 });
    await dialog.$(`input[type="text"]`).setValue(content.folder);
    await capture("new-folder");
    await dialog.$(`button=${msg("new_folder_create")}`).click();
    await dialog.waitForDisplayed({ reverse: true, timeout: 30000 });
    // Créé sur le serveur, puis affiché dans les dossiers du compte.
    await $(
      `//*[@aria-label="${msg("sidebar_folders_of", { address })}"]//span[normalize-space()="${content.folder}"]`,
    ).waitForDisplayed({ timeout: 30000 });
  });

  it("glisser un message sur un dossier, puis annuler", async () => {
    const subject = content.messages[2][1];
    const folders = `//*[@aria-label="${msg("sidebar_folders_of", { address })}"]`;
    // Recherchée à chaque fois : la liste virtualisée réutilise ses lignes.
    const row = () => $(`//*[@role="option"][.//span[normalize-space()="${subject}"]]`);
    const target = $(`${folders}//span[normalize-space()="${content.folder}"]`);
    await row().waitForDisplayed({ timeout: 10000 });
    // Bouton enfoncé, déplacé jusqu'au dossier, sans relâcher : capture
    // pendant le glisser. Même pointeur pour les deux séquences : sa
    // position est conservée.
    const mouse = () =>
      browser.action("pointer", { id: "souris", parameters: { pointerType: "mouse" } });
    await mouse()
      .move({ origin: row() })
      .down({ button: 0 })
      .move({ origin: row(), x: 20, y: 10, duration: 100 })
      .move({ origin: target, duration: 300 })
      .perform(true);
    await $(".drag-ghost").waitForDisplayed({ timeout: 10000 });
    await capture("drag-to-folder");
    await mouse().move({ origin: target, duration: 50 }).up({ button: 0 }).perform();
    await expect($(".notice-region")).toHaveText(
      expect.stringContaining(msg("action_moved", { folder: content.folder })),
      { wait: 10000 },
    );
    await row().waitForDisplayed({ reverse: true, timeout: 10000 });
    // « Annuler » dans l'avis (libellé suivi du raccourci) : le message
    // revient dans la boîte de réception.
    await $(`//button[starts-with(normalize-space(), "${msg("action_undo")}")]`).click();
    await row().waitForDisplayed({ timeout: 10000 });
  });

  it("glisser un résultat de recherche sur un dossier hors de vue, puis annuler", async () => {
    const folders = `//*[@aria-label="${msg("sidebar_folders_of", { address })}"]`;
    const target = $(`${folders}//span[normalize-space()="${content.folder}"]`);
    await button(msg("search_label")).click();
    const box = $('input[type="search"]');
    await box.waitForDisplayed({ timeout: 10000 });
    const results = $(`[role="listbox"][aria-label="${msg("search_results")}"]`);
    const hit = results.$(`.//*[@role="option"][.//mark[normalize-space()="${content.search}"]]`);
    await browser.waitUntil(
      async () => {
        await box.click();
        await box.clearValue();
        await box.setValue(content.search);
        await browser.keys("Enter");
        return hit.isDisplayed();
      },
      { timeout: 60000, interval: 1000, timeoutMsg: "mot recherché introuvable" },
    );

    // Panneau de gauche réduit : le dossier est sous le bord bas, hors de vue.
    // Éléments résolus : `browser.execute` ne reçoit que des éléments trouvés.
    const nav = await $("nav[data-drop-scroll]");
    const folder = await target;
    await browser.execute((el) => {
      el.style.height = "200px";
      el.style.alignSelf = "start";
      el.scrollTop = 0;
    }, nav);
    const inView = () =>
      browser.execute(
        (el, t) => {
          const pane = el.getBoundingClientRect();
          const r = t.getBoundingClientRect();
          return r.top >= pane.top && r.bottom <= pane.bottom - 60;
        },
        nav,
        folder,
      );
    expect(await inView()).toBe(false);

    // Bouton enfoncé, amené près du bord bas du panneau et tenu : le panneau
    // défile jusqu'à montrer le dossier.
    const { height } = await nav.getSize();
    const mouse = () =>
      browser.action("pointer", { id: "souris", parameters: { pointerType: "mouse" } });
    await mouse()
      .move({ origin: hit })
      .down({ button: 0 })
      .move({ origin: hit, x: 20, y: 10, duration: 100 })
      .move({ origin: nav, x: 0, y: Math.floor(height / 2) - 4, duration: 300 })
      .perform(true);
    await $(".drag-ghost").waitForDisplayed({ timeout: 10000 });
    await browser.waitUntil(inView, { timeout: 15000, timeoutMsg: "panneau pas défilé" });
    await mouse().move({ origin: target, duration: 200 }).perform(true);
    await expect(target.parentElement().parentElement()).toHaveElementClass("drop-over");
    await mouse().move({ origin: target, duration: 50 }).up({ button: 0 }).perform();
    await expect($(".notice-region")).toHaveText(
      expect.stringContaining(msg("action_moved", { folder: content.folder })),
      { wait: 10000 },
    );
    await browser.execute((el) => {
      el.style.height = "";
      el.style.alignSelf = "";
    }, nav);
    await $(`//button[starts-with(normalize-space(), "${msg("action_undo")}")]`).click();
    await expect($(".notice-region")).toHaveText(expect.stringContaining(msg("action_undone")), {
      wait: 10000,
    });
    await browser.keys("Escape");
    await box.waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("renommer, déplacer (fenêtre puis glisser) et supprimer un dossier (à la souris)", async () => {
    const folders = `//*[@aria-label="${msg("sidebar_folders_of", { address })}"]`;
    const label = (name) => $(`${folders}//span[normalize-space()="${name}"]`);
    /** Ouvre le menu « ⋯ » du dossier `name`, visible au survol. */
    const folderMenu = async (name) => {
      await label(name).moveTo();
      await openMenu($(`aria/${msg("sidebar_folder_menu", { folder: name })}`));
    };

    await folderMenu(content.folder);
    await menuItem(msg("cmd_rename_folder")).waitForDisplayed({ timeout: 10000 });
    await capture("folder-menu");
    await menuItem(msg("cmd_rename_folder")).click();
    const rename = dialogTitled(msg("rename_folder_title"));
    await rename.waitForDisplayed({ timeout: 10000 });
    const input = rename.$(`input[type="text"]`);
    await input.clearValue();
    await input.setValue(content.renamed);
    await capture("rename-folder");
    await rename.$(`button=${msg("rename_folder_confirm")}`).click();
    await rename.waitForDisplayed({ reverse: true, timeout: 30000 });
    // Renommé sur le serveur, puis affiché sous son nouveau nom.
    await label(content.renamed).waitForDisplayed({ timeout: 30000 });

    // Déplacé sous la boîte de réception, avec le même nom.
    await folderMenu(content.renamed);
    await menuItem(msg("cmd_move_folder")).waitForDisplayed({ timeout: 10000 });
    await menuItem(msg("cmd_move_folder")).click();
    const move = dialogTitled(msg("move_folder_title"));
    await move.waitForDisplayed({ timeout: 10000 });
    // Premier choix : le premier niveau ; le deuxième : la boîte de réception.
    await move.$("select").selectByIndex(1);
    await capture("move-folder");
    await move.$(`button=${msg("move_folder_confirm")}`).click();
    await move.waitForDisplayed({ reverse: true, timeout: 30000 });
    await expect($(".notice-region")).toHaveText(
      expect.stringContaining(msg("action_folder_moved", { folder: content.renamed })),
      { wait: 30000 },
    );
    await label(content.renamed).waitForDisplayed({ timeout: 30000 });
    // Affiché juste sous la boîte de réception, avant les autres dossiers
    // spéciaux.
    let shown = [];
    await browser
      .waitUntil(
        async () => {
          shown = await browser
            .$$(`${folders}//button/span[contains(concat(" ", @class, " "), " label ")]`)
            .map((e) => e.getText());
          return shown[shown.indexOf(content.renamed) - 1] === msg("mailbox_inbox");
        },
        { timeout: 30000 },
      )
      .catch(() => {
        throw new Error(`dossier pas affiché sous la boîte de réception : ${shown.join(", ")}`);
      });

    // Glissé sur la ligne du compte : revenu au premier niveau.
    const root = $("div.account[data-drop-account]");
    const pointer = () =>
      browser.action("pointer", { id: "souris", parameters: { pointerType: "mouse" } });
    await pointer()
      .move({ origin: label(content.renamed) })
      .down({ button: 0 })
      .move({ origin: label(content.renamed), x: 20, y: 10, duration: 100 })
      .move({ origin: root, duration: 300 })
      .perform(true);
    await $(".drag-ghost").waitForDisplayed({ timeout: 10000 });
    await expect(root).toHaveElementClass("drop-over");
    await capture("drag-folder");
    await pointer().move({ origin: root, duration: 50 }).up({ button: 0 }).perform();
    await $(".drag-ghost").waitForDisplayed({ reverse: true, timeout: 10000 });
    // Le dossier revenu au premier niveau n'est plus en retrait.
    await browser.waitUntil(
      async () =>
        (await label(content.renamed).parentElement().getAttribute("style"))?.includes("+ 0px)") ??
        false,
      { timeout: 30000, timeoutMsg: "dossier pas revenu au premier niveau" },
    );

    await folderMenu(content.renamed);
    await menuItem(msg("cmd_delete_folder")).waitForDisplayed({ timeout: 10000 });
    await menuItem(msg("cmd_delete_folder")).click();
    const remove = dialogTitled(msg("delete_folder_title"));
    await remove.waitForDisplayed({ timeout: 10000 });
    await capture("delete-folder");
    await remove.$(`button=${msg("delete_folder_confirm")}`).click();
    await remove.waitForDisplayed({ reverse: true, timeout: 30000 });
    await label(content.renamed).waitForDisplayed({ reverse: true, timeout: 30000 });
  });

  it("menu d'un dossier au clic droit, sous-dossier, nouveau dossier en déplaçant (à la souris)", async () => {
    const folders = `//*[@aria-label="${msg("sidebar_folders_of", { address })}"]`;
    const label = (name) => $(`${folders}//span[normalize-space()="${name}"]`);
    const inbox = msg("mailbox_inbox");

    // Clic droit sur la boîte de réception : seulement « Nouveau
    // sous-dossier… », la boîte de réception déjà choisie comme emplacement.
    await openContextMenu(label(inbox));
    await menuItem(msg("cmd_new_subfolder")).waitForDisplayed({ timeout: 10000 });
    await expect(menuItem(msg("cmd_rename_folder"))).not.toBeExisting();
    await capture("folder-context-menu");
    await menuItem(msg("cmd_new_subfolder")).click();
    const sub = dialogTitled(msg("new_folder_title"));
    await sub.waitForDisplayed({ timeout: 10000 });
    await expect(sub.$("select option:checked")).toHaveText(expect.stringContaining(inbox));
    await sub.$(`input[type="text"]`).setValue(content.subfolder);
    await sub.$(`button=${msg("new_folder_create")}`).click();
    await sub.waitForDisplayed({ reverse: true, timeout: 30000 });
    await label(content.subfolder).waitForDisplayed({ timeout: 30000 });
    // Rangé sous la boîte de réception : en retrait.
    await browser.waitUntil(
      async () =>
        (await label(content.subfolder).parentElement().getAttribute("style"))?.includes(
          "+ 12px)",
        ) ?? false,
      { timeout: 30000, timeoutMsg: "sous-dossier pas rangé sous la boîte de réception" },
    );

    // « Déplacer vers… » puis « Nouveau dossier… » : dossier créé et
    // message déplacé dedans, en un geste ; puis annulé.
    const subject = content.messages[3][1];
    const row = () => $(`//*[@role="option"][.//span[normalize-space()="${subject}"]]`);
    await $("nav")
      .$(`button*=${msg("mailbox_unified_inbox")}`)
      .click();
    await row().click();
    await expect($(`h1=${subject}`)).toBeDisplayed({ wait: 30000 });
    await openMenu(button(msg("action_more")));
    await menuItem(msg("cmd_move")).click();
    const picker = $(`[placeholder="${msg("picker_placeholder")}"]`);
    await picker.waitForDisplayed({ timeout: 10000 });
    await picker.setValue(content.newFolder);
    const create = $(`//*[@role="option"][normalize-space()="${msg("picker_new_folder")}"]`);
    await create.waitForDisplayed({ timeout: 10000 });
    await capture("move-new-folder");
    await create.click();
    const dialog = dialogTitled(msg("new_folder_title"));
    await dialog.waitForDisplayed({ timeout: 10000 });
    await expect(dialog.$(`input[type="text"]`)).toHaveValue(content.newFolder);
    await dialog.$(`button=${msg("new_folder_create_and_move")}`).click();
    await dialog.waitForDisplayed({ reverse: true, timeout: 30000 });
    await expect($(".notice-region")).toHaveText(
      expect.stringContaining(msg("action_moved", { folder: content.newFolder })),
      { wait: 30000 },
    );
    await row().waitForDisplayed({ reverse: true, timeout: 10000 });
    // Annulé (dans le délai) : le message revient dans la boîte de réception.
    await $(`//button[starts-with(normalize-space(), "${msg("action_undo")}")]`).click();
    await row().waitForDisplayed({ timeout: 30000 });
    // Le dossier créé est affiché dans le panneau de gauche.
    await label(content.newFolder).waitForDisplayed({ timeout: 30000 });

    // Dossiers de la démonstration supprimés : les captures suivantes
    // gardent le panneau de gauche d'origine.
    for (const name of [content.newFolder, content.subfolder]) {
      await openContextMenu(label(name));
      await menuItem(msg("cmd_delete_folder")).waitForDisplayed({ timeout: 10000 });
      await menuItem(msg("cmd_delete_folder")).click();
      const remove = dialogTitled(msg("delete_folder_title"));
      await remove.waitForDisplayed({ timeout: 10000 });
      await remove.$(`button=${msg("delete_folder_confirm")}`).click();
      await remove.waitForDisplayed({ reverse: true, timeout: 30000 });
      await label(name).waitForDisplayed({ reverse: true, timeout: 30000 });
    }
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

  it("contacts (à la souris)", async () => {
    await $("nav")
      .$(`button=${msg("sidebar_contacts")}`)
      .click();
    const field = (key) =>
      $(`//label[span[normalize-space()="${msg(key)}"]]//*[self::input or self::textarea]`);
    for (const [i, [given, family, email, organization]] of content.contacts.entries()) {
      await button(msg("cmd_contact_new")).click();
      const form = dialogTitled(msg("contact_dialog_new"));
      await form.waitForDisplayed();
      await field("contact_given_name").setValue(given);
      await field("contact_family_name").setValue(family);
      await $(`[aria-label="${msg("contact_email_n", { n: 1 })}"]`).setValue(email);
      if (organization !== "") await field("contact_organization").setValue(organization);
      if (i === 0) {
        await $(`[aria-label="${msg("contact_email_label_n", { n: 1 })}"]`).setValue(
          lang === "fr" ? "maison" : "home",
        );
        await button(msg("contact_add_phone")).click();
        await $(`[aria-label="${msg("contact_phone_n", { n: 1 })}"]`).setValue("06 12 34 56 78");
        await capture("contact-dialog");
        // Fonction, anniversaire, adresse postale et site web.
        const d = content.contactDetails;
        await field("contact_title").setValue(d.title);
        await $(`[aria-label="${msg("contact_birthday_day")}"]`).setValue("12");
        await $(`select[aria-label="${msg("contact_birthday_month")}"]`).selectByAttribute(
          "value",
          "4",
        );
        await $(`[aria-label="${msg("contact_birthday_year")}"]`).setValue("1985");
        await button(msg("contact_add_postal")).click();
        await $(`[aria-label="${msg("contact_street_n", { n: 1 })}"]`).setValue(d.street);
        await $(`[aria-label="${msg("contact_postal_code_n", { n: 1 })}"]`).setValue(d.code);
        await $(`[aria-label="${msg("contact_locality_n", { n: 1 })}"]`).setValue(d.city);
        await $(`[aria-label="${msg("contact_country_n", { n: 1 })}"]`).setValue(d.country);
        await $(`[aria-label="${msg("contact_postal_label_n", { n: 1 })}"]`).setValue(d.label);
        await button(msg("contact_add_website")).click();
        const website = $(`[aria-label="${msg("contact_website_n", { n: 1 })}"]`);
        await website.setValue(d.website);
        await website.scrollIntoView({ block: "center" });
        await capture("contact-dialog-details");
        // Nom qui imite une adresse : refusé, message sous le champ.
        const displayName = field("contact_display_name");
        await displayName.setValue("support@banque.fr");
        await button(msg("contact_save")).click();
        await $("#contact-display_name-error").waitForDisplayed({ timeout: 10000 });
        await capture("contact-name-refused");
        // Champ corrigé (puis vidé) : le message disparaît.
        await displayName.setValue("x");
        await browser.keys("Backspace");
        await $("#contact-display_name-error").waitForDisplayed({ reverse: true, timeout: 10000 });
      }
      await button(msg("contact_save")).click();
      await form.waitForDisplayed({ reverse: true, timeout: 10000 });
    }
    const first = `${content.contacts[0][0]} ${content.contacts[0][1]}`;
    await $(`//*[@role="option"][.//*[normalize-space()="${first}"]]`).click();
    await $(`article[aria-label="${msg("contacts_card", { name: first })}"]`).waitForDisplayed();
    await capture("contacts");
    await openContextMenu($(`//*[@role="option"][.//*[normalize-space()="${first}"]]`));
    await menuItem(msg("cmd_contact_edit")).waitForDisplayed();
    await capture("contact-menu");
    await browser.keys("Escape");
  });

  it("import de contacts (à la souris)", async () => {
    // Export CSV de Google Contacts des jeux d'essai : aperçu capturé, puis
    // annulé (les captures suivantes gardent le carnet de la démonstration).
    const file = path.resolve(
      path.dirname(fileURLToPath(import.meta.url)),
      "../../../crates/adapters/contacts-io/tests/data/gmail.csv",
    );
    await headerAction(msg("contacts_import_button"));
    await pickFile(msg("contacts_import_pick_title"), file);
    const preview = dialogTitled(msg("contacts_import_title"));
    await preview.waitForDisplayed({ timeout: 20000 });
    await preview.$(`summary=${msg("contacts_import_rejected_details")}`).click();
    await capture("contacts-import");
    await button(msg("common_cancel")).click();
    await preview.waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("export de contacts (à la souris)", async () => {
    // Fenêtre du choix du format, capturée puis fermée (le sélecteur
    // d'enregistrement du système n'est pas capturé).
    await headerAction(msg("contacts_export_button"));
    const window = dialogTitled(msg("contacts_export_title"));
    await window.waitForDisplayed({ timeout: 10000 });
    await capture("contacts-export");
    await button(msg("common_cancel")).click();
    await window.waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("doublons de contacts (à la souris)", async () => {
    // Une seconde fiche au nom du premier contact, saisie prénom et nom
    // inversés : le groupe est capturé, puis fusionné dans la fiche de la
    // démonstration (la plus complète, gardée : son nom et son adresse
    // préférée ne changent pas pour les captures suivantes).
    const [given, family] = content.contacts[0];
    const field = (key) =>
      $(`//label[span[normalize-space()="${msg(key)}"]]//*[self::input or self::textarea]`);
    await button(msg("cmd_contact_new")).click();
    const form = dialogTitled(msg("contact_dialog_new"));
    await form.waitForDisplayed();
    await field("contact_given_name").setValue(family);
    await field("contact_family_name").setValue(given);
    await $(`[aria-label="${msg("contact_email_n", { n: 1 })}"]`).setValue(
      `${given.toLowerCase()}.${family.toLowerCase()}@norelly.test`,
    );
    await button(msg("contact_save")).click();
    await form.waitForDisplayed({ reverse: true, timeout: 10000 });
    await headerAction(msg("cmd_contacts_find_duplicates"));
    const window = dialogTitled(msg("contacts_duplicates_title"));
    await window.waitForDisplayed({ timeout: 10000 });
    await window.$(`button=${msg("contacts_duplicates_merge")}`).waitForDisplayed();
    await capture("contacts-duplicates");
    await window.$(`button=${msg("contacts_duplicates_merge")}`).click();
    await window.$(`p=${msg("contacts_duplicates_done")}`).waitForDisplayed({ timeout: 10000 });
    await window.$(`button=${msg("common_close")}`).click();
    await window.waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("synchronisation du carnet avec un serveur (à la souris)", async () => {
    // Carnet trouvé sur le faux serveur CardDAV, capturé avant d'être
    // relié, puis fenêtre fermée : les captures suivantes gardent le
    // carnet de la démonstration.
    const field = (key) => $(`//label[span[normalize-space()="${msg(key)}"]]//input`);
    await headerAction(msg("cmd_contacts_carddav"));
    const window = dialogTitled(msg("carddav_title"));
    await window.waitForDisplayed({ timeout: 10000 });
    await field("carddav_server").setValue("carddav.norelly.test");
    await field("carddav_username").setValue("norelly");
    await field("carddav_password").setValue(password);
    await window.$(`button=${msg("carddav_search")}`).click();
    await window.$('input[type="radio"][name="carddav-book"]').waitForDisplayed({ timeout: 15000 });
    await capture("contacts-carddav");
    await button(msg("common_cancel")).click();
    await window.waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("photo d'un contact (à la souris)", async () => {
    // vCard des jeux d'essai : une fiche avec photo intégrée, une avec photo
    // par adresse (jamais chargée), une avec photo piégée (écartée). La
    // fiche avec photo est capturée (l'initiale l'est par « contacts ») ; les
    // trois contacts sont ensuite supprimés (carnet de la démonstration
    // inchangé).
    const file = path.resolve(
      path.dirname(fileURLToPath(import.meta.url)),
      "../../../crates/adapters/contacts-io/tests/data/photo.vcf",
    );
    await headerAction(msg("contacts_import_button"));
    await pickFile(msg("contacts_import_pick_title"), file);
    const preview = dialogTitled(msg("contacts_import_title"));
    await preview.waitForDisplayed({ timeout: 20000 });
    await preview.$(`button=${msg("contacts_import_confirm")}`).click();
    await preview.waitForDisplayed({ reverse: true, timeout: 20000 });
    const row = (name) => $(`//*[@role="option"][.//*[normalize-space()="${name}"]]`);
    await row("Camille Rousseau").click();
    await $(
      `article[aria-label="${msg("contacts_card", { name: "Camille Rousseau" })}"]`,
    ).waitForDisplayed();
    await browser.waitUntil(
      () =>
        browser.execute(() => {
          const img = document.querySelector("article .avatar img");
          return img instanceof HTMLImageElement && img.complete && img.naturalWidth > 0;
        }),
      { timeout: 10000, timeoutMsg: "photo non chargée" },
    );
    await capture("contacts-photo");
    for (const name of ["Camille Rousseau", "Hugo Lemaire", "Inès Faure"]) {
      await row(name).$('input[type="checkbox"]').click();
    }
    const selection = $(`section[aria-label="${msg("contacts_selection")}"]`);
    const visible = `[not(ancestor::*[@aria-hidden="true"])]`;
    const direct = selection.$(
      `.//button[normalize-space()="${msg("cmd_contact_delete")}"]${visible}`,
    );
    if (await direct.isDisplayed().catch(() => false)) {
      await direct.click();
    } else {
      await openMenu(selection.$(`.//button[normalize-space()="${msg("action_more")}"]${visible}`));
      await menuItem(msg("cmd_contact_delete")).click();
    }
    const confirm = dialogTitled(msg("delete_contacts_title_many"));
    await confirm.waitForDisplayed();
    await confirm.$(`button=${msg("delete_contacts_confirm")}`).click();
    await confirm.waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("listes de diffusion (à la souris)", async () => {
    const field = (key) =>
      $(`//label[span[normalize-space()="${msg(key)}"]]//*[self::input or self::textarea]`);
    await button(msg("contacts_filter_lists")).click();
    await button(msg("cmd_list_new")).click();
    const form = dialogTitled(msg("list_dialog_new"));
    await form.waitForDisplayed();
    await field("list_name").setValue(content.list.name);
    await field("list_description").setValue(content.list.description);
    await form.$('input[type="checkbox"]').click();
    for (const [query, name] of content.list.members) {
      await form.$(`input[aria-label="${msg("list_find_member")}"]`).setValue(query);
      const add = form.$(`button[aria-label="${msg("list_add_member", { name })}"]`);
      await add.waitForDisplayed({ timeout: 10000 });
      await add.click();
    }
    // Recherche laissée ouverte : le contact déjà membre est signalé.
    await form
      .$(`input[aria-label="${msg("list_find_member")}"]`)
      .setValue(content.list.members[0][0]);
    await form.$(`span*=${msg("list_already_member")}`).waitForDisplayed({ timeout: 10000 });
    await capture("contact-list-dialog");
    await button(msg("list_save")).click();
    await form.waitForDisplayed({ reverse: true, timeout: 10000 });
    await $(
      `article[aria-label="${msg("lists_card", { name: content.list.name })}"]`,
    ).waitForDisplayed();
    await capture("contact-lists");
    await button(msg("contacts_filter_all")).click();
  });

  it("personnes d'un message et carnet (à la souris)", async () => {
    await $("nav")
      .$(`button*=${msg("mailbox_unified_inbox")}`)
      .click();
    // Expéditeur déjà dans le carnet : pastille et « Voir le contact ».
    const [from, subject] = content.messages[0];
    const name = from.replace(/ <.*$/, "");
    await $(`span=${subject}`).click();
    await expect($(`h1=${subject}`)).toBeDisplayed({ wait: 30000 });
    const known = `${msg("person_menu", { name })} (${msg("person_in_contacts")})`;
    await openMenu(button(known));
    await menuItem(msg("cmd_person_show")).waitForDisplayed({ timeout: 10000 });
    await capture("person-menu");
    await browser.keys("Escape");
    // Retour à la liste : le menu d'une ligne vise la conversation ouverte.
    await button(msg("conversation_back")).click();
    await $(`h1=${subject}`).waitForDisplayed({ reverse: true, timeout: 10000 });

    // Expéditeur inconnu : « Ajouter au carnet d'adresses », fiche
    // pré-remplie.
    const [stranger, other] = content.messages.at(-1);
    await openContextMenu($(`span=${other}`));
    await menuItem(msg("cmd_person_add")).waitForDisplayed({ timeout: 10000 });
    await capture("person-add-menu");
    await menuItem(msg("cmd_person_add")).click();
    const form = dialogTitled(msg("contact_dialog_new"));
    await form.waitForDisplayed();
    await expect($(`[aria-label="${msg("contact_email_n", { n: 1 })}"]`)).toHaveValue(
      stranger.replace(/^.*<(.*)>$/, "$1"),
    );
    await capture("person-add-dialog");
    await button(msg("common_cancel")).click();
    await form.waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("suggestions de destinataires : contact du carnet signalé", async () => {
    await button(msg("cmd_compose")).click();
    const dialog = dialogTitled(msg("compose_title_new"));
    await dialog.waitForDisplayed({ timeout: 20000 });
    await dialog.$(`[aria-label="${msg("compose_to")}"]`).click();
    // Un contact (Jeanne Leroy, Jane Lee) et une adresse reçue (le cabinet).
    await browser.keys(content.suggest);
    await dialog.$('[role="listbox"]').waitForDisplayed({ timeout: 10000 });
    await dialog.$$('[role="option"] button')[1].moveTo();
    await capture("compose-suggestions");
    await browser.keys("Escape");
    await dialog.waitForDisplayed({ reverse: true, timeout: 20000 });
  });

  it("suggestions de destinataires : liste de diffusion", async () => {
    await button(msg("cmd_compose")).click();
    const dialog = dialogTitled(msg("compose_title_new"));
    await dialog.waitForDisplayed({ timeout: 20000 });
    await dialog.$(`[aria-label="${msg("compose_to")}"]`).click();
    await browser.keys(content.list.suggest);
    await dialog.$('[role="listbox"]').waitForDisplayed({ timeout: 10000 });
    await dialog.$$('[role="option"] button')[0].moveTo();
    await capture("compose-list-suggestion");
    await browser.keys("Escape");
    await dialog.waitForDisplayed({ reverse: true, timeout: 20000 });
  });

  it("expéditeur du carnet non vérifié : avertissement, sans pastille", async () => {
    // Même adresse qu'un contact, mais le serveur du compte constate un
    // échec : « Expéditeur non vérifié », jamais « Dans vos contacts ».
    const [from] = content.messages[0];
    const name = from.replace(/ <.*$/, "");
    const subject = lang === "fr" ? "Facture à régler aujourd'hui" : "Invoice due today";
    const date = new Date(Date.UTC(2026, 10, 5, 8, 0, 0)).toUTCString().replace("GMT", "+0000");
    await appendMessages(
      [
        `Authentication-Results: mx.norelly.test; dkim=none; dmarc=fail header.from=norelly.test
Authentication-Results: mx.norelly.test; dmarc=pass header.from=norelly.test
From: ${from}
To: ${content.name} <${address}>
Subject: ${subject}
Date: ${date}
Message-ID: <capture-${lang}-non-verifie@norelly.test>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

${lang === "fr" ? "Merci de régler la facture jointe." : "Please pay the attached invoice."}
`,
      ],
      user,
    );
    await $("nav")
      .$(`button*=${msg("mailbox_unified_inbox")}`)
      .click();
    const row = $(`span=${subject}`);
    await row.waitForDisplayed({ timeout: 90000 });
    await row.click();
    await expect($(`h1=${subject}`)).toBeDisplayed({ wait: 30000 });
    const unverified = button(`${msg("person_menu", { name })} (${msg("person_unverified")})`);
    await unverified.waitForDisplayed({ timeout: 10000 });
    await expect(unverified.$(`[title="${msg("person_in_contacts")}"]`)).not.toBeExisting();
    await capture("sender-unverified");
    await button(msg("conversation_back")).click();
    await $(`h1=${subject}`).waitForDisplayed({ reverse: true, timeout: 10000 });
  });

  it("expéditeur au nom en forme de domaine : adresse réelle affichée", async () => {
    // Nom d'en-tête qui ressemble à un site, longue adresse réelle : la
    // liste montre l'adresse (abrégée) puis le nom.
    const subject = lang === "fr" ? "Votre compte est suspendu" : "Your account is suspended";
    const date = new Date(Date.UTC(2026, 10, 5, 9, 0, 0)).toUTCString().replace("GMT", "+0000");
    await appendMessages(
      [
        `From: Banque.fr <service.client.securite.alerte@exemple.net>
To: ${content.name} <${address}>
Subject: ${subject}
Date: ${date}
Message-ID: <capture-${lang}-domaine@norelly.test>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

${lang === "fr" ? "Cliquez ici." : "Click here."}
`,
      ],
      user,
    );
    await $("nav")
      .$(`button*=${msg("mailbox_unified_inbox")}`)
      .click();
    const row = $(`//*[@role="option"][.//*[normalize-space()="${subject}"]]`);
    await row.waitForDisplayed({ timeout: 90000 });
    // Annonce d'une action précédente refermée (« Liste créée »).
    await browser.waitUntil(async () => (await $(".notice-region").getText()) === "", {
      timeout: 20000,
    });
    await capture("sender-domain-name");
  });
});
