import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConversationDto, FolderDto, SearchHit } from "./ipc/api";
import {
  DRAG_THRESHOLD,
  DragMove,
  EDGE_SCROLL_BAND,
  EDGE_SCROLL_MAX,
  LONG_PRESS_MS,
  canDrop,
  dragPayload,
  edgeScroll,
  folderPayload,
  hitPayload,
  openedPayload,
} from "./drag-move.svelte";

function conversation(id: number, account = 1, mailbox = 10): ConversationDto {
  return {
    id,
    account,
    mailbox,
    date: 0,
    subject: `Sujet ${id}`,
    senders: [],
    messages: 1,
    unread: 0,
    flagged: false,
    has_attachments: false,
  };
}

/** Valeur présente, sinon échec du test. */
function must<T>(value: T | null | undefined): T {
  if (value === null || value === undefined) throw new Error("valeur attendue");
  return value;
}

/** Appui d'un pointeur de type `pointerType` en (100, 100). */
function pointer(pointerType: string, pointerId = 1) {
  return { pointerType, pointerId, clientX: 100, clientY: 100 };
}

function folder(id: number, selectable = true): FolderDto {
  return {
    id,
    name: `F${id}`,
    leaf: `F${id}`,
    depth: 0,
    indent: 0,
    label: `F${id}`,
    role: "normal",
    selectable,
    total: 0,
    unread: 0,
  };
}

describe("conversations emportées", () => {
  const list = [conversation(1), conversation(2), conversation(3)];

  it("la ligne saisie seule, si elle n'est pas cochée", () => {
    expect(dragPayload(must(list[1]), [1, 3], list)).toEqual({
      kind: "conversations",
      ids: [2],
      account: 1,
      mailboxes: [10],
      label: "Sujet 2",
    });
  });

  it("toutes les lignes cochées, si la ligne saisie en fait partie", () => {
    expect(dragPayload(must(list[0]), [1, 3], list)).toMatchObject({ ids: [1, 3], label: null });
  });

  it("rien si les conversations cochées viennent de plusieurs comptes", () => {
    const mixed = [conversation(1, 1), conversation(2, 2)];
    expect(dragPayload(must(mixed[0]), [1, 2], mixed)).toBeNull();
  });
});

describe("dossiers où déposer", () => {
  const payload = must(dragPayload(conversation(1, 1, 10), [], [conversation(1, 1, 10)]));

  it("un dossier du même compte, autre que celui d'origine", () => {
    expect(canDrop(payload, { account: 1, folder: folder(11) })).toBe(true);
  });

  it("ni un autre compte, ni le dossier d'origine, ni un dossier non sélectionnable", () => {
    expect(canDrop(payload, { account: 2, folder: folder(11) })).toBe(false);
    expect(canDrop(payload, { account: 1, folder: folder(10) })).toBe(false);
    expect(canDrop(payload, { account: 1, folder: folder(11, false) })).toBe(false);
    // Le premier niveau (ligne du compte) ne reçoit pas de messages.
    expect(canDrop(payload, { account: 1, folder: null })).toBe(false);
  });

  it("le dossier d'origine reste une cible si d'autres conversations viennent d'ailleurs", () => {
    const list = [conversation(1, 1, 10), conversation(2, 1, 12)];
    const both = must(dragPayload(must(list[0]), [1, 2], list));
    expect(canDrop(both, { account: 1, folder: folder(10) })).toBe(true);
  });
});

