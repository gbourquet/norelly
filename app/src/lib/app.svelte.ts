// État de l'interface (runes Svelte 5) et registre de commandes de
// l'application.

import { CommandRegistry } from "./commands/registry";
import {
  api,
  type AccountDto,
  type ComposeKindDto,
  type DraftsDto,
  type OutboxItemDto,
} from "./ipc/api";
import { ComposeState } from "./compose.svelte";
import { MailState } from "./mail.svelte";
import { SearchState } from "./search.svelte";
import { SettingsState } from "./settings.svelte";
import { UpdatesState } from "./updates.svelte";
import type { ConversationMessage } from "./ipc/api";
import { failureLabel } from "./outbox";
import { m } from "./paraglide/messages.js";
import { errorText } from "./errors";

/** Mots-clés de recherche d'un message « a b c ». */
const words = (s: string): string[] => s.split(" ").filter(Boolean);

export type DialogName =
  | "palette"
  | "help"
  | "add-account"
  | "remove-account"
  | "move"
  | "archive-folder"
  | "compose"
  | "outbox"
  | "settings"
  | "drafts";

export interface AppContext {
  selectedAccount: AccountDto | null;
  dialog: DialogName | null;
  /** Nombre de conversations affichées. */
  conversations: number;
  /** Une conversation est ouverte (vue de lecture). */
  reading: boolean;
  /** Une action peut viser au moins une conversation. */
  hasTargets: boolean;
  /** Nombre de conversations cochées. */
  marked: number;
  /** La dernière action peut être annulée. */
  canUndo: boolean;
  /** La recherche est affichée à la place de la liste. */
  searching: boolean;
  /** Nombre de résultats de recherche affichés. */
  results: number;
}

class AppState {
  accounts = $state<AccountDto[]>([]);
  selectedAccountId = $state<number | null>(null);
  dialog = $state<DialogName | null>(null);
  loadError = $state<string | null>(null);
  /** Nature de la rédaction ouverte. */
  composeKind = $state<ComposeKindDto>("new");
  /** File d'envoi du compte affiché dans la boîte d'envoi. */
  outbox = $state<OutboxItemDto[]>([]);
  outboxAccount = $state<number | null>(null);
  /** Brouillons du compte sélectionné (liste des brouillons). */
  drafts = $state<DraftsDto>({ local: [], remote: [] });
  /** Compte de la liste des brouillons affichée. */
  draftsAccount = $state<number | null>(null);
  /** Préférence « texte brut », affichée dans les paramètres. */
  plainText = $state(false);
  /** Échecs déjà signalés (identifiants dans la file). */
  #reported = new Set<number>();

  readonly selectedAccount = $derived(
    this.accounts.find((a) => a.id === this.selectedAccountId) ?? null,
  );

  context(): AppContext {
    return {
      selectedAccount: this.selectedAccount,
      dialog: this.dialog,
      conversations: mail.conversations.length,
      reading: mail.opened !== null,
      // Liste des résultats sans conversation ouverte : rien à viser (la
      // liste des conversations est masquée).
      hasTargets: search.active && mail.opened === null ? false : mail.targets().length > 0,
      marked: mail.marked.length,
      canUndo: mail.lastAction !== null,
      searching: search.active,
      results: search.hits.length,
    };
  }

  open(dialog: DialogName): void {
    this.dialog = dialog;
  }

  close(): void {
    this.dialog = null;
  }

  /** Ouvre l'éditeur : nouveau message, réponse ou transfert. */
  async compose(
    kind: ComposeKindDto,
    account: number,
    message: ConversationMessage | null,
  ): Promise<void> {
    this.composeKind = kind;
    this.dialog = "compose";
    await compose.open(account, kind, message);
  }

  /** Relit la file d'un compte ; signale une fois chaque nouvel échec. */
  async refreshOutbox(account: number): Promise<void> {
    try {
      const items = await api.outbox(account);
      if (this.outboxAccount === account) this.outbox = items;
      for (const item of items) {
        if (item.state.state === "failed" && !this.#reported.has(item.id)) {
          this.#reported.add(item.id);
          mail.notice = {
            text: m.compose_send_failed({
              subject: item.subject || m.outbox_no_subject(),
              reason: failureLabel(item.state.failure),
            }),
            undoable: false,
          };
        }
      }
    } catch (e) {
      mail.notice = { text: errorText(e), undoable: false };
    }
  }

