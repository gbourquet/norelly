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
import { $, browser, expect } from "@wdio/globals";
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
    search: "tarte",
    suggest: "ler",
    contacts: [
      ["Alice", "Martin", "alice@norelly.test", "Association des parents d'élèves"],
      ["Paul", "Bernard", "paul@norelly.test", ""],
      ["Jeanne", "Leroy", "jeanne@norelly.test", "Cabinet médical Leroy"],
    ],
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
    search: "pie",
    suggest: "lee",
    contacts: [
      ["Alice", "Martin", "alice@norelly.test", "Parents' association"],
      ["Paul", "Brown", "paul@norelly.test", ""],
      ["Jane", "Lee", "jane@norelly.test", "Dr Lee's surgery"],
    ],
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

/** Bouton visible dont le libellé (texte ou `aria-label`) est `name` ; la
 * copie invisible de mesure des barres d'actions est ignorée. */
const button = (name) =>
  $(
    `//button[@aria-label="${name}" or normalize-space()="${name}"][not(ancestor::*[@aria-hidden="true"])]`,
  );

/** Entrée de menu dont le libellé est exactement `label` (raccourci à droite). */
const menuItem = (label) =>
  $(`//*[@role="menuitem"][.//*[contains(@class, "menu-label")][normalize-space()="${label}"]]`);

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
    await $(`[role="toolbar"][aria-label="${msg("contacts_header_actions")}"]`)
      .$(`button=${msg("contacts_import_button")}`)
      .click();
    await pickFile(msg("contacts_import_pick_title"), file);
    const preview = dialogTitled(msg("contacts_import_title"));
    await preview.waitForDisplayed({ timeout: 20000 });
    await preview.$(`summary=${msg("contacts_import_rejected_details")}`).click();
    await capture("contacts-import");
    await button(msg("common_cancel")).click();
    await preview.waitForDisplayed({ reverse: true, timeout: 10000 });
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
});