describe("geste de glisser", () => {
  const payload = must(dragPayload(conversation(1), [], [conversation(1)]));
  const target = { account: 1, folder: folder(11) };

  it("ne commence qu'au-delà du seuil : un simple clic reste un clic", () => {
    const drag = new DragMove();
    drag.press(payload, 100, 100);
    drag.move(100 + DRAG_THRESHOLD - 1, 100, null);
    expect(drag.payload).toBeNull();
    expect(drag.release()).toBeNull();
    expect(drag.suppressClick()).toBe(false);
  });

  it("dépose sur un dossier valide survolé", () => {
    const drag = new DragMove();
    drag.press(payload, 100, 100);
    drag.move(140, 120, null);
    expect(drag.payload).toEqual(payload);
    expect([drag.x, drag.y]).toEqual([140, 120]);
    drag.move(150, 130, target);
    expect(drag.over).toBe(11);
    expect(drag.release()).toEqual({ payload, account: 1, folder: target.folder });
    expect(drag.payload).toBeNull();
    // Le clic qui suit le relâchement n'ouvre pas la conversation.
    expect(drag.suppressClick()).toBe(true);
    expect(drag.suppressClick()).toBe(false);
  });

  it("ne dépose rien hors d'un dossier valide", () => {
    const drag = new DragMove();
    drag.press(payload, 100, 100);
    drag.move(140, 120, { account: 2, folder: folder(20) });
    expect(drag.over).toBeNull();
    expect(drag.release()).toBeNull();
  });

  it("Échap annule le glisser", () => {
    const drag = new DragMove();
    drag.press(payload, 100, 100);
    drag.move(140, 120, target);
    drag.cancel();
    expect(drag.payload).toBeNull();
    expect(drag.release()).toBeNull();
  });
});

describe("glisser un dossier", () => {
  function named(id: number, name: string, role = "normal", selectable = true): FolderDto {
    return {
      ...folder(id, selectable),
      name,
      leaf: name.split("/").at(-1) ?? name,
      depth: name.split("/").length - 1,
      indent: name.split("/").length - 1,
      label: name.split("/").at(-1) ?? name,
      role,
    };
  }
  const inbox = named(1, "INBOX", "inbox");
  const bla = named(2, "Bla");
  const bli = named(3, "Bli");
  const sub = named(4, "Bli/2026");
  const deep = named(5, "Bli/2026/Mars");
  const blabla = named(6, "Bli bla");
  const all = named(7, "Tous", "all");
  const box = named(8, "Conteneur", "normal", false);

  it("seuls les dossiers ordinaires s'emportent", () => {
    expect(folderPayload(1, bli)).toEqual({
      kind: "folder",
      account: 1,
      folder: bli,
      label: "Bli",
    });
    expect(folderPayload(1, inbox)).toBeNull();
    expect(folderPayload(1, named(9, "Envoyés", "sent"))).toBeNull();
  });

  it("porte le libellé affiché du dossier", () => {
    const orphan = { ...named(10, "INBOX/Perso/Banque"), indent: 1, label: "Perso/Banque" };
    expect(folderPayload(1, orphan)?.label).toBe("Perso/Banque");
  });

  it("se dépose dans un autre dossier du compte, même non sélectionnable", () => {
    const payload = must(folderPayload(1, bli));
    for (const target of [inbox, bla, blabla, box]) {
      expect(canDrop(payload, { account: 1, folder: target }), target.name).toBe(true);
    }
  });

  it("ni dans lui-même, ni dans ses sous-dossiers, ni ailleurs que son compte", () => {
    const payload = must(folderPayload(1, bli));
    for (const target of [bli, sub, deep, all]) {
      expect(canDrop(payload, { account: 1, folder: target }), target.name).toBe(false);
    }
    expect(canDrop(payload, { account: 2, folder: bla })).toBe(false);
  });

  it("pas là où il est déjà ; le premier niveau seulement s'il n'y est pas", () => {
    expect(canDrop(must(folderPayload(1, sub)), { account: 1, folder: bli })).toBe(false);
    expect(canDrop(must(folderPayload(1, bli)), { account: 1, folder: null })).toBe(false);
    expect(canDrop(must(folderPayload(1, sub)), { account: 1, folder: null })).toBe(true);
  });

  it("le geste signale le premier niveau survolé et le dépôt", () => {
    const drag = new DragMove();
    const payload = must(folderPayload(1, sub));
    drag.press(payload, 100, 100);
    drag.move(140, 120, { account: 1, folder: null });
    expect(drag.over).toBeNull();
    expect(drag.overRoot).toBe(1);
    expect(drag.release()).toEqual({ payload, account: 1, folder: null });
    expect(drag.overRoot).toBeNull();
  });
});