  /** Liste des brouillons d'un compte. */
  async showDrafts(account: number): Promise<void> {
    try {
      this.drafts = await api.listDrafts(account);
      this.draftsAccount = account;
      this.dialog = "drafts";
    } catch (e) {
      mail.notice = { text: errorText(e), undoable: false };
    }
  }

  /** Rouvre un brouillon enregistré dans l'éditeur. */
  async openDraft(id: number): Promise<void> {
    try {
      const draft = await api.openDraft(id);
      this.composeKind = "new";
      compose.resume(draft);
      this.dialog = "compose";
    } catch (e) {
      mail.notice = { text: errorText(e), undoable: false };
    }
  }

  /** Reprend un brouillon du dossier « Brouillons » écrit ailleurs. */
  async resumeRemoteDraft(account: number, message: number): Promise<void> {
    try {
      const draft = await api.prepareCompose(account, "resume", message, "");
      this.composeKind = "new";
      compose.resume(draft);
      this.dialog = "compose";
    } catch (e) {
      mail.notice = { text: errorText(e), undoable: false };
    }
  }

  /** Reprend un envoi programmé pour le modifier. */
  async editScheduled(account: number, id: number): Promise<void> {
    try {
      const draft = await api.unscheduleSend(account, id);
      this.composeKind = "new";
      compose.resume(draft);
      this.dialog = "compose";
    } catch (e) {
      mail.notice = { text: errorText(e), undoable: false };
    }
  }

  async showOutbox(account: number): Promise<void> {
    this.outboxAccount = account;
    this.outbox = [];
    this.dialog = "outbox";
    await this.refreshOutbox(account);
  }

  async refreshAccounts(): Promise<void> {
    try {
      this.accounts = await api.listAccounts();
      performance.mark("norelly:accounts-listed");
      this.loadError = null;
      if (
        this.selectedAccountId !== null &&
        !this.accounts.some((a) => a.id === this.selectedAccountId)
      ) {
        this.selectedAccountId = null;
      }
      if (this.selectedAccountId === null) {
        this.selectedAccountId = this.accounts[0]?.id ?? null;
      }
      await Promise.all(this.accounts.map((a) => mail.loadFolders(a.id)));
    } catch (e) {
      this.loadError = errorText(e);
    }
  }
}

export const mail = new MailState(api);

export const search = new SearchState(api);

export const settings = new SettingsState(api);

export const updates = new UpdatesState(api);

// Une action peut retirer un résultat (suppression, déplacement…).
mail.onActed = () => void search.refresh();

/** Ouvre la conversation du résultat sélectionné, dans son dossier. */
export async function openSelectedHit(): Promise<void> {
  const hit = search.selected;
  if (hit === null) return;
  const conversation = search.conversationOf(hit);
  if (conversation === null) return;
  await mail.openConversation(conversation, { kind: "mailbox", id: hit.mailbox });
}

/** Résultat suivant ou précédent ; ouvert si la lecture est affichée. */
async function stepHit(forward: boolean): Promise<void> {
  if (forward) search.next();
  else search.previous();
  if (mail.opened !== null) await openSelectedHit();
}

export const compose = new ComposeState(api);

export const app = new AppState();

export const registry = new CommandRegistry<AppContext>();

/** Compte des conversations visées par « déplacer » ; `null` s'il y en a
 * plusieurs (un dossier n'appartient qu'à un compte) ou aucune. */
export function moveAccount(): number | null {
  const all = mail.targets().map((c) => c.account);
  const accounts = all.filter((a, i) => all.indexOf(a) === i);
  return accounts.length === 1 ? (accounts[0] ?? null) : null;
}

