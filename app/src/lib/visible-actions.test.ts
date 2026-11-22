import { describe, expect, it } from "vitest";
import { LIST, SIDEBAR } from "../components/ThreePaneLayout.svelte";
import {
  bulkActions,
  contactBulkActions,
  contactMenuActions,
  contactsHeaderActions,
  contactsToolbar,
  contactListMenuActions,
  contactListToolbar,
  fitByPriority,
  folderMenuActions,
  helpMenuActions,
  menuActions,
  personActions,
  readerActions,
  rowActions,
  subjectOf,
} from "./visible-actions";

const ids = (actions: { command: string }[]): string[] => actions.map((a) => a.command);

describe("actions visibles", () => {
  it("barre du mail ouvert : répondre, répondre à tous, transférer, archiver, supprimer", () => {
    const { primary } = readerActions({ unread: 0, flagged: false });
    expect(ids(primary)).toEqual([
      "compose.reply",
      "compose.reply-all",
      "compose.forward",
      "action.archive",
      "action.delete",
    ]);
  });

  it("menu « Plus… » du mail ouvert : actions secondaires et navigation", () => {
    expect(ids(readerActions({ unread: 0, flagged: false }).more)).toEqual([
      "action.mark-unread",
      "action.flag",
      "action.label",
      "action.move",
      "reading.plain-text",
      "reading.print",
      "conversation.previous",
      "conversation.next",
    ]);
    // Conversation non lue : la bascule propose « marquer comme lu ».
    expect(ids(readerActions({ unread: 2, flagged: false }).more)[0]).toBe("action.mark-read");
  });

  it("menu « Plus… » en groupes : état, affichage, navigation", () => {
    const sections = readerActions(null).more.map((a) => a.section);
    expect(sections).toEqual([1, 1, 1, 1, 2, 2, 3, 3]);
    // Les actions principales repliées dans « Plus… » forment le premier groupe.
    expect(readerActions(null).primary.every((a) => (a.section ?? 0) === 0)).toBe(true);
  });

  it("le suivi a un libellé selon l'état", () => {
    const flag = (flagged: boolean) =>
      readerActions({ unread: 0, flagged }).more.find((a) => a.command === "action.flag");
    expect(flag(false)?.label?.()).toBe("Suivre");
    expect(flag(true)?.label?.()).toBe("Ne plus suivre");
  });

  it("survol d'une ligne : archiver, supprimer, lu / non lu", () => {
    expect(ids(rowActions({ unread: 1, flagged: false }))).toEqual([
      "action.archive",
      "action.delete",
      "action.mark-read",
    ]);
    expect(ids(rowActions({ unread: 0, flagged: false }))[2]).toBe("action.mark-unread");
  });

  it("barre groupée : archiver, supprimer, lu, non lu, déplacer ; suivi et étiqueter dans « Plus… »", () => {
    const { primary, more } = bulkActions();
    expect(ids(primary)).toEqual([
      "action.archive",
      "action.delete",
      "action.mark-read",
      "action.mark-unread",
      "action.move",
    ]);
    expect(ids(more)).toEqual(["action.flag", "action.label"]);
  });

  it("menu contextuel : ouvrir, puis les actions de la barre, en groupes", () => {
    const actions = menuActions({ unread: 1, flagged: true });
    expect(ids(actions)).toEqual([
      "conversation.open",
      "compose.reply",
      "compose.reply-all",
      "compose.forward",
      "action.archive",
      "action.delete",
      "action.mark-read",
      "action.flag",
      "action.label",
      "action.move",
    ]);
    expect(actions.map((a) => a.section ?? 0)).toEqual([0, 1, 1, 1, 2, 2, 3, 3, 3, 3]);
    expect(actions.find((a) => a.command === "action.flag")?.label?.()).toBe("Ne plus suivre");
  });

  it("le reste des boutons primaires a une priorité : répondre d'abord", () => {
    const { primary } = readerActions(null);
    const byPriority = [...primary].sort((a, b) => (a.priority ?? 0) - (b.priority ?? 0));
    expect(ids(byPriority)).toEqual([
      "compose.reply",
      "action.archive",
      "action.delete",
      "compose.forward",
      "compose.reply-all",
    ]);
  });
});