describe("glisser depuis la recherche ou le message ouvert", () => {
  function hit(conversation: number | null): SearchHit {
    return {
      message: 7,
      conversation,
      account: 2,
      mailbox: 30,
      date: 0,
      from: null,
      subject: [
        { text: "Facture", hit: true },
        { text: " de mars", hit: false },
      ],
      snippet: [],
      unread: false,
      flagged: false,
      has_attachments: false,
    };
  }

  it("un résultat emporte sa conversation, agie dans le dossier du résultat", () => {
    expect(hitPayload(hit(5))).toEqual({
      kind: "conversations",
      ids: [5],
      account: 2,
      mailboxes: [30],
      label: "Facture de mars",
      scope: { kind: "mailbox", id: 30 },
    });
  });

  it("un résultat pas encore rattaché à une conversation ne s'emporte pas", () => {
    expect(hitPayload(hit(null))).toBeNull();
  });

  it("le message ouvert emporte sa conversation, dans la portée où elle est ouverte", () => {
    const payload = openedPayload(conversation(4, 1, 12), { kind: "mailbox", id: 12 });
    expect(payload).toEqual({
      kind: "conversations",
      ids: [4],
      account: 1,
      mailboxes: [12],
      label: "Sujet 4",
      scope: { kind: "mailbox", id: 12 },
    });
    expect(canDrop(payload, { account: 1, folder: folder(12) })).toBe(false);
    expect(canDrop(payload, { account: 1, folder: folder(13) })).toBe(true);
  });
});

describe("défilement automatique du panneau", () => {
  const rect = { left: 0, right: 200, top: 100, bottom: 600 };

  it("immobile loin des bords ou hors du panneau", () => {
    expect(edgeScroll(100, 350, rect)).toBe(0);
    expect(edgeScroll(100, 100 + EDGE_SCROLL_BAND, rect)).toBe(0);
    expect(edgeScroll(100, 600 - EDGE_SCROLL_BAND, rect)).toBe(0);
    // À droite du panneau (liste des conversations) : pas de défilement.
    expect(edgeScroll(250, 105, rect)).toBe(0);
  });

  it("vers le haut près du bord haut, plus vite au plus près", () => {
    const near = edgeScroll(100, 105, rect);
    const far = edgeScroll(100, 100 + EDGE_SCROLL_BAND - 10, rect);
    expect(near).toBeLessThan(far);
    expect(far).toBeLessThan(0);
    expect(edgeScroll(100, 100, rect)).toBe(-EDGE_SCROLL_MAX);
    // Au-delà du bord : vitesse maximale.
    expect(edgeScroll(100, 40, rect)).toBe(-EDGE_SCROLL_MAX);
  });

  it("vers le bas près du bord bas, symétrique", () => {
    expect(edgeScroll(100, 595, rect)).toBe(-edgeScroll(100, 105, rect));
    expect(edgeScroll(100, 600, rect)).toBe(EDGE_SCROLL_MAX);
    expect(edgeScroll(100, 700, rect)).toBe(EDGE_SCROLL_MAX);
  });

  it("au moins un pixel par image dans la bande (pas de pas fractionnaire perdu)", () => {
    expect(edgeScroll(100, 100 + EDGE_SCROLL_BAND - 0.5, rect)).toBe(-1);
  });
});

