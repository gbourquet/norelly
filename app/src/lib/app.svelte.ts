// État de l'interface (runes Svelte 5) et registre de commandes de
// l'application.

import { CommandRegistry } from "./commands/registry";
import {
  api,
  type AccountDto,
  type ComposeKindDto,
  type CorrespondentDto,
  type DraftsDto,
  type FolderDto,
  type OutboxItemDto,
} from "./ipc/api";
import { isEditableFolder } from "./folder-edit.svelte";
import { ComposeState } from "./compose.svelte";
import { MailState } from "./mail.svelte";
import { ContactFormState, ContactsState, DeleteContactsState } from "./contacts.svelte";
import { ContactExportState } from "./contact-export.svelte";
import { ContactDuplicatesState } from "./contact-duplicates.svelte";
import { ContactImportState } from "./contact-import.svelte";
import {
  AddToListState,
  ContactListFormState,
  ContactListsState,
  DeleteContactListState,
} from "./contact-lists.svelte";
import { SearchState } from "./search.svelte";
import { SettingsState } from "./settings.svelte";
import { UpdatesState } from "./updates.svelte";
import type { ConversationMessage } from "./ipc/api";
import { failureLabel } from "./outbox";
import type { PersonKind } from "./visible-actions";
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
  | "new-folder"
  | "rename-folder"
  | "move-folder"
  | "delete-folder"
  | "compose"
  | "outbox"
  | "settings"
  | "drafts"
  | "contact"
  | "delete-contacts"
  | "contact-list"
  | "delete-contact-list"
  | "add-to-list"
  | "contact-import"
  | "contact-export"
  | "contact-duplicates";

export interface AppContext {
  selectedAccount: AccountDto | null;
  /** Dossier visé par les commandes de dossier : celui dont le menu est
   * ouvert, sinon le dossier affiché. */
  folder: FolderDto | null;
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
  /** Le carnet d'adresses est affiché. */
  contactsView: boolean;
  /** Nombre de contacts visés (menu d'une ligne, cochés, fiche ouverte). */
  contactTargets: number;
  /** Nombre de contacts cochés. */
  contactsMarked: number;
  /** La section « Listes » de la vue Contacts est affichée. */
  listsView: boolean;
  /** Une liste est visée (menu d'une ligne, liste ouverte). */
  listTargets: number;
  /** Personne d'un message visée ([`AppState.targetPerson`]) : déjà dans
   * le carnet, inconnue du carnet, ou aucune. */
  person: PersonKind;
}

/** Nature d'une personne pour les commandes du carnet. */
function personKind(person: CorrespondentDto | null): PersonKind {
  if (person === null || person.address === "") return null;
  return person.contact !== null ? "contact" : "stranger";
}

/** Vue affichée dans les panneaux de liste et de lecture. */
export type ViewName = "mail" | "contacts";

class AppState {
  accounts = $state<AccountDto[]>([]);
  view = $state<ViewName>("mail");
  /** Contact dont le menu contextuel est ouvert (sinon : les cochés, ou
   * la fiche ouverte). */
  contactTarget = $state<number | null>(null);
  /** Liste dont le menu contextuel est ouvert (sinon : la liste ouverte). */
  listTarget = $state<number | null>(null);
  /** Liste dont la suppression est demandée (dialogue ouvert). */
  listToDelete = $state<number | null>(null);
  /** Contacts à ajouter à une liste (dialogue ouvert). */
  contactsToAdd = $state<number[]>([]);
  /** Personne d'un message dont le menu est ouvert (expéditeur,
   * destinataire, ligne de la liste). */
  personTarget = $state<CorrespondentDto | null>(null);
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
  /** Dossier dont le menu a été ouvert (sinon : le dossier affiché). */
  folderTarget = $state<FolderDto | null>(null);
  /** Échecs déjà signalés (identifiants dans la file). */
  #reported = new Set<number>();

  readonly selectedAccount = $derived(
    this.accounts.find((a) => a.id === this.selectedAccountId) ?? null,
  );

  accountById(id: number): AccountDto | null {
    return this.accounts.find((a) => a.id === id) ?? null;
  }

