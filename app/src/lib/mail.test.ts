import { describe, expect, it, vi } from "vitest";
import type { ActionDto, ConversationDto, ConversationMessage, ScopeDto } from "./ipc/api";
import { MailState, PAGE_SIZE, UNDO_WINDOW_MS, UNIFIED_INBOX, type MailApi } from "./mail.svelte";
import { IpcFailure } from "./errors";

/** Sans marquage comme lu à l'ouverture (réglages non chargés). */
const noAutoSeen = (): null => null;

function conversation(id: number, account = 1): ConversationDto {
  return {
    id,
    account,
    mailbox: 10,
    date: 1_700_000_000 - id,
    subject: `Sujet ${id}`,
    senders: [{ name: "Alice", address: "alice@example.com", contact: null }],
    messages: 1,
    unread: 0,
    flagged: false,
    has_attachments: false,
  };
}

function fakeApi(total: number, unread: number[] = []): MailApi & { calls: ScopeDto[] } {
  let all = Array.from({ length: total }, (_, i) => ({
    ...conversation(i + 1),
    unread: unread.includes(i + 1) ? 1 : 0,
  }));
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
    markOpenedSeen: vi.fn(async (_conversation: number, shown: number[]) => ({
      ops: shown.map((id) => id + 2000),
      messages: shown.length,
    })),
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
        unread: unread.includes(id),
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
    const mail = new MailState(api, noAutoSeen);
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
    const mail = new MailState(fakeApi(5), noAutoSeen);
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
    const mail = new MailState(api, noAutoSeen);
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
    const mail = new MailState(api, noAutoSeen);
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

  it("action sur des conversations désignées (glisser-déposer), pas sur la sélection", async () => {
    const api = fakeApi(5);
    const mail = new MailState(api, noAutoSeen);
    await mail.show(UNIFIED_INBOX);
    mail.select(0);
    await mail.open();
    const opened = mail.opened?.id;
    expect(opened).toBe(1);
    mail.toggleMarkAt(1);
    await mail.act({ kind: "move", mailbox: 77 }, "Déplacé.", [3, 4]);
    expect(api.performAction).toHaveBeenCalledWith([3, 4], UNIFIED_INBOX, {
      kind: "move",
      mailbox: 77,
    });
    // La conversation ouverte n'était pas visée : elle reste ouverte.
    expect(mail.opened?.id).toBe(opened);
    expect(mail.marked).toEqual([]);
    expect(mail.notice).toEqual({ text: "Déplacé.", undoable: true });
  });

  it("sélection multiple avec x, puis action sur les conversations cochées", async () => {
    const api = fakeApi(5);
    const mail = new MailState(api, noAutoSeen);
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

  it("case à cocher d'une ligne : coche sans changer la sélection, puis tout décocher", async () => {
    const api = fakeApi(5);
    const mail = new MailState(api, noAutoSeen);
    await mail.show(UNIFIED_INBOX);
    mail.select(1);
    mail.toggleMarkAt(0);
    mail.toggleMarkAt(3);
    expect(mail.marked).toEqual([1, 4]);
    expect(mail.selectedIndex).toBe(1);
    mail.toggleMarkAt(0);
    expect(mail.marked).toEqual([4]);
    // Ligne inexistante : rien.
    mail.toggleMarkAt(42);
    expect(mail.marked).toEqual([4]);
    mail.clearMarks();
    expect(mail.marked).toEqual([]);
  });

  it("annulation trop tardive et erreur d'action", async () => {
    const api = fakeApi(3);
    const mail = new MailState(api, noAutoSeen);
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
      const mail = new MailState(fakeApi(3), noAutoSeen);
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
    const mail = new MailState(api, noAutoSeen);
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
    const mail = new MailState(api, noAutoSeen);
    await mail.show(UNIFIED_INBOX);
    mail.toggleMark();
    await mail.openConversation(conversation(9), { kind: "mailbox", id: 10 });
    expect(mail.marked).toEqual([]);
    expect(mail.targets().map((c) => c.id)).toEqual([9]);
  });
});

describe("mesure de performance", () => {
  it("pose un repère à chaque liste affichée, et un premier repère au démarrage", async () => {
    performance.clearMarks();
    const mail = new MailState(fakeApi(3), noAutoSeen);
    await mail.show(UNIFIED_INBOX);
    expect(performance.getEntriesByName("norelly:first-list")).toHaveLength(1);
    expect(performance.getEntriesByName("norelly:list-shown")).toHaveLength(1);
    await mail.show({ kind: "mailbox", id: 10 });
    expect(performance.getEntriesByName("norelly:first-list")).toHaveLength(1);
    expect(performance.getEntriesByName("norelly:list-shown")).toHaveLength(2);
  });
});

describe("marquage comme lu à l'ouverture", () => {
  const DELAY_MS = 5000;

  function seenCalls(api: MailApi): unknown[][] {
    return vi.mocked(api.markOpenedSeen).mock.calls;
  }

  function message(id: number, unread: boolean): ConversationMessage {
    return {
      id,
      account: 1,
      mailbox: 10,
      date: 0,
      from: null,
      to: [],
      cc: [],
      subject: "Sujet",
      unread,
      flagged: false,
      has_attachments: false,
    };
  }

  it("marque lus les messages affichés non lus restés ouverts, sans message ni annulation", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi(3, [1]);
      const mail = new MailState(api, () => DELAY_MS);
      await mail.show({ kind: "mailbox", id: 10 });
      await mail.open();
      await vi.advanceTimersByTimeAsync(DELAY_MS - 1);
      expect(seenCalls(api)).toEqual([]);
      await vi.advanceTimersByTimeAsync(1);
      expect(seenCalls(api)).toEqual([[1, [10]]]);
      expect(api.performAction).not.toHaveBeenCalled();
      expect(mail.notice).toBeNull();
      expect(mail.lastAction).toBeNull();
      expect(mail.opened?.id).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("une réponse arrivée pendant le délai n'est pas marquée", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi(3, [1]);
      const mail = new MailState(api, () => DELAY_MS);
      await mail.show(UNIFIED_INBOX);
      vi.mocked(api.conversationMessages).mockResolvedValueOnce([
        message(10, true),
        message(9, false),
      ]);
      await mail.open();
      // Synchronisation : la conversation relue a une réponse non lue.
      vi.mocked(api.conversationMessages).mockResolvedValueOnce([
        message(11, true),
        message(10, true),
        message(9, false),
      ]);
      await mail.onSyncEvent({ kind: "folder_changed", account: 1, mailbox: 10 });
      expect(mail.messages.map((x) => x.id)).toEqual([11, 10, 9]);
      await vi.advanceTimersByTimeAsync(DELAY_MS);
      expect(seenCalls(api)).toEqual([[1, [10]]]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("ne marque pas une conversation quittée avant le délai", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi(3, [1, 2]);
      const mail = new MailState(api, () => DELAY_MS);
      await mail.show(UNIFIED_INBOX);
      await mail.open();
      await vi.advanceTimersByTimeAsync(DELAY_MS / 2);
      await mail.next();
      await vi.advanceTimersByTimeAsync(DELAY_MS / 2);
      mail.back();
      await vi.advanceTimersByTimeAsync(DELAY_MS);
      expect(seenCalls(api)).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("une ouverture dépassée pendant le chargement ne programme rien", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi(3, [1, 2]);
      let release: () => void = () => undefined;
      vi.mocked(api.conversationMessages).mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            release = () => resolve([message(10, true)]);
          }),
      );
      const mail = new MailState(api, () => DELAY_MS);
      await mail.show(UNIFIED_INBOX);
      // Conversation 1 ouverte, puis 2 avant que les messages de 1 arrivent.
      const first = mail.open();
      mail.select(1);
      await mail.open();
      release();
      await first;
      await vi.advanceTimersByTimeAsync(DELAY_MS);
      expect(seenCalls(api)).toEqual([[2, [20]]]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("ne fait rien pour une conversation déjà lue", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi(3);
      const mail = new MailState(api, () => DELAY_MS);
      await mail.show(UNIFIED_INBOX);
      await mail.open();
      await vi.advanceTimersByTimeAsync(DELAY_MS);
      expect(seenCalls(api)).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("une action de l'utilisateur sur la conversation ouverte annule le marquage", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi(3, [1]);
      const mail = new MailState(api, () => DELAY_MS);
      await mail.show(UNIFIED_INBOX);
      await mail.open();
      await mail.act({ kind: "mark_flagged", value: true }, "Suivi.");
      await vi.advanceTimersByTimeAsync(DELAY_MS);
      expect(seenCalls(api)).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("une action de l'utilisateur attend le marquage en cours d'envoi", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi(3, [1]);
      let finish: () => void = () => undefined;
      vi.mocked(api.markOpenedSeen).mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = () => resolve({ ops: [2010], messages: 1 });
          }),
      );
      const mail = new MailState(api, () => DELAY_MS);
      await mail.show(UNIFIED_INBOX);
      await mail.open();
      await vi.advanceTimersByTimeAsync(DELAY_MS);
      expect(seenCalls(api)).toHaveLength(1);

      const unseen = mail.act({ kind: "mark_seen", value: false }, "Non lu.");
      await vi.advanceTimersByTimeAsync(0);
      expect(api.performAction).not.toHaveBeenCalled();
      finish();
      await unseen;
      expect(api.performAction).toHaveBeenCalledWith([1], UNIFIED_INBOX, {
        kind: "mark_seen",
        value: false,
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("conversation ouverte depuis la recherche", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi(3, [42]);
      const mail = new MailState(api, () => DELAY_MS);
      await mail.show(UNIFIED_INBOX);
      await mail.openConversation({ ...conversation(42), unread: 1 }, { kind: "mailbox", id: 77 });
      await vi.advanceTimersByTimeAsync(DELAY_MS);
      expect(seenCalls(api)).toEqual([[42, [420]]]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("un échec du marquage est signalé", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi(3, [1]);
      const mail = new MailState(api, () => DELAY_MS);
      await mail.show(UNIFIED_INBOX);
      await mail.open();
      vi.mocked(api.markOpenedSeen).mockRejectedValueOnce(new IpcFailure("too_many_messages"));
      await vi.advanceTimersByTimeAsync(DELAY_MS);
      expect(mail.notice?.undoable).toBe(false);
      expect(mail.notice?.text).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });

  it("suit le délai des réglages, relu à chaque ouverture", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi(3, [1]);
      let delay: number | null = 12_000;
      const mail = new MailState(api, () => delay);
      await mail.show(UNIFIED_INBOX);
      await mail.open();
      await vi.advanceTimersByTimeAsync(11_999);
      expect(seenCalls(api)).toEqual([]);
      await vi.advanceTimersByTimeAsync(1);
      expect(seenCalls(api)).toHaveLength(1);

      delay = 0;
      mail.back();
      await mail.open();
      await vi.advanceTimersByTimeAsync(0);
      expect(seenCalls(api)).toHaveLength(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("réglages pas encore chargés : rien n'est marqué", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi(3, [1]);
      const mail = new MailState(api, () => null);
      await mail.show(UNIFIED_INBOX);
      await mail.open();
      await vi.advanceTimersByTimeAsync(60_000);
      expect(seenCalls(api)).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });
});