describe("glisser au toucher", () => {
  const payload = must(dragPayload(conversation(1), [], [conversation(1)]));
  const target = { account: 1, folder: folder(11) };

  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("commence après un appui long, à l'endroit du doigt", () => {
    const drag = new DragMove();
    drag.pressTouch(payload, 50, 60, true);
    vi.advanceTimersByTime(LONG_PRESS_MS - 1);
    expect(drag.payload).toBeNull();
    vi.advanceTimersByTime(1);
    expect(drag.payload).toEqual(payload);
    expect([drag.x, drag.y]).toEqual([50, 60]);
    drag.move(80, 200, target);
    expect(drag.over).toBe(11);
    expect(drag.release()).toEqual({ payload, ...target });
    expect(drag.suppressClick()).toBe(true);
  });

  it("un doigt qui bouge avant l'appui long fait défiler : pas de glisser", () => {
    const drag = new DragMove();
    drag.pressTouch(payload, 50, 60, true);
    drag.move(50, 60 + DRAG_THRESHOLD, null);
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(drag.payload).toBeNull();
    expect(drag.release()).toBeNull();
    expect(drag.suppressClick()).toBe(false);
  });

  it("un tremblement sous le seuil n'empêche pas l'appui long", () => {
    const drag = new DragMove();
    drag.pressTouch(payload, 50, 60, true);
    drag.move(51, 61, null);
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(drag.payload).toEqual(payload);
  });

  it("doigt levé avant l'appui long : un simple toucher", () => {
    const drag = new DragMove();
    drag.pressTouch(payload, 50, 60, true);
    expect(drag.release()).toBeNull();
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(drag.payload).toBeNull();
    expect(drag.suppressClick()).toBe(false);
  });

  it("annulé (défilement natif, Échap) : l'appui long ne démarre plus rien", () => {
    const drag = new DragMove();
    drag.pressTouch(payload, 50, 60, true);
    drag.cancel();
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(drag.payload).toBeNull();
  });

  it("le menu de l'appui long l'emporte tant que le doigt n'a pas bougé", () => {
    const drag = new DragMove();
    drag.pressTouch(payload, 50, 60, true);
    vi.advanceTimersByTime(LONG_PRESS_MS);
    drag.move(51, 60, null);
    // Le menu s'ouvre : le glisser s'efface sans être bloqué.
    expect(drag.contextMenu()).toBe(false);
    expect(drag.payload).toBeNull();
  });

  it("une fois le doigt parti, le menu de l'appui long est retenu", () => {
    const drag = new DragMove();
    drag.pressTouch(payload, 50, 60, true);
    vi.advanceTimersByTime(LONG_PRESS_MS);
    drag.move(50, 60 + DRAG_THRESHOLD, null);
    expect(drag.contextMenu()).toBe(true);
    expect(drag.payload).toEqual(payload);
  });

  it("le menu de l'appui long qui arrive au lever du doigt, après un dépôt, est retenu", () => {
    const drag = new DragMove();
    drag.pressTouch(payload, 50, 60, true);
    vi.advanceTimersByTime(LONG_PRESS_MS);
    drag.move(80, 200, target);
    expect(drag.release()).not.toBeNull();
    expect(drag.contextMenu()).toBe(true);
    // Une seule fois : le menu suivant est libre.
    expect(drag.contextMenu()).toBe(false);
  });

  it("passé un court délai après le lever du doigt, le menu est libre", () => {
    const drag = new DragMove();
    drag.pressTouch(payload, 50, 60, true);
    vi.advanceTimersByTime(LONG_PRESS_MS);
    drag.move(80, 200, target);
    drag.release();
    vi.advanceTimersByTime(1000);
    expect(drag.contextMenu()).toBe(false);
  });

  it("après un glisser à la souris, le clic droit suivant est libre", () => {
    const drag = new DragMove();
    drag.press(payload, 100, 100);
    drag.move(140, 120, target);
    drag.release();
    expect(drag.contextMenu()).toBe(false);
  });

  it("menu de l'appui long avant qu'il ne commence : l'appui est abandonné", () => {
    const drag = new DragMove();
    drag.pressTouch(payload, 50, 60, true);
    expect(drag.contextMenu()).toBe(false);
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(drag.payload).toBeNull();
  });

  it("menu contextuel libre sans glisser, retenu pendant un glisser à la souris", () => {
    const drag = new DragMove();
    expect(drag.contextMenu()).toBe(false);
    drag.press(payload, 100, 100);
    drag.move(140, 120, null);
    expect(drag.contextMenu()).toBe(true);
  });
});

