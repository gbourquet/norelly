import { describe, expect, it, vi } from "vitest";
import type { ConversationDto, ScopeDto } from "./ipc/api";
import { MailState, PAGE_SIZE, UNIFIED_INBOX, type MailApi } from "./mail.svelte";

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
  const all = Array.from({ length: total }, (_, i) => conversation(i + 1));
  const calls: ScopeDto[] = [];
  return {
    calls,
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
});