describe("menu d'un dossier", () => {
  const folder = (role: string) => ({
    id: 1,
    name: "X",
    leaf: "X",
    depth: 0,
    indent: 0,
    label: "X",
    role,
    selectable: true,
    total: 0,
    unread: 0,
    labels_refused: false,
  });

  it("dossier ordinaire : nouveau sous-dossier, puis renommer, déplacer, puis supprimer", () => {
    const actions = folderMenuActions(folder("normal"));
    expect(ids(actions)).toEqual([
      "folder.new-subfolder",
      "folder.rename",
      "folder.move",
      "folder.delete",
    ]);
    expect(actions.map((a) => a.section ?? 0)).toEqual([0, 1, 1, 2]);
  });

  it("dossier spécial : seulement nouveau sous-dossier (ni renommé, ni déplacé, ni supprimé)", () => {
    for (const role of ["inbox", "sent", "drafts", "trash", "junk", "archive"]) {
      expect(ids(folderMenuActions(folder(role)))).toEqual(["folder.new-subfolder"]);
    }
  });

  it("dossier virtuel du serveur : aucune entrée", () => {
    expect(folderMenuActions(folder("all"))).toEqual([]);
    expect(folderMenuActions(folder("flagged"))).toEqual([]);
  });
});

describe("actions de la vue Contacts", () => {
  it("barre de la fiche : écrire, modifier, supprimer ; favori dans « Plus… »", () => {
    const { primary, more } = contactsToolbar({ favorite: false });
    expect(ids(primary)).toEqual(["contacts.write", "contacts.edit", "contacts.delete"]);
    expect(ids(more)).toEqual(["contacts.favorite", "contacts.add-to-list"]);
    const byPriority = [...primary].sort((a, b) => (a.priority ?? 0) - (b.priority ?? 0));
    expect(ids(byPriority)[0]).toBe("contacts.write");
  });

  it("le favori a un libellé selon l'état", () => {
    const favorite = (value: boolean | null) =>
      contactsToolbar(value === null ? null : { favorite: value }).more[0]?.label?.();
    expect(favorite(false)).toBe("Ajouter aux favoris");
    expect(favorite(null)).toBe("Ajouter aux favoris");
    expect(favorite(true)).toBe("Retirer des favoris");
  });

  it("menu contextuel d'un contact : modifier, écrire, favori, puis supprimer", () => {
    const actions = contactMenuActions({ favorite: true });
    expect(ids(actions)).toEqual([
      "contacts.edit",
      "contacts.write",
      "contacts.favorite",
      "contacts.add-to-list",
      "contacts.delete",
    ]);
    expect(actions.map((a) => a.section ?? 0)).toEqual([0, 0, 0, 0, 1]);
    expect(actions[2]?.label?.()).toBe("Retirer des favoris");
  });

  it("en-tête de la vue : nouvelle liste, importer, exporter ; exporter replié dans « Plus… » d'abord", () => {
    const { primary, more } = contactsHeaderActions();
    expect(ids(primary)).toEqual(["lists.new", "contacts.import", "contacts.export"]);
    // Recherche des doublons et synchronisation avec un serveur : actions
    // rares, toujours dans « Plus… ».
    expect(ids(more)).toEqual(["contacts.find-duplicates", "contacts.carddav"]);
    const byPriority = [...primary].sort((a, b) => (a.priority ?? 0) - (b.priority ?? 0));
    expect(ids(byPriority)).toEqual(["lists.new", "contacts.import", "contacts.export"]);
    expect(primary[1]?.label?.()).toBe("Importer…");
    expect(primary[2]?.label?.()).toBe("Exporter…");
  });

  it("barre des contacts cochés : écrire, supprimer, exporter, puis ajouter à une liste", () => {
    expect(ids(contactBulkActions().primary)).toEqual([
      "contacts.write",
      "contacts.delete",
      "contacts.export-selected",
    ]);
    expect(contactBulkActions().primary[2]?.label?.()).toBe("Exporter…");
    expect(ids(contactBulkActions().more)).toEqual(["contacts.add-to-list"]);
  });

  it("barre d'une liste : écrire, modifier, supprimer, écrire gardé en dernier", () => {
    const { primary, more } = contactListToolbar();
    expect(ids(primary)).toEqual(["lists.write", "lists.edit", "lists.delete"]);
    expect(more).toEqual([]);
    const byPriority = [...primary].sort((a, b) => (a.priority ?? 0) - (b.priority ?? 0));
    expect(ids(byPriority)[0]).toBe("lists.write");
  });

  it("menu contextuel d'une liste : modifier, écrire, puis supprimer", () => {
    const actions = contactListMenuActions();
    expect(ids(actions)).toEqual(["lists.edit", "lists.write", "lists.delete"]);
    expect(actions.map((a) => a.section ?? 0)).toEqual([0, 0, 1]);
  });
});

