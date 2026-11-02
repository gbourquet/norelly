// État de l'interface (runes Svelte 5) et registre de commandes de
// l'application.

import { CommandRegistry } from "./commands/registry";
import { api, type AccountDto } from "./ipc/api";
import { MailState } from "./mail.svelte";
import { m } from "./paraglide/messages.js";
import { errorText } from "./errors";

/** Mots-clés de recherche d'un message « a b c ». */
const words = (s: string): string[] => s.split(" ").filter(Boolean);

export type DialogName =
  "palette" | "help" | "add-account" | "remove-account" | "move" | "archive-folder";

export interface AppContext {
  selectedAccount: AccountDto | null;
  dialog: DialogName | null;
  /** Nombre de conversations affichées. */
  conversations: number;
  /** Une conversation est ouverte (vue de lecture). */
  reading: boolean;
  /** Une action peut viser au moins une conversation. */
  hasTargets: boolean;
  /** La dernière action peut être annulée. */
  canUndo: boolean;
}

class AppState {
  accounts = $state<AccountDto[]>([]);
  selectedAccountId = $state<number | null>(null);
  dialog = $state<DialogName | null>(null);
  loadError = $state<string | null>(null);

  readonly selectedAccount = $derived(
    this.accounts.find((a) => a.id === this.selectedAccountId) ?? null,
  );

  context(): AppContext {
    return {
      selectedAccount: this.selectedAccount,
      dialog: this.dialog,
      conversations: mail.conversations.length,
      reading: mail.opened !== null,
      hasTargets: mail.targets().length > 0,
      canUndo: mail.lastAction !== null,
    };
  }

  open(dialog: DialogName): void {
    this.dialog = dialog;
  }

  close(): void {
    this.dialog = null;
  }

  async refreshAccounts(): Promise<void> {
    try {
      this.accounts = await api.listAccounts();
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
      when: (ctx) => ctx.conversations > 0,
      run: () => mail.next(),
    }),
    registry.register({
      id: "conversation.previous",
      title: m.cmd_previous(),
      group: m.group_reading(),
      shortcut: "k",
      when: (ctx) => ctx.conversations > 0,
      run: () => mail.previous(),
    }),
    registry.register({
      id: "conversation.open",
      title: m.cmd_open(),
      group: m.group_reading(),
      shortcut: "Enter",
      when: (ctx) => ctx.conversations > 0 && !ctx.reading,
      run: () => mail.open(),
    }),
    registry.register({
      id: "conversation.back",
      title: m.cmd_back(),
      group: m.group_reading(),
      shortcut: "Escape",
      when: (ctx) => ctx.reading,
      run: () => mail.back(),
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
      id: "action.undo",
      title: m.cmd_undo(),
      group: m.group_actions(),
      shortcut: "z",
      keywords: words(m.cmd_undo_keywords()),
      when: (ctx) => ctx.canUndo,
      run: () => mail.undo(),
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
