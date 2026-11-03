import { describe, expect, it, vi } from "vitest";
import type { ActionDto, ConversationDto, ScopeDto } from "./ipc/api";
import { MailState, PAGE_SIZE, UNDO_WINDOW_MS, UNIFIED_INBOX, type MailApi } from "./mail.svelte";
import { IpcFailure } from "./errors";

function conversation(id: number, account = 1): ConversationDto {
  return {
    id,
    account,
    mailbox: 10,
    date: 1_700_000_000 - id,
    subject: `Sujet ${id}`,
    senders: [{ name: "Alice", address: "alice@example.com" }],
    messages: 1,
    unread: 0,
    flagged: false,
    has_attachments: false,
  };
}

function fakeApi(total: number): MailApi & { calls: ScopeDto[] } {
  let all = Array.from({ length: total }, (_, i) => conversation(i + 1));
  const calls: ScopeDto[] = [];
  let removed: ConversationDto[] = [];
  return {
    calls,
    performAction: vi.fn(async (ids: number[], _scope: ScopeDto, action: ActionDto) => {
      if (action.kind === "move" || action.kind === "delete" || action.kind === "archive") {
        removed = all.filter((c) => ids.includes(c.id));
        all = all.filter((c) => !ids.includes(c.id));
      }
      return { ops: ids.map((id) => id + 1000), messages: ids.length };
    }),
    undoActions: vi.fn(async (ops: number[]) => {
      all = [...all, ...removed].sort((a, b) => a.id - b.id);
      removed = [];
      return ops.length;
    }),
    listFolders: vi.fn(async () => []),
    syncAccount: vi.fn(async () => undefined),
    conversationMessages: vi.fn(async (id: number) => [
      {
        id: id * 10,
        account: 1,
        mailbox: 10,
        date: 0,
        from: null,
        to: [],
        cc: [],
        subject: `Sujet ${id}`,
        unread: false,
        flagged: false,
        has_attachments: false,
      },
    ]),
    listConversations: vi.fn(async (scope, cursor, limit) => {
      calls.push(scope);
      const start = cursor === null ? 0 : all.findIndex((c) => c.id === cursor.id) + 1;
      const items = all.slice(start, start + limit);
      const last = items.at(-1);
      const next =
        start + limit < all.length && last !== undefined ? { date: last.date, id: last.id } : null;
      return { items, next };
    }),
  };
}