/** Commandes de base de l'application. Renvoie la fonction de retrait. */
export function registerAppCommands(): () => void {
  const offs = [
    registry.register({
      id: "palette.open",
      title: m.cmd_palette(),
      group: m.group_general(),
      shortcut: "Ctrl+K",
      inPalette: false,
      run: () => app.open("palette"),
    }),
    registry.register({
      id: "help.shortcuts",
      title: m.cmd_help(),
      group: m.group_general(),
      shortcut: "?",
      keywords: words(m.cmd_help_keywords()),
      run: () => app.open("help"),
    }),
    registry.register({
      id: "settings.open",
      title: m.cmd_settings(),
      group: m.group_general(),
      shortcut: "Ctrl+,",
      keywords: words(m.cmd_settings_keywords()),
      run: async () => {
        const prefs = await api.readingPreferences().catch(() => null);
        app.plainText = prefs?.prefer_plain_text ?? false;
        app.open("settings");
      },
    }),
    registry.register({
      id: "account.add",
      title: m.cmd_account_add(),
      group: m.group_accounts(),
      keywords: words(m.cmd_account_add_keywords()),
      run: () => app.open("add-account"),
    }),
    registry.register({
      id: "conversation.next",
      title: m.cmd_next(),
      group: m.group_reading(),
      shortcut: "j",
      when: (ctx) => (ctx.searching ? ctx.results > 0 : ctx.conversations > 0),
      run: () => (search.active ? stepHit(true) : mail.next()),
    }),
    registry.register({
      id: "conversation.previous",
      title: m.cmd_previous(),
      group: m.group_reading(),
      shortcut: "k",
      when: (ctx) => (ctx.searching ? ctx.results > 0 : ctx.conversations > 0),
      run: () => (search.active ? stepHit(false) : mail.previous()),
    }),
    registry.register({
      id: "conversation.open",
      title: m.cmd_open(),
      group: m.group_reading(),
      shortcut: "Enter",
      when: (ctx) => !ctx.reading && (ctx.searching ? ctx.results > 0 : ctx.conversations > 0),
      run: () => (search.active ? openSelectedHit() : mail.open()),
    }),
    registry.register({
      id: "conversation.back",
      title: m.cmd_back(),
      group: m.group_reading(),
      shortcut: "Escape",
      when: (ctx) => ctx.reading || ctx.searching,
      run: (ctx) => (ctx.reading ? mail.back() : search.close()),
    }),
    registry.register({
      id: "search.open",
      title: m.cmd_search(),
      group: m.group_reading(),
      shortcut: "/",
      keywords: words(m.cmd_search_keywords()),
      run: () => search.open(),
    }),
    registry.register({
      id: "mailbox.unified",
      title: m.cmd_unified(),
      group: m.group_reading(),
      keywords: words(m.cmd_unified_keywords()),
      run: () => mail.show({ kind: "unified", role: "inbox" }),
    }),
    registry.register({
      id: "reading.plain-text",
      title: m.cmd_plain_text(),
      group: m.group_reading(),
      keywords: words(m.cmd_plain_text_keywords()),
      run: async () => {
        const prefs = await api.readingPreferences();
        await api.setPreferPlainText(!prefs.prefer_plain_text);
        if (mail.opened !== null) await mail.open();
      },
    }),
    registry.register({
      id: "action.archive",
      title: m.cmd_archive(),
      group: m.group_actions(),
      shortcut: "e",
      keywords: words(m.cmd_archive_keywords()),
      when: (ctx) => ctx.hasTargets,
      run: () => mail.act({ kind: "archive" }, m.action_archived()),
    }),
    registry.register({
      id: "action.delete",
      title: m.cmd_delete(),
      group: m.group_actions(),
      shortcut: "#",
      keywords: words(m.cmd_delete_keywords()),
      when: (ctx) => ctx.hasTargets,
      run: () => mail.act({ kind: "delete" }, m.action_deleted()),
    }),
    registry.register({
      id: "action.flag",
      title: m.cmd_flag(),
      group: m.group_actions(),
      shortcut: "s",
      keywords: words(m.cmd_flag_keywords()),
      when: (ctx) => ctx.hasTargets,
      run: () => {
        // Tout est déjà suivi : on retire le suivi ; sinon on l'ajoute.
        const value = !mail.targets().every((c) => c.flagged);
        return mail.act(
          { kind: "mark_flagged", value },
          value ? m.action_flagged() : m.action_unflagged(),
        );
      },
    }),
    registry.register({
      id: "action.mark-read",
      title: m.cmd_mark_read(),
      group: m.group_actions(),
      shortcut: "Shift+I",
      keywords: words(m.cmd_mark_read_keywords()),
      when: (ctx) => ctx.hasTargets,
      run: () => mail.act({ kind: "mark_seen", value: true }, m.action_marked_read()),
    }),
    registry.register({
      id: "action.mark-unread",
      title: m.cmd_mark_unread(),
      group: m.group_actions(),
      shortcut: "Shift+U",
      keywords: words(m.cmd_mark_unread_keywords()),
      when: (ctx) => ctx.hasTargets,
      run: () => mail.act({ kind: "mark_seen", value: false }, m.action_marked_unread()),
    }),
    registry.register({
      id: "action.move",
      title: m.cmd_move(),
      group: m.group_actions(),
      shortcut: "v",
      keywords: words(m.cmd_move_keywords()),
      when: (ctx) => ctx.hasTargets,
      run: () => {
        if (moveAccount() === null) {
          mail.notice = { text: m.action_one_account(), undoable: false };
          return;
        }
        app.open("move");
      },
    }),
    registry.register({
      id: "action.toggle-mark",
      title: m.cmd_toggle_mark(),
      group: m.group_actions(),
      shortcut: "x",
      keywords: words(m.cmd_toggle_mark_keywords()),
      when: (ctx) => ctx.conversations > 0 && !ctx.reading,
      run: () => mail.toggleMark(),
    }),
    registry.register({
      id: "selection.clear",
      title: m.cmd_clear_selection(),
      group: m.group_actions(),
      keywords: words(m.cmd_clear_selection_keywords()),
      when: (ctx) => ctx.marked > 0,
      run: () => mail.clearMarks(),
    }),
    registry.register({
      id: "action.undo",
      title: m.cmd_undo(),
      group: m.group_actions(),
      shortcut: "z",
      keywords: words(m.cmd_undo_keywords()),
      when: (ctx) => ctx.canUndo,
      run: () => mail.undo(),
    }),
    registry.register({
      id: "compose.new",
      title: m.cmd_compose(),
      group: m.group_compose(),
      shortcut: "c",
      keywords: words(m.cmd_compose_keywords()),
      when: (ctx) => ctx.selectedAccount !== null,
      run: (ctx) =>
        ctx.selectedAccount ? app.compose("new", ctx.selectedAccount.id, null) : undefined,
    }),
    ...(
      [
        ["compose.reply", m.cmd_reply(), "r", "reply"],
        ["compose.reply-all", m.cmd_reply_all(), "a", "reply_all"],
        ["compose.forward", m.cmd_forward(), "f", "forward"],
      ] as const
    ).map(([id, title, shortcut, kind]) =>
      registry.register({
        id,
        title,
        group: m.group_compose(),
        shortcut,
        // Conversation ouverte, ou une seule visée dans la liste (menu
        // contextuel, raccourci) : elle est ouverte d'abord.
        when: (ctx) =>
          ctx.reading ? mail.messages.length > 0 : ctx.hasTargets && ctx.marked === 0,
        run: async () => {
          if (mail.opened === null) await mail.open();
          const message = mail.messages.at(-1);
          return message ? app.compose(kind, message.account, message) : undefined;
        },
      }),
    ),
    registry.register({
      id: "drafts.open",
      title: m.cmd_drafts(),
      group: m.group_compose(),
      keywords: words(m.cmd_drafts_keywords()),
      when: (ctx) => ctx.selectedAccount !== null,
      run: (ctx) => (ctx.selectedAccount ? app.showDrafts(ctx.selectedAccount.id) : undefined),
    }),
    registry.register({
      id: "compose.outbox",
      title: m.cmd_outbox(),
      group: m.group_compose(),
      keywords: words(m.cmd_outbox_keywords()),
      when: (ctx) => ctx.selectedAccount !== null,
      run: (ctx) => (ctx.selectedAccount ? app.showOutbox(ctx.selectedAccount.id) : undefined),
    }),
    registry.register({
      id: "account.archive-folder",
      title: m.cmd_archive_folder(),
      group: m.group_accounts(),
      keywords: words(m.cmd_archive_folder_keywords()),
      when: (ctx) => ctx.selectedAccount !== null,
      run: () => app.open("archive-folder"),
    }),
    registry.register({
      id: "account.sync",
      title: m.cmd_sync(),
      group: m.group_accounts(),
      keywords: words(m.cmd_sync_keywords()),
      when: (ctx) => ctx.selectedAccount !== null,
      run: (ctx) => (ctx.selectedAccount ? api.syncAccount(ctx.selectedAccount.id) : undefined),
    }),
    registry.register({
      id: "account.remove",
      title: m.cmd_account_remove(),
      group: m.group_accounts(),
      keywords: words(m.cmd_account_remove_keywords()),
      when: (ctx) => ctx.selectedAccount !== null,
      run: () => app.open("remove-account"),
    }),
  ];
  return () => offs.forEach((off) => off());
}
