// État de la recherche (runes Svelte 5) : requête, résultats paginés,
// sélection. La requête est analysée, exécutée et surlignée en Rust (index
// local, hors ligne) ; aucune décision ici.

import type { Api, ConversationDto, SearchHit } from "./ipc/api";
import { errorText } from "./errors";
import { localUtcOffset } from "./format";

/** Résultats demandés par page. */
export const SEARCH_PAGE = 50;
/** Attente après la dernière frappe avant de chercher. */
export const SEARCH_DELAY_MS = 150;

export type SearchApi = Pick<Api, "searchMessages">;

export class SearchState {
  readonly #api: SearchApi;
  readonly #offset: () => number;
  active = $state(false);
  query = $state("");
  hits = $state<SearchHit[]>([]);
  hasMore = $state(false);
  error = $state<string | null>(null);
  selectedIndex = $state(-1);
  /** La requête affichée a été exécutée (résultats à jour). */
  searched = $state(false);
  /** Incrémenté pour demander le focus de la barre. */
  focusRequest = $state(0);

  readonly selected = $derived(this.hits[this.selectedIndex] ?? null);

  constructor(api: SearchApi, utcOffset: () => number = localUtcOffset) {
    this.#api = api;
    this.#offset = utcOffset;
  }

  #next: number | null = null;
  #generation = 0;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #loading: Promise<void> | null = null;

  /** Affiche la barre et lui donne le focus. */
  open(): void {
    this.active = true;
    this.focusRequest += 1;
  }

  /** Referme la recherche : requête et résultats effacés. */
  close(): void {
    this.#cancelTimer();
    this.#generation += 1;
    this.active = false;
    this.query = "";
    this.#clear();
  }

  /** Nouvelle saisie : recherche après une courte pause de frappe. */
  setQuery(text: string): void {
    this.query = text;
    this.searched = false;
    this.#cancelTimer();
    this.#timer = setTimeout(() => {
      this.#timer = null;
      void this.submit();
    }, SEARCH_DELAY_MS);
  }

  /** Cherche tout de suite (Entrée) : première page, premier résultat
   * sélectionné. */
  submit(): Promise<void> {
    this.#cancelTimer();
    return this.#run(0);
  }

  /** Relance la requête affichée (après une action ou une
   * synchronisation), en gardant la position. */
  async refresh(): Promise<void> {
    if (!this.active || this.query.trim() === "") return;
    await this.#run(this.selectedIndex);
  }

  /** Page suivante. */
  loadMore(): Promise<void> {
    const offset = this.#next;
    if (offset === null) return Promise.resolve();
    this.#loading ??= (async () => {
      const generation = this.#generation;
      try {
        const page = await this.#api.searchMessages(
          this.query,
          { kind: "all" },
          this.#offset(),
          offset,
          SEARCH_PAGE,
        );
        if (generation !== this.#generation) return;
        this.hits = [...this.hits, ...page.items];
        this.#next = page.next;
        this.hasMore = page.next !== null;
      } catch (e) {
        if (generation === this.#generation) this.error = errorText(e);
      } finally {
        this.#loading = null;
      }
    })();
    return this.#loading;
  }

  select(index: number): void {
    if (index < 0 || index >= this.hits.length) return;
    this.selectedIndex = index;
    if (index >= this.hits.length - 10) void this.loadMore();
  }

  next(): void {
    this.select(Math.min(this.selectedIndex + 1, this.hits.length - 1));
  }

  previous(): void {
    this.select(Math.max(this.selectedIndex - 1, 0));
  }

  async #run(keepIndex: number): Promise<void> {
    const generation = ++this.#generation;
    const query = this.query;
    if (query.trim() === "") {
      this.#clear();
      return;
    }
    try {
      const page = await this.#api.searchMessages(
        query,
        { kind: "all" },
        this.#offset(),
        0,
        SEARCH_PAGE,
      );
      if (generation !== this.#generation) return;
      this.hits = page.items;
      this.#next = page.next;
      this.hasMore = page.next !== null;
      this.error = null;
      this.searched = true;
      this.selectedIndex =
        page.items.length === 0 ? -1 : Math.min(Math.max(keepIndex, 0), page.items.length - 1);
    } catch (e) {
      if (generation !== this.#generation) return;
      this.#clear();
      this.error = errorText(e);
    }
  }

  #clear(): void {
    this.searched = false;
    this.hits = [];
    this.#next = null;
    this.hasMore = false;
    this.error = null;
    this.selectedIndex = -1;
  }

  #cancelTimer(): void {
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
  }

  /** Conversation d'un résultat, à ouvrir ; `null` si le message n'est
   * rattaché à aucune conversation. */
  conversationOf(hit: SearchHit): ConversationDto | null {
    if (hit.conversation === null) return null;
    return {
      id: hit.conversation,
      account: hit.account,
      mailbox: hit.mailbox,
      date: hit.date,
      subject: hit.subject.map((s) => s.text).join(""),
      senders: hit.from === null ? [] : [hit.from],
      messages: 1,
      unread: hit.unread ? 1 : 0,
      flagged: hit.flagged,
      has_attachments: hit.has_attachments,
    };
  }
}