describe("saisie selon le pointeur", () => {
  const payload = must(dragPayload(conversation(1), [], [conversation(1)]));

  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("souris et stylet : glisser dès le seuil franchi", () => {
    for (const kind of ["mouse", "pen"]) {
      const drag = new DragMove();
      drag.grab(payload, pointer(kind));
      drag.move(100 + DRAG_THRESHOLD, 100, null);
      expect(drag.payload, kind).toEqual(payload);
    }
  });

  it("doigt : appui long d'abord", () => {
    const drag = new DragMove();
    drag.grab(payload, pointer("touch"));
    expect(drag.payload).toBeNull();
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(drag.payload).toEqual(payload);
  });

  it("pointeur inconnu : rien", () => {
    const drag = new DragMove();
    drag.grab(payload, pointer(""));
    drag.move(200, 200, null);
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(drag.payload).toBeNull();
  });

  it("seul le pointeur qui a saisi mène le glisser", () => {
    const drag = new DragMove();
    expect(drag.owns(9)).toBe(true);
    drag.grab(payload, pointer("touch", 3));
    expect(drag.owns(3)).toBe(true);
    expect(drag.owns(4)).toBe(false);
    drag.cancel();
    expect(drag.owns(4)).toBe(true);
  });

  it("doigt posé sur un élément saisissable : signalé jusqu'au lever", () => {
    const drag = new DragMove();
    expect(drag.touching).toBe(false);
    drag.grab(payload, pointer("touch"));
    expect(drag.touching).toBe(true);
    drag.release();
    expect(drag.touching).toBe(false);
    drag.grab(payload, pointer("mouse"));
    expect(drag.touching).toBe(false);
  });

  it("sans menu derrière (titre, résultat), l'appui long prolongé garde le glisser", () => {
    const drag = new DragMove();
    drag.grab(payload, pointer("touch"));
    vi.advanceTimersByTime(LONG_PRESS_MS);
    // Menu natif de la webview au bout de l'appui : retenu, glisser gardé.
    expect(drag.contextMenu()).toBe(true);
    expect(drag.payload).toEqual(payload);
  });

  it("avec un menu derrière (ligne de la liste), le menu l'emporte", () => {
    const drag = new DragMove();
    drag.grab(payload, pointer("touch"), true);
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(drag.contextMenu()).toBe(false);
    expect(drag.payload).toBeNull();
  });
});

describe("après le glisser", () => {
  const payload = must(dragPayload(conversation(1), [], [conversation(1)]));
  const target = { account: 1, folder: folder(11) };

  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("le clic retenu ne vaut que pour l'événement qui suit le relâchement", () => {
    const drag = new DragMove();
    drag.press(payload, 100, 100);
    drag.move(140, 120, target);
    drag.release();
    // Aucun clic sur l'élément saisi (relâché ailleurs) : le prochain clic,
    // plus tard, sur un autre élément, n'est pas avalé.
    vi.advanceTimersByTime(0);
    expect(drag.suppressClick()).toBe(false);
  });

  it("idem après une annulation", () => {
    const drag = new DragMove();
    drag.press(payload, 100, 100);
    drag.move(140, 120, target);
    drag.cancel();
    vi.advanceTimersByTime(0);
    expect(drag.suppressClick()).toBe(false);
  });
});

describe("défilement automatique engagé", () => {
  const payload = must(dragPayload(conversation(1), [], [conversation(1)]));

  it("seulement une fois le pointeur éloigné du point de saisie", () => {
    const drag = new DragMove();
    drag.press(payload, 100, 100);
    drag.move(100, 100 + DRAG_THRESHOLD, null);
    expect(drag.payload).not.toBeNull();
    // Saisi près d'un bord : le panneau ne file pas sous le pointeur.
    expect(drag.autoScroll).toBe(false);
    drag.move(100, 100 + EDGE_SCROLL_BAND, null);
    expect(drag.autoScroll).toBe(true);
    // Revenu près du point de saisie : reste engagé.
    drag.move(100, 101, null);
    expect(drag.autoScroll).toBe(true);
    drag.release();
    expect(drag.autoScroll).toBe(false);
  });
});
