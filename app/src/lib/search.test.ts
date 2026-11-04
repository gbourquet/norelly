import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SearchHit, SearchPage } from "./ipc/api";
import { SEARCH_DELAY_MS, SEARCH_PAGE, SearchState, type SearchApi } from "./search.svelte";
import { IpcFailure } from "./errors";

function hit(n: number, conversation: number | null = n): SearchHit {
  return {
    message: n,
    conversation,
    account: 1,
    mailbox: 10,
    date: 1_700_000_000 - n,
    from: { name: "Alice", address: "alice@example.com" },
    subject: [
      { text: "Facture", hit: true },
      { text: ` ${n}`, hit: false },
    ],
    snippet: [{ text: "texte", hit: false }],
    unread: n % 2 === 0,
    flagged: n === 3,
    has_attachments: false,
  };
}

type Call = { query: string; offset: number; tz: number; limit: number };

function fakeApi(total: number) {
  const calls: Call[] = [];
  const api: SearchApi = {
    searchMessages: vi.fn(async (query, _scope, tz, offset, limit): Promise<SearchPage> => {
      calls.push({ query, offset, tz, limit });
      const all = Array.from({ length: total }, (_, i) => hit(i + 1));
      const items = all.slice(offset, offset + limit);
      return { items, next: offset + limit < total ? offset + limit : null };
    }),
  };
  return { api, calls };
}

describe("état de la recherche", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("s'ouvre en demandant le focus et se referme à zéro", async () => {
    const { api } = fakeApi(3);
    const search = new SearchState(api, () => 60);
    search.open();
    expect(search.active).toBe(true);
    expect(search.focusRequest).toBe(1);
    search.query = "facture";
    await search.submit();
    expect(search.hits).toHaveLength(3);
    search.close();
    expect(search.active).toBe(false);
    expect(search.query).toBe("");
    expect(search.hits).toEqual([]);
    expect(search.selectedIndex).toBe(-1);
  });

  it("cherche après une pause de frappe, avec la dernière requête et le fuseau", async () => {
    const { api, calls } = fakeApi(3);
    const search = new SearchState(api, () => 60);
    search.open();
    search.setQuery("fac");
    search.setQuery("factu");
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS - 1);
    expect(calls).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(calls).toEqual([{ query: "factu", offset: 0, tz: 60, limit: SEARCH_PAGE }]);
    expect(search.hits).toHaveLength(3);
    expect(search.selectedIndex).toBe(0);
  });

  it("une requête vide efface les résultats sans appel", async () => {
    const { api, calls } = fakeApi(3);
    const search = new SearchState(api);
    search.open();
    search.query = "facture";
    await search.submit();
    search.query = "   ";
    await search.submit();
    expect(calls).toHaveLength(1);
    expect(search.hits).toEqual([]);
    expect(search.selectedIndex).toBe(-1);
  });

  it("ignore une réponse arrivée après une requête plus récente", async () => {
    let release: (page: SearchPage) => void = () => undefined;
    const api: SearchApi = {
      searchMessages: vi.fn((query: string) =>
        query === "lent"
          ? new Promise<SearchPage>((resolve) => (release = resolve))
          : Promise.resolve({ items: [hit(7)], next: null }),
      ),
    };
    const search = new SearchState(api);
    search.open();
    search.query = "lent";
    const slow = search.submit();
    search.query = "rapide";
    await search.submit();
    release({ items: [hit(1), hit(2)], next: null });
    await slow;
    expect(search.hits.map((h) => h.message)).toEqual([7]);
  });

  it("pagine et navigue dans les résultats", async () => {
    const { api, calls } = fakeApi(SEARCH_PAGE + 5);
    const search = new SearchState(api);
    search.open();
    search.query = "facture";
    await search.submit();
    expect(search.hasMore).toBe(true);
    await search.loadMore();
    expect(calls.at(-1)?.offset).toBe(SEARCH_PAGE);
    expect(search.hits).toHaveLength(SEARCH_PAGE + 5);
    expect(search.hasMore).toBe(false);
    search.previous();
    expect(search.selectedIndex).toBe(0);
    search.next();
    expect(search.selected?.message).toBe(2);
    search.select(SEARCH_PAGE + 4);
    search.next();
    expect(search.selectedIndex).toBe(SEARCH_PAGE + 4);
    search.select(999);
    expect(search.selectedIndex).toBe(SEARCH_PAGE + 4);
  });

  it("relance la requête en gardant la position, seulement si active", async () => {
    const { api, calls } = fakeApi(5);
    const search = new SearchState(api);
    await search.refresh();
    expect(calls).toHaveLength(0);
    search.open();
    search.query = "facture";
    await search.submit();
    search.select(3);
    await search.refresh();
    expect(calls).toHaveLength(2);
    expect(search.selectedIndex).toBe(3);
  });

  it("affiche l'erreur et vide les résultats", async () => {
    const api: SearchApi = {
      searchMessages: vi.fn(async () => {
        throw new IpcFailure("invalid_input", "query");
      }),
    };
    const search = new SearchState(api);
    search.open();
    search.query = "x";
    await search.submit();
    expect(search.error).toContain("recherche");
    expect(search.hits).toEqual([]);
  });

  it("donne la conversation d'un résultat", () => {
    const search = new SearchState(fakeApi(0).api);
    expect(search.conversationOf(hit(4))).toEqual({
      id: 4,
      account: 1,
      mailbox: 10,
      date: 1_700_000_000 - 4,
      subject: "Facture 4",
      senders: [{ name: "Alice", address: "alice@example.com" }],
      messages: 1,
      unread: 1,
      flagged: false,
      has_attachments: false,
    });
    expect(search.conversationOf(hit(5, null))).toBeNull();
  });

  it("sait si la requête affichée a été exécutée", async () => {
    const { api } = fakeApi(0);
    const search = new SearchState(api);
    search.open();
    expect(search.searched).toBe(false);
    search.setQuery("zzz");
    expect(search.searched).toBe(false);
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
    expect(search.searched).toBe(true);
    search.setQuery("zzzz");
    // Requête modifiée, pas encore cherchée.
    expect(search.searched).toBe(false);
    await search.submit();
    expect(search.searched).toBe(true);
    search.close();
    expect(search.searched).toBe(false);
  });
});