describe("état de la lecture", () => {
  it("affiche la boîte unifiée, pagine et navigue avec j/k", async () => {
    const api = fakeApi(PAGE_SIZE + 30);
    const mail = new MailState(api);
    await mail.show(UNIFIED_INBOX);
    expect(api.calls[0]).toEqual(UNIFIED_INBOX);
    expect(mail.conversations).toHaveLength(PAGE_SIZE);
    expect(mail.hasMore).toBe(true);
    expect(mail.selectedIndex).toBe(0);

    await mail.next();
    expect(mail.selected?.id).toBe(2);
    await mail.previous();
    await mail.previous();
    expect(mail.selected?.id).toBe(1);

    // Approche de la fin : page suivante chargée.
    mail.select(PAGE_SIZE - 5);
    await mail.loadMore();
    expect(mail.conversations).toHaveLength(PAGE_SIZE + 30);
    expect(mail.hasMore).toBe(false);
  });

  it("ouvre, suit la navigation et revient à la liste", async () => {
    const mail = new MailState(fakeApi(5));
    await mail.show({ kind: "mailbox", id: 10 });
    await mail.open();
    expect(mail.opened?.id).toBe(1);
    expect(mail.messages.map((m) => m.id)).toEqual([10]);
    await mail.next();
    expect(mail.opened?.id).toBe(2);
    expect(mail.messages.map((m) => m.id)).toEqual([20]);
    mail.back();
    expect(mail.opened).toBeNull();
    expect(mail.selected?.id).toBe(2);
  });

  it("rafraîchit selon les événements de synchronisation", async () => {
    const api = fakeApi(3);
    const mail = new MailState(api);
    await mail.show({ kind: "mailbox", id: 10 });
    mail.select(2);
    await mail.onSyncEvent({ kind: "folder_changed", account: 1, mailbox: 99 });
    expect(api.calls).toHaveLength(1);
    await mail.onSyncEvent({ kind: "folder_changed", account: 1, mailbox: 10 });
    expect(api.calls).toHaveLength(2);
    expect(mail.selected?.id).toBe(3);
    await mail.onSyncEvent({ kind: "offline", account: 1, failure: "network", detail: null });
    expect(mail.offline[1]).toBe("serveur injoignable");
    await mail.onSyncEvent({
      kind: "offline",
      account: 1,
      failure: "authentication",
      detail: "NO [AUTHENTICATIONFAILED]",
    });
    expect(mail.offline[1]).toBe("identifiants refusés (« NO [AUTHENTICATIONFAILED] »)");
    await mail.onSyncEvent({ kind: "online", account: 1 });
    expect(mail.offline[1]).toBeUndefined();
  });

  it("actions : effet immédiat, sélection suivante, annulation", async () => {
    const api = fakeApi(5);
    const mail = new MailState(api);
    await mail.show(UNIFIED_INBOX);
    mail.select(1);
    await mail.act({ kind: "archive" }, "Archivé.");
    expect(api.performAction).toHaveBeenCalledWith([2], UNIFIED_INBOX, { kind: "archive" });
    expect(mail.conversations.map((c) => c.id)).toEqual([1, 3, 4, 5]);
    expect(mail.selected?.id).toBe(3);
    expect(mail.notice).toEqual({ text: "Archivé.", undoable: true });
    expect(mail.lastAction).toEqual([1002]);
    await mail.undo();
    expect(api.undoActions).toHaveBeenCalledWith([1002]);
    expect(mail.conversations.map((c) => c.id)).toEqual([1, 2, 3, 4, 5]);
    expect(mail.notice?.text).toBe("Action annulée.");
    expect(mail.lastAction).toBeNull();
  });

  it("sélection multiple avec x, puis action sur les conversations cochées", async () => {
    const api = fakeApi(5);
    const mail = new MailState(api);
    await mail.show(UNIFIED_INBOX);
    mail.select(0);
    mail.toggleMark();
    mail.select(3);
    mail.toggleMark();
    expect(mail.marked).toEqual([1, 4]);
    mail.toggleMark();
    expect(mail.marked).toEqual([1]);
    mail.toggleMark();
    await mail.act({ kind: "mark_seen", value: true }, "Lu.");
    expect(api.performAction).toHaveBeenCalledWith([1, 4], UNIFIED_INBOX, {
      kind: "mark_seen",
      value: true,
    });
    expect(mail.marked).toEqual([]);
  });

  it("annulation trop tardive et erreur d'action", async () => {
    const api = fakeApi(3);
    const mail = new MailState(api);
    await mail.show(UNIFIED_INBOX);
    mail.select(0);
    await mail.act({ kind: "delete" }, "Supprimé.");
    vi.mocked(api.undoActions).mockResolvedValueOnce(0);
    await mail.undo();
    expect(mail.notice?.text).toMatch(/Trop tard/);
    vi.mocked(api.performAction).mockRejectedValueOnce(new IpcFailure("no_trash"));
    await mail.act({ kind: "delete" }, "Supprimé.");
    expect(mail.notice).toEqual({
      text: "Aucun dossier Corbeille sur ce compte.",
      undoable: false,
    });
  });

  it("« annuler » n'est plus proposé après la fenêtre d'annulation", async () => {
    vi.useFakeTimers();
    try {
      const mail = new MailState(fakeApi(3));
      await mail.show(UNIFIED_INBOX);
      mail.select(0);
      await mail.act({ kind: "archive" }, "Archivé.");
      expect(mail.lastAction).not.toBeNull();
      vi.advanceTimersByTime(UNDO_WINDOW_MS);
      expect(mail.lastAction).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("conversation ouverte depuis la recherche", () => {
  it("agit dans le dossier du résultat, puis revient à la portée affichée", async () => {
    const api = fakeApi(3);
    const mail = new MailState(api);
    await mail.show(UNIFIED_INBOX);
    const acted = vi.fn();
    mail.onActed = acted;
    const found = { ...conversation(42), mailbox: 77 };
    await mail.openConversation(found, { kind: "mailbox", id: 77 });
    expect(mail.opened?.id).toBe(42);
    expect(mail.messages.map((x) => x.id)).toEqual([420]);
    expect(mail.targets().map((c) => c.id)).toEqual([42]);
    await mail.act({ kind: "mark_seen", value: true }, "lu");
    expect(api.performAction).toHaveBeenLastCalledWith(
      [42],
      { kind: "mailbox", id: 77 },
      { kind: "mark_seen", value: true },
    );
    expect(acted).toHaveBeenCalledTimes(1);
    mail.back();
    mail.select(0);
    await mail.act({ kind: "mark_seen", value: true }, "lu");
    expect(api.performAction).toHaveBeenLastCalledWith([1], UNIFIED_INBOX, {
      kind: "mark_seen",
      value: true,
    });
  });

  it("ignore les conversations cochées dans la liste", async () => {
    const api = fakeApi(3);
    const mail = new MailState(api);
    await mail.show(UNIFIED_INBOX);
    mail.toggleMark();
    await mail.openConversation(conversation(9), { kind: "mailbox", id: 10 });
    expect(mail.marked).toEqual([]);
    expect(mail.targets().map((c) => c.id)).toEqual([9]);
  });
});
