import { describe, expect, it, vi } from "vitest";
import type { CardDavBookDto, CardDavStatus } from "./ipc/api";
import { IpcFailure } from "./errors";
import { CardDavState, type CardDavApi } from "./carddav.svelte";

const none: CardDavStatus = { account: null, last_sync: null, syncing: false };
const linked: CardDavStatus = {
  account: {
    server: "https://dav.example.com/",
    username: "alice",
    book_url: "https://dav.example.com/alice/contacts/",
    book_name: "Contacts",
  },
  last_sync: null,
  syncing: false,
};
const synced: CardDavStatus = {
  ...linked,
  last_sync: {
    at: 1_700_000_000,
    report: { pulled: 2, pushed: 1, skipped: 0 },
    failure: null,
  },
};
const books: CardDavBookDto[] = [
  { url: "https://dav.example.com/alice/contacts/", name: "Contacts" },
  { url: "https://dav.example.com/alice/travail/", name: "Travail" },
];

function fakeApi(start: CardDavStatus = none) {
  let status = start;
  const api = {
    carddavStatus: vi.fn(async () => status),
    findCarddavBooks: vi.fn(async (): Promise<CardDavBookDto[]> => books),
    connectCarddav: vi.fn(async () => {
      status = linked;
      return linked;
    }),
    syncCarddav: vi.fn(async () => {
      status = synced;
      return { pulled: 2, pushed: 1, skipped: 0 };
    }),
    disconnectCarddav: vi.fn(async () => {
      status = none;
      return null;
    }),
  } satisfies CardDavApi;
  return api;
}

describe("synchronisation CardDAV", () => {
  it("s'ouvre sur l'état du carnet distant, formulaire vide", async () => {
    const state = new CardDavState(fakeApi(linked));
    state.password = "reste d'avant";
    await state.open();
    expect(state.connected).toBe(true);
    expect(state.password).toBe("");
    expect(state.books).toBeNull();
    expect(state.error).toBeNull();
  });

  it("cherche les carnets puis propose le premier", async () => {
    const api = fakeApi();
    const state = new CardDavState(api);
    await state.open();
    expect(state.connected).toBe(false);
    expect(state.canSearch).toBe(false);
    state.server = "dav.example.com";
    state.username = "alice";
    state.password = "mot de passe";
    expect(state.canSearch).toBe(true);
    await state.search();
    expect(api.findCarddavBooks).toHaveBeenCalledWith("dav.example.com", "alice", "mot de passe");
    expect(state.books).toEqual(books);
    expect(state.book).toBe(books[0]?.url);
    // Adresse ou identifiant modifiés : la liste n'est plus valable.
    state.edited();
    expect(state.books).toBeNull();
    expect(state.book).toBeNull();
  });

  it("relie le carnet choisi, oublie le mot de passe et synchronise aussitôt", async () => {
    const api = fakeApi();
    const state = new CardDavState(api);
    await state.open();
    state.server = "dav.example.com";
    state.username = "alice";
    state.password = "mot de passe";
    await state.search();
    state.book = books[1]?.url ?? null;
    const report = await state.connect();
    expect(api.connectCarddav).toHaveBeenCalledWith(
      "dav.example.com",
      "alice",
      "mot de passe",
      books[1]?.url,
    );
    expect(state.password).toBe("");
    expect(api.syncCarddav).toHaveBeenCalledOnce();
    expect(report).toEqual({ pulled: 2, pushed: 1, skipped: 0 });
    expect(state.status).toEqual(synced);
    expect(state.busy).toBe(false);
  });

  it("montre l'erreur d'une recherche refusée sans perdre la saisie", async () => {
    const api = fakeApi();
    api.findCarddavBooks.mockRejectedValueOnce(new IpcFailure("card_dav_unauthorized"));
    const state = new CardDavState(api);
    await state.open();
    state.server = "dav.example.com";
    state.username = "alice";
    state.password = "mauvais";
    await state.search();
    expect(state.error).toBe("Identifiant ou mot de passe refusé par le serveur de contacts.");
    expect(state.books).toBeNull();
    expect(state.server).toBe("dav.example.com");
    expect(state.busy).toBe(false);
  });

  it("une synchronisation échouée relit l'état enregistré", async () => {
    const api = fakeApi(linked);
    api.syncCarddav.mockRejectedValueOnce(new IpcFailure("card_dav_unreachable"));
    const state = new CardDavState(api);
    await state.open();
    expect(await state.sync()).toBeNull();
    expect(state.error).not.toBeNull();
    expect(api.carddavStatus).toHaveBeenCalledTimes(2);
  });

  it("déconnecte le carnet distant", async () => {
    const api = fakeApi(linked);
    const state = new CardDavState(api);
    await state.open();
    await state.disconnect();
    expect(api.disconnectCarddav).toHaveBeenCalledOnce();
    expect(state.connected).toBe(false);
  });

  it("fermée : le mot de passe saisi est oublié", async () => {
    const state = new CardDavState(fakeApi());
    await state.open();
    state.password = "secret";
    state.close();
    expect(state.password).toBe("");
  });
});
