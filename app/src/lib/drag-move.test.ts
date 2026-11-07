import { describe, expect, it } from "vitest";
import type { ConversationDto, FolderDto } from "./ipc/api";
import { DRAG_THRESHOLD, DragMove, canDrop, dragPayload } from "./drag-move.svelte";

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

function folder(id: number, selectable = true): FolderDto {
  return {
    id,
    name: `F${id}`,
    leaf: `F${id}`,
    depth: 0,
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
    expect(canDrop(payload, 1, folder(11))).toBe(true);
  });

  it("ni un autre compte, ni le dossier d'origine, ni un dossier non sélectionnable", () => {
    expect(canDrop(payload, 2, folder(11))).toBe(false);
    expect(canDrop(payload, 1, folder(10))).toBe(false);
    expect(canDrop(payload, 1, folder(11, false))).toBe(false);
  });

  it("le dossier d'origine reste une cible si d'autres conversations viennent d'ailleurs", () => {
    const list = [conversation(1, 1, 10), conversation(2, 1, 12)];
    const both = must(dragPayload(must(list[0]), [1, 2], list));
    expect(canDrop(both, 1, folder(10))).toBe(true);
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