describe("actions sur une personne d'un message", () => {
  it("inconnue du carnet : l'ajouter ; déjà contact : voir sa fiche", () => {
    expect(ids(personActions("stranger"))).toEqual(["contacts.add-person"]);
    expect(ids(personActions("contact"))).toEqual(["contacts.show-person"]);
    expect(personActions(null)).toEqual([]);
  });

  it("dans « Plus… » du mail ouvert et le menu d'une ligne, en dernier groupe", () => {
    const subject = { unread: 0, flagged: false };
    const more = readerActions(subject, "stranger").more;
    expect(more.at(-1)?.command).toBe("contacts.add-person");
    expect(more.at(-1)?.section).toBeGreaterThan(more.at(-2)?.section ?? 0);
    const menu = menuActions(subject, "contact");
    expect(menu.at(-1)?.command).toBe("contacts.show-person");
    expect(menu.at(-1)?.section).toBeGreaterThan(menu.at(-2)?.section ?? 0);
    // Sans personne : menus inchangés.
    expect(ids(readerActions(subject).more)).not.toContain("contacts.add-person");
    expect(ids(menuActions(subject))).not.toContain("contacts.show-person");
  });
});

describe("subjectOf", () => {
  it("résume plusieurs conversations comme les commandes les traitent", () => {
    const c = (unread: number, flagged: boolean) => ({ unread, flagged });
    // Suivi : « ne plus suivre » seulement si toutes sont suivies.
    expect(subjectOf([c(0, true), c(0, false)])?.flagged).toBe(false);
    expect(subjectOf([c(0, true), c(0, true)])?.flagged).toBe(true);
    // Lu : « marquer comme lu » dès qu'une n'est pas lue.
    expect(subjectOf([c(0, false), c(2, false)])?.unread).toBeGreaterThan(0);
    expect(subjectOf([c(0, false), c(0, false)])?.unread).toBe(0);
  });

  it("sans conversation : rien à résumer", () => {
    expect(subjectOf([])).toBeNull();
  });
});

