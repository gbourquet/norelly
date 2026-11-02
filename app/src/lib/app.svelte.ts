// État de l'interface (runes Svelte 5) et registre de commandes de
// l'application.

import { CommandRegistry } from "./commands/registry";
import { api, type AccountDto } from "./ipc/api";
import { MailState } from "./mail.svelte";

export type DialogName = "palette" | "help" | "add-account" | "remove-account";

export interface AppContext {
  selectedAccount: AccountDto | null;
  dialog: DialogName | null;
  /** Nombre de conversations affichées. */
  conversations: number;
  /** Une conversation est ouverte (vue de lecture). */
  reading: boolean;
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
      this.loadError = e instanceof Error ? e.message : String(e);
    }
  }
}

export const mail = new MailState(api);

export const app = new AppState();

export const registry = new CommandRegistry<AppContext>();

/** Commandes de base de l'application. Renvoie la fonction de retrait. */
export function registerAppCommands(): () => void {
  const offs = [
    registry.register({
      id: "palette.open",
      title: "Palette de commandes",
      group: "Général",
      shortcut: "Ctrl+K",
      inPalette: false,
      run: () => app.open("palette"),
    }),
    registry.register({
      id: "help.shortcuts",
      title: "Aide des raccourcis clavier",
      group: "Général",
      shortcut: "?",
      keywords: ["clavier", "touches"],
      run: () => app.open("help"),
    }),
    registry.register({
      id: "account.add",
      title: "Ajouter un compte…",
      group: "Comptes",
      keywords: ["imap", "smtp", "nouveau"],
      run: () => app.open("add-account"),
    }),
    registry.register({
      id: "conversation.next",
      title: "Conversation suivante",
      group: "Lecture",
      shortcut: "j",
      when: (ctx) => ctx.conversations > 0,
      run: () => mail.next(),
    }),
    registry.register({
      id: "conversation.previous",
      title: "Conversation précédente",
      group: "Lecture",
      shortcut: "k",
      when: (ctx) => ctx.conversations > 0,
      run: () => mail.previous(),
    }),
    registry.register({
      id: "conversation.open",
      title: "Ouvrir la conversation",
      group: "Lecture",
      shortcut: "Enter",
      when: (ctx) => ctx.conversations > 0 && !ctx.reading,
      run: () => mail.open(),
    }),
    registry.register({
      id: "conversation.back",
      title: "Retour à la liste",
      group: "Lecture",
      shortcut: "Escape",
      when: (ctx) => ctx.reading,
      run: () => mail.back(),
    }),
    registry.register({
      id: "mailbox.unified",
      title: "Boîte de réception unifiée",
      group: "Lecture",
      keywords: ["inbox", "tous les comptes"],
      run: () => mail.show({ kind: "unified", role: "inbox" }),
    }),
    registry.register({
      id: "reading.plain-text",
      title: "Préférer le texte brut (ou le HTML)",
      group: "Lecture",
      keywords: ["html", "texte", "affichage"],
      run: async () => {
        const prefs = await api.readingPreferences();
        await api.setPreferPlainText(!prefs.prefer_plain_text);
        if (mail.opened !== null) await mail.open();
      },
    }),
    registry.register({
      id: "account.sync",
      title: "Synchroniser le compte sélectionné",
      group: "Comptes",
      keywords: ["actualiser", "relever"],
      when: (ctx) => ctx.selectedAccount !== null,
      run: (ctx) => (ctx.selectedAccount ? api.syncAccount(ctx.selectedAccount.id) : undefined),
    }),
    registry.register({
      id: "account.remove",
      title: "Supprimer le compte sélectionné…",
      group: "Comptes",
      keywords: ["retirer", "effacer"],
      when: (ctx) => ctx.selectedAccount !== null,
      run: () => app.open("remove-account"),
    }),
  ];
  return () => offs.forEach((off) => off());
}
