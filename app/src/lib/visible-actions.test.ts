import { describe, expect, it } from "vitest";
import {
  bulkActions,
  contactBulkActions,
  contactMenuActions,
  contactsHeaderActions,
  contactsToolbar,
  contactListMenuActions,
  contactListToolbar,
  fitByPriority,
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
      "action.move",
      "reading.plain-text",
      "conversation.previous",
      "conversation.next",
    ]);
    // Conversation non lue : la bascule propose « marquer comme lu ».
    expect(ids(readerActions({ unread: 2, flagged: false }).more)[0]).toBe("action.mark-read");
  });

  it("menu « Plus… » en groupes : état, affichage, navigation", () => {
    const sections = readerActions(null).more.map((a) => a.section);
    expect(sections).toEqual([1, 1, 1, 2, 3, 3]);
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

  it("barre groupée : archiver, supprimer, lu, non lu, déplacer ; suivi dans « Plus… »", () => {
    const { primary, more } = bulkActions();
    expect(ids(primary)).toEqual([
      "action.archive",
      "action.delete",
      "action.mark-read",
      "action.mark-unread",
      "action.move",
    ]);
    expect(ids(more)).toEqual(["action.flag"]);
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
      "action.move",
    ]);
    expect(actions.map((a) => a.section ?? 0)).toEqual([0, 1, 1, 1, 2, 2, 3, 3, 3]);
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

  it("en-tête de la vue : nouvelle liste puis importer, importer replié dans « Plus… » d'abord", () => {
    const { primary, more } = contactsHeaderActions();
    expect(ids(primary)).toEqual(["lists.new", "contacts.import"]);
    expect(more).toEqual([]);
    const byPriority = [...primary].sort((a, b) => (a.priority ?? 0) - (b.priority ?? 0));
    expect(ids(byPriority)).toEqual(["lists.new", "contacts.import"]);
    expect(primary[1]?.label?.()).toBe("Importer…");
  });

  it("barre des contacts cochés : écrire, supprimer, puis ajouter à une liste", () => {
    expect(ids(contactBulkActions().primary)).toEqual(["contacts.write", "contacts.delete"]);
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

  it("garde toujours le premier bouton, même trop large", () => {
    expect(fitByPriority(items, 50, 60, 10)).toEqual([true, false, false, false, false]);
  });

  it("largeurs inconnues (pas encore mesurées) : tout est visible", () => {
    expect(fitByPriority(items, 0, 60, 10)).toEqual([true, true, true, true, true]);
  });
});
