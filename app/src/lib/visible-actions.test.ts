import { describe, expect, it } from "vitest";
import { fitByPriority, readerActions, rowActions } from "./visible-actions";

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