describe("fitByPriority", () => {
  // Cinq boutons de 100 px, priorités dans l'ordre 1, 4, 5, 2, 3 ; 10 px
  // entre deux boutons ; 60 px pour « Plus… ».
  const items = [
    { width: 100, priority: 1 },
    { width: 100, priority: 4 },
    { width: 100, priority: 5 },
    { width: 100, priority: 2 },
    { width: 100, priority: 3 },
  ];

  it("tout tient : tout est visible", () => {
    expect(fitByPriority(items, 1000, 60, 10)).toEqual([true, true, true, true, true]);
  });

  it("place insuffisante : les moins prioritaires passent dans « Plus… »", () => {
    // 3 boutons + « Plus… » : 300 + 60 + 3 × 10 = 390.
    expect(fitByPriority(items, 390, 60, 10)).toEqual([true, false, false, true, true]);
    expect(fitByPriority(items, 389, 60, 10)).toEqual([true, false, false, true, false]);
  });

  it("sans action secondaire, « Plus… » n'occupe de place que s'il sert", () => {
    // 5 boutons sans « Plus… » : 500 + 4 × 10 = 540.
    expect(fitByPriority(items, 540, 60, 10, false)).toEqual([true, true, true, true, true]);
    // Avec des actions secondaires, « Plus… » reste affiché : sa place est due.
    expect(fitByPriority(items, 540, 60, 10, true)).toEqual([true, true, false, true, true]);
    // Un bouton de trop : « Plus… » réapparaît, et sa place avec.
    expect(fitByPriority(items, 539, 60, 10, false)).toEqual([true, true, false, true, true]);
  });

  it("garde toujours le premier bouton, même trop large", () => {
    expect(fitByPriority(items, 50, 60, 10)).toEqual([true, false, false, false, false]);
  });

  it("largeurs inconnues (pas encore mesurées) : tout est visible", () => {
    expect(fitByPriority(items, 0, 60, 10)).toEqual([true, true, true, true, true]);
  });

  it("sur deux rangées : la seconde accueille ce qui ne tient pas sur la première", () => {
    // Rangée 1 : 3 boutons (320 px) ; rangée 2 : 2 boutons et « Plus… » (280 px).
    expect(fitByPriority(items, 320, 60, 10, true, 2)).toEqual([true, true, true, true, true]);
    // Au-delà de deux rangées, le moins prioritaire passe dans « Plus… ».
    expect(fitByPriority(items, 280, 60, 10, true, 2)).toEqual([true, true, false, true, true]);
  });

  it("sur deux rangées, le retour à la ligne suit l'ordre de la barre", () => {
    // 400 px par rangée. Le bouton large ne tient pas à côté du premier :
    // 100 | 300 + 60, deux rangées. Le troisième n'a plus de place
    // (100 | 300 | 100 + 60), bien que la somme des largeurs tienne.
    const wide = [
      { width: 100, priority: 1 },
      { width: 300, priority: 2 },
      { width: 100, priority: 3 },
    ];
    expect(fitByPriority(wide, 400, 60, 10, true, 2)).toEqual([true, true, false]);
  });

  it("une action absente (non affichée) ne prend aucune place", () => {
    // Sans l'action absente : 100 | 100 + 60 sur deux rangées de 200 px.
    const withAbsent = [
      { width: 100, priority: 1 },
      { width: 40, priority: 2, absent: true },
      { width: 100, priority: 3 },
    ];
    expect(fitByPriority(withAbsent, 200, 60, 10, true, 2)).toEqual([true, false, true]);
    expect(fitByPriority(withAbsent, 0, 60, 10, true, 2)).toEqual([true, false, true]);
  });

  it("volet de lecture à 1 280 px : les cinq boutons restent visibles", () => {
    // Largeurs mesurées (police par défaut des captures) : Répondre,
    // Répondre à tous, Transférer, Archiver, Supprimer, puis « Plus… » ;
    // volet de lecture : fenêtre moins les panneaux par défaut et les deux
    // séparateurs (--splitter-size, 5 px), moins ses marges (2 × --space-4).
    const { primary, rows } = readerActions(null);
    const available = 1280 - SIDEBAR.initial - LIST.initial - 2 * 5 - 2 * 16;
    const fr = [119, 170, 125, 110, 125];
    const en = [89, 114, 108, 103, 97];
    for (const [widths, more] of [
      [fr, 92],
      [en, 101],
    ] as const) {
      const fitted = fitByPriority(
        primary.map((a, i) => ({ width: widths[i] ?? 0, priority: a.priority ?? 0 })),
        available,
        more,
        8,
        true,
        rows,
      );
      expect(fitted).toEqual([true, true, true, true, true]);
    }
  });
});

describe("menu « Aide »", () => {
  it("guide d'utilisation, raccourcis clavier, site de Norelly", () => {
    expect(ids(helpMenuActions())).toEqual(["help.guide", "help.shortcuts", "help.website"]);
  });
});
