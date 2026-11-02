// État de la lecture (runes Svelte 5) : portée affichée (dossier ou boîte
// unifiée), conversations paginées, sélection, conversation ouverte. Les
// listes sont paginées côté Rust (ADR-001) ; aucune décision de sécurité
// ici.

import type {
  Api,
  ConversationCursorDto,
  ConversationDto,
  ConversationMessage,
  FolderDto,
  ScopeDto,
  SyncEventDto,
} from "./ipc/api";
import { errorText, failureText, withDetail } from "./errors";

/** Taille d'une page demandée au Rust. */
export const PAGE_SIZE = 100;

export const UNIFIED_INBOX: ScopeDto = { kind: "unified", role: "inbox" };

export type MailApi = Pick<
  Api,
  "listFolders" | "listConversations" | "conversationMessages" | "syncAccount"
>;

/** Copie d'un dictionnaire sans une clé. */
function without<T>(record: Record<number, T>, key: number): Record<number, T> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => Number(k) !== key));
}

function sameScope(a: ScopeDto, b: ScopeDto): boolean {
  return a.kind === "mailbox" && b.kind === "mailbox"
    ? a.id === b.id
    : a.kind === "unified" && b.kind === "unified" && a.role === b.role;
}

export class MailState {
  readonly #api: MailApi;
  /** Dossiers par compte. */
  folders = $state<Record<number, FolderDto[]>>({});
  /** Comptes hors ligne (raison). */
  offline = $state<Record<number, string>>({});
  scope = $state<ScopeDto>(UNIFIED_INBOX);
  conversations = $state<ConversationDto[]>([]);
  #next: ConversationCursorDto | null = null;
  #loading: Promise<void> | null = null;
  #generation = 0;
  hasMore = $state(false);
  error = $state<string | null>(null);
  selectedIndex = $state(-1);
  /** Conversation ouverte (vue de lecture), `null` : liste seule. */
  opened = $state<ConversationDto | null>(null);
  messages = $state<ConversationMessage[]>([]);

  readonly selected = $derived(this.conversations[this.selectedIndex] ?? null);

  constructor(api: MailApi) {
    this.#api = api;
  }

  async loadFolders(account: number): Promise<void> {
    try {
      this.folders[account] = await this.#api.listFolders(account);
    } catch (e) {
      this.error = errorText(e);
    }
  }

  forgetAccount(account: number): void {
    this.folders = without(this.folders, account);
    this.offline = without(this.offline, account);
    this.conversations = this.conversations.filter((c) => c.account !== account);
    if (this.opened?.account === account) this.back();
  }

  /** Affiche une portée : première page, sélection remise à zéro. */
  async show(scope: ScopeDto): Promise<void> {
    this.scope = scope;
    this.opened = null;
    this.messages = [];
    this.selectedIndex = -1;
    await this.reload();
  }

  /**
   * Recharge la liste depuis le début, au moins jusqu'à la longueur déjà
   * affichée ; la sélection suit la conversation sélectionnée.
   */
  async reload(): Promise<void> {
    const generation = ++this.#generation;
    const keep = this.selected?.id ?? null;
    const wanted = Math.max(this.conversations.length, PAGE_SIZE);
    const items: ConversationDto[] = [];
    let cursor: ConversationCursorDto | null = null;
    try {
      do {
        const page = await this.#api.listConversations(this.scope, cursor, PAGE_SIZE);
        if (generation !== this.#generation) return;
        items.push(...page.items);
        cursor = page.next;
      } while (cursor !== null && items.length < wanted);
      this.conversations = items;
      this.#next = cursor;
      this.hasMore = cursor !== null;
      this.error = null;
      const index = keep === null ? -1 : items.findIndex((c) => c.id === keep);
      this.selectedIndex = index >= 0 ? index : items.length > 0 ? 0 : -1;
      if (this.opened !== null) {
        const fresh = items.find((c) => c.id === this.opened?.id);
        if (fresh !== undefined) {
          this.opened = fresh;
          await this.#loadMessages(fresh.id);
        }
      }
    } catch (e) {
      if (generation === this.#generation) this.error = errorText(e);
    }
  }

  /** Page suivante (défilement). */
  loadMore(): Promise<void> {
    if (this.#next === null) return Promise.resolve();
    this.#loading ??= (async () => {
      const generation = this.#generation;
      try {
        const page = await this.#api.listConversations(this.scope, this.#next, PAGE_SIZE);
        if (generation !== this.#generation) return;
        const known: Record<number, true> = {};
        for (const c of this.conversations) known[c.id] = true;
        this.conversations = [...this.conversations, ...page.items.filter((c) => !known[c.id])];
        this.#next = page.next;
        this.hasMore = page.next !== null;
      } catch (e) {
        this.error = errorText(e);
      } finally {
        this.#loading = null;
      }
    })();
    return this.#loading;
  }

  select(index: number): void {
    if (index < 0 || index >= this.conversations.length) return;
    this.selectedIndex = index;
    if (index >= this.conversations.length - 10) void this.loadMore();
  }

  /** Conversation suivante (j) ; ouverte si la lecture est affichée. */
  async next(): Promise<void> {
    const target = Math.min(this.selectedIndex + 1, this.conversations.length - 1);
    if (target === this.selectedIndex || target < 0) return;
    this.select(target);
    if (this.opened !== null) await this.open();
  }

  /** Conversation précédente (k). */
  async previous(): Promise<void> {
    const target = Math.max(this.selectedIndex - 1, 0);
    if (target === this.selectedIndex || this.conversations.length === 0) return;
    this.select(target);
    if (this.opened !== null) await this.open();
  }

  /** Ouvre la conversation sélectionnée. */
  async open(): Promise<void> {
    const conversation = this.selected;
    if (conversation === null) return;
    this.opened = conversation;
    this.messages = [];
    await this.#loadMessages(conversation.id);
  }

  async #loadMessages(id: number): Promise<void> {
    try {
      const messages = await this.#api.conversationMessages(id);
      if (this.opened?.id === id) this.messages = messages;
    } catch (e) {
      this.error = errorText(e);
    }
  }

  /** Retour à la liste. */
  back(): void {
    this.opened = null;
    this.messages = [];
  }

  /** Événement de synchronisation : rafraîchit ce qui est concerné. */
  async onSyncEvent(event: SyncEventDto): Promise<void> {
    switch (event.kind) {
      case "folders_changed":
        await this.loadFolders(event.account);
        await this.reload();
        break;
      case "folder_changed": {
        await this.loadFolders(event.account);
        const concerned =
          this.scope.kind === "unified" ||
          sameScope(this.scope, { kind: "mailbox", id: event.mailbox });
        if (concerned) await this.reload();
        break;
      }
      case "online":
        this.offline = without(this.offline, event.account);
        break;
      case "offline":
        this.offline[event.account] = withDetail(failureText(event.failure), event.detail);
        break;
    }
  }
}