  /** Dossier visé par les commandes de dossier ([`AppContext.folder`]). */
  targetFolder(): FolderDto | null {
    if (this.folderTarget !== null) return this.folderTarget;
    const scope = mail.scope;
    if (scope.kind !== "mailbox" || this.selectedAccountId === null) return null;
    return (mail.folders[this.selectedAccountId] ?? []).find((f) => f.id === scope.id) ?? null;
  }

  context(): AppContext {
    if (this.view === "contacts") {
      // Courrier masqué : ses commandes ne visent rien ; section « Listes »
      // affichée : celles des contacts non plus.
      const listsView = lists.shown;
      return {
        selectedAccount: this.selectedAccount,
        folder: null,
        dialog: this.dialog,
        conversations: 0,
        reading: false,
        hasTargets: false,
        marked: 0,
        canUndo: mail.lastAction !== null,
        searching: false,
        results: 0,
        contactsView: true,
        contactTargets: listsView ? 0 : this.contactTargets().length,
        contactsMarked: listsView ? 0 : contacts.marked.length,
        listsView,
        listTargets: listsView && this.listTargetId() !== null ? 1 : 0,
        person: null,
      };
    }
    return {
      selectedAccount: this.selectedAccount,
      folder: this.targetFolder(),
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
      contactsView: false,
      contactTargets: 0,
      contactsMarked: 0,
      listsView: false,
      listTargets: 0,
      person: personKind(this.targetPerson()),
    };
  }

  /** Personne visée par « Ajouter au carnet » et « Voir le contact » :
   * celle dont le menu est ouvert, sinon l'expéditeur du dernier message
   * de la conversation ouverte. */
  targetPerson(): CorrespondentDto | null {
    if (this.view !== "mail") return null;
    if (this.personTarget !== null) return this.personTarget;
    if (mail.opened === null) return null;
    return mail.messages.at(-1)?.from ?? null;
  }

  /** Personne visée, puis menu oublié (la commande l'a prise en charge). */
  takePerson(): CorrespondentDto | null {
    const person = this.targetPerson();
    this.personTarget = null;
    return person;
  }

  /** Contexte des commandes qui visent `person` (menu d'une personne). */
  contextForPerson(person: CorrespondentDto): AppContext {
    return { ...this.context(), person: personKind(person) };
  }

  /** Contacts visés par les commandes du carnet : celui dont le menu est
   * ouvert, sinon les cochés, sinon la fiche ouverte. */
  contactTargets(): number[] {
    return this.contactTarget !== null ? [this.contactTarget] : contacts.targets();
  }

  /** Contacts visés, puis menu d'une ligne oublié (la commande les a
   * pris en charge). */
  takeContactTargets(): number[] {
    const targets = this.contactTargets();
    this.contactTarget = null;
    return targets;
  }

  /** Liste visée par les commandes des listes : celle dont le menu est
   * ouvert, sinon la liste ouverte. */
  listTargetId(): number | null {
    return this.listTarget ?? lists.selectedId;
  }

  /** Liste visée, puis menu d'une ligne oublié. */
  takeListTarget(): number | null {
    const id = this.listTargetId();
    this.listTarget = null;
    return id;
  }

  /** Contexte des commandes qui visent la liste `id` (menu d'une ligne). */
  contextForList(id: number): AppContext {
    return { ...this.context(), listTargets: id > 0 ? 1 : 0 };
  }

  /** Contacts dont la suppression est demandée (dialogue ouvert). */
  contactsToDelete = $state<number[]>([]);

  /** Affiche le carnet d'adresses à la place du courrier. */
  async showContacts(): Promise<void> {
    this.view = "contacts";
    search.close();
    await contacts.load();
  }

  /** Revient au courrier, relu : les noms tiennent compte des fiches
   * modifiées dans le carnet. */
  showMail(): void {
    const fromContacts = this.view === "contacts";
    this.view = "mail";
    this.contactTarget = null;
    if (fromContacts) void mail.reload();
  }

