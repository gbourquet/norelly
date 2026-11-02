// État de l'interface (runes Svelte 5) et registre de commandes de
// l'application.

import { CommandRegistry } from "./commands/registry";
import { api, type AccountDto } from "./ipc/api";

export type DialogName = "palette" | "help" | "add-account" | "remove-account";

export interface AppContext {
  selectedAccount: AccountDto | null;
  dialog: DialogName | null;
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
    return { selectedAccount: this.selectedAccount, dialog: this.dialog };
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
    } catch (e) {
      this.loadError = e instanceof Error ? e.message : String(e);
    }
  }
}

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