  /** Contexte des commandes qui visent le contact `id` (menu d'une
   * ligne). */
  contextFor(id: number): AppContext {
    return { ...this.context(), contactTargets: id > 0 ? 1 : 0 };
  }

  open(dialog: DialogName): void {
    this.dialog = dialog;
  }

  close(): void {
    this.dialog = null;
    this.folderTarget = null;
    this.contactTarget = null;
    this.personTarget = null;
    this.listTarget = null;
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

/** Exécute la commande `id` pour `person` (menu d'une personne d'un
 * message, d'une ligne de la liste). */
export async function runForPerson(id: string, person: CorrespondentDto): Promise<void> {
  const context = app.contextForPerson(person);
  app.personTarget = person;
  try {
    await registry.execute(id, context);
  } finally {
    app.personTarget = null;
  }
}

export const mail = new MailState(api, () => {
  const current = settings.current;
  return current === null ? null : current.mark_seen_delay_secs * 1000;
});

export const search = new SearchState(api);

export const contacts = new ContactsState(api);

/** Formulaire d'ajout ou de modification d'un contact. */
export const contactForm = new ContactFormState(api);

/** Confirmation de suppression de contacts. */
export const contactDeletion = new DeleteContactsState(api);
export const contactImport = new ContactImportState(api);
export const contactExport = new ContactExportState(api);
export const contactDuplicates = new ContactDuplicatesState(api);

/** Listes de diffusion (section « Listes » de la vue Contacts). */
export const lists = new ContactListsState(api);

/** Formulaire de création ou de modification d'une liste. */
export const listForm = new ContactListFormState(api);

/** Ajout de contacts à une liste. */
export const addToList = new AddToListState(api);

/** Confirmation de suppression d'une liste. */
export const listDeletion = new DeleteContactListState(api);

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
      run: () => {
        app.showMail();
        search.open();
      },
    }),
    registry.register({
      id: "mailbox.unified",
      title: m.cmd_unified(),
      group: m.group_reading(),
      keywords: words(m.cmd_unified_keywords()),
      run: () => {
        app.showMail();
        return mail.show({ kind: "unified", role: "inbox" });
      },
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
      id: "account.new-folder",
      title: m.cmd_new_folder(),
      group: m.group_accounts(),
      keywords: words(m.cmd_new_folder_keywords()),
      when: (ctx) => ctx.selectedAccount !== null,
      run: () => app.open("new-folder"),
    }),
    registry.register({
      id: "folder.rename",
      title: m.cmd_rename_folder(),
      group: m.group_accounts(),
      keywords: words(m.cmd_rename_folder_keywords()),
      when: (ctx) =>
        ctx.selectedAccount !== null && ctx.folder !== null && isEditableFolder(ctx.folder),
      run: () => app.open("rename-folder"),
    }),
    registry.register({
      id: "folder.move",
      title: m.cmd_move_folder(),
      group: m.group_accounts(),
      keywords: words(m.cmd_move_folder_keywords()),
      when: (ctx) =>
        ctx.selectedAccount !== null && ctx.folder !== null && isEditableFolder(ctx.folder),
      run: () => app.open("move-folder"),
    }),
    registry.register({
      id: "folder.delete",
      title: m.cmd_delete_folder(),
      group: m.group_accounts(),
      keywords: words(m.cmd_delete_folder_keywords()),
      when: (ctx) =>
        ctx.selectedAccount !== null && ctx.folder !== null && isEditableFolder(ctx.folder),
      run: () => app.open("delete-folder"),
    }),
    registry.register({
      id: "contacts.open",
      title: m.cmd_contacts(),
      group: m.group_contacts(),
      shortcut: "Ctrl+Shift+B",
      keywords: words(m.cmd_contacts_keywords()),
      run: () => app.showContacts(),
    }),
    registry.register({
      id: "contacts.new",
      title: m.cmd_contact_new(),
      group: m.group_contacts(),
      shortcut: "n",
      keywords: words(m.cmd_contact_new_keywords()),
      run: async () => {
        if (app.view !== "contacts") await app.showContacts();
        app.contactTarget = null;
        contactForm.reset(null);
        app.open("contact");
      },
    }),
    registry.register({
      id: "contacts.import",
      title: m.cmd_contacts_import(),
      group: m.group_contacts(),
      keywords: words(m.cmd_contacts_import_keywords()),
      run: async () => {
        if (app.view !== "contacts") await app.showContacts();
        try {
          const shown = await contactImport.choose(
            m.contacts_import_pick_title(),
            m.contacts_import_pick_filter(),
          );
          if (shown) app.open("contact-import");
        } catch (e) {
          mail.notice = { text: errorText(e), undoable: false };
        }
      },
    }),
    registry.register({
      id: "contacts.export",
      title: m.cmd_contacts_export(),
      group: m.group_contacts(),
      keywords: words(m.cmd_contacts_export_keywords()),
      run: async () => {
        if (app.view !== "contacts") await app.showContacts();
        contactExport.open({ kind: "all" });
        app.open("contact-export");
      },
    }),
    registry.register({
      id: "contacts.export-selected",
      title: m.cmd_contacts_export_selected(),
      group: m.group_contacts(),
      keywords: words(m.cmd_contacts_export_selected_keywords()),
      when: (ctx) => ctx.contactsView && ctx.contactTargets > 0,
      run: () => {
        const ids = app.takeContactTargets();
        if (ids.length === 0) return;
        contactExport.open({ kind: "contacts", ids });
        app.open("contact-export");
      },
    }),
    registry.register({
      id: "contacts.find-duplicates",
      title: m.cmd_contacts_find_duplicates(),
      group: m.group_contacts(),
      keywords: words(m.cmd_contacts_find_duplicates_keywords()),
      run: async () => {
        if (app.view !== "contacts") await app.showContacts();
        app.open("contact-duplicates");
        await contactDuplicates.open();
      },
    }),
    registry.register({
      id: "contacts.edit",
      title: m.cmd_contact_edit(),
      group: m.group_contacts(),
      shortcut: "Shift+E",
      keywords: words(m.cmd_contact_edit_keywords()),
      when: (ctx) => ctx.contactsView && ctx.contactTargets === 1,
      run: async () => {
        const [id] = app.takeContactTargets();
        if (id === undefined) return;
        try {
          contactForm.reset(await api.getContact(id));
          app.open("contact");
        } catch (e) {
          mail.notice = { text: errorText(e), undoable: false };
        }
      },
    }),
    registry.register({
      id: "contacts.write",
      title: m.cmd_contact_write(),
      group: m.group_contacts(),
      shortcut: "w",
      keywords: words(m.cmd_contact_write_keywords()),
      when: (ctx) => ctx.contactsView && ctx.contactTargets > 0 && ctx.selectedAccount !== null,
      run: async (ctx) => {
        const account = ctx.selectedAccount;
        const recipients = await contacts.recipients(app.takeContactTargets());
        if (account === null || recipients.length === 0) return;
        await app.compose("new", account.id, null);
        await compose.addRecipients("to", recipients);
      },
    }),
    registry.register({
      id: "contacts.favorite",
      title: m.cmd_contact_favorite(),
      group: m.group_contacts(),
      shortcut: "Shift+S",
      keywords: words(m.cmd_contact_favorite_keywords()),
      when: (ctx) => ctx.contactsView && ctx.contactTargets === 1,
      run: async () => {
        const [id] = app.takeContactTargets();
        if (id === undefined) return;
        if (contacts.selectedId !== id) await contacts.select(id);
        const name = contacts.selected?.name ?? "";
        const favorite = await contacts.toggleFavorite();
        if (favorite === null) {
          if (contacts.error !== null) mail.notice = { text: contacts.error, undoable: false };
          return;
        }
        mail.notice = {
          text: favorite
            ? m.contacts_notice_favorited({ name })
            : m.contacts_notice_unfavorited({ name }),
          undoable: false,
        };
      },
    }),
    registry.register({
      id: "contacts.delete",
      title: m.cmd_contact_delete(),
      group: m.group_contacts(),
      shortcut: "Delete",
      keywords: words(m.cmd_contact_delete_keywords()),
      when: (ctx) => ctx.contactsView && ctx.contactTargets > 0,
      run: () => {
        app.contactsToDelete = app.takeContactTargets();
        contactDeletion.reset();
        app.open("delete-contacts");
      },
    }),
    registry.register({
      id: "contacts.add-to-list",
      title: m.cmd_contact_add_to_list(),
      group: m.group_contacts(),
      keywords: words(m.cmd_contact_add_to_list_keywords()),
      when: (ctx) => ctx.contactsView && ctx.contactTargets > 0,
      run: async () => {
        app.contactsToAdd = app.takeContactTargets();
        app.open("add-to-list");
        await addToList.open();
      },
    }),
    registry.register({
      id: "lists.new",
      title: m.cmd_list_new(),
      group: m.group_contacts(),
      keywords: words(m.cmd_list_new_keywords()),
      run: async () => {
        if (app.view !== "contacts") await app.showContacts();
        if (!lists.shown) await lists.show(true);
        app.listTarget = null;
        listForm.reset(null);
        app.open("contact-list");
      },
    }),
    registry.register({
      id: "lists.edit",
      title: m.cmd_list_edit(),
      group: m.group_contacts(),
      keywords: words(m.cmd_list_edit_keywords()),
      when: (ctx) => ctx.listsView && ctx.listTargets === 1,
      run: async () => {
        const id = app.takeListTarget();
        if (id === null) return;
        try {
          listForm.reset(await api.getContactList(id));
          app.open("contact-list");
        } catch (e) {
          mail.notice = { text: errorText(e), undoable: false };
        }
      },
    }),
    registry.register({
      id: "lists.write",
      title: m.cmd_list_write(),
      group: m.group_contacts(),
      keywords: words(m.cmd_list_write_keywords()),
      when: (ctx) => ctx.listsView && ctx.listTargets === 1 && ctx.selectedAccount !== null,
      run: async (ctx) => {
        const account = ctx.selectedAccount;
        const id = app.takeListTarget();
        if (account === null || id === null) return;
        await app.compose("new", account.id, null);
        await compose.addList("to", id);
      },
    }),
    registry.register({
      id: "lists.delete",
      title: m.cmd_list_delete(),
      group: m.group_contacts(),
      keywords: words(m.cmd_list_delete_keywords()),
      when: (ctx) => ctx.listsView && ctx.listTargets === 1,
      run: () => {
        app.listToDelete = app.takeListTarget();
        listDeletion.reset();
        app.open("delete-contact-list");
      },
    }),
    registry.register({
      id: "contacts.add-person",
      title: m.cmd_person_add(),
      group: m.group_contacts(),
      keywords: words(m.cmd_person_add_keywords()),
      when: (ctx) => ctx.person === "stranger",
      run: () => {
        const person = app.takePerson();
        if (person === null) return;
        contactForm.prefill(person);
        app.open("contact");
      },
    }),
    registry.register({
      id: "contacts.show-person",
      title: m.cmd_person_show(),
      group: m.group_contacts(),
      keywords: words(m.cmd_person_show_keywords()),
      when: (ctx) => ctx.person === "contact",
      run: async () => {
        const id = app.takePerson()?.contact ?? null;
        if (id === null) return;
        await app.showContacts();
        await contacts.select(id);
      },
    }),
    registry.register({
      id: "contacts.clear-selection",
      title: m.cmd_contacts_clear_selection(),
      group: m.group_contacts(),
      keywords: words(m.cmd_contacts_clear_selection_keywords()),
      when: (ctx) => ctx.contactsView && ctx.contactsMarked > 0,
      run: () => contacts.clearMarks(),
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
      id: "account.reconnect",
      title: m.cmd_account_reconnect(),
      group: m.group_accounts(),
      keywords: words(m.cmd_account_reconnect_keywords()),
      when: (ctx) => (ctx.selectedAccount?.oauth_provider ?? null) !== null,
      run: (ctx) => (ctx.selectedAccount ? mail.reconnect(ctx.selectedAccount.id) : undefined),
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
