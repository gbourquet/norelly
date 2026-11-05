// Accès au contrat IPC généré (bindings.ts) : les erreurs deviennent des
// exceptions `IpcFailure`. Aucune logique de sécurité ici : le Rust
// revalide tout.

import { Channel } from "@tauri-apps/api/core";
import {
  commands,
  events,
  type AccountDto,
  type ConversationCursorDto,
  type ConversationMessageDto_Serialize,
  type ConversationPageDto_Serialize,
  type FolderDto,
  type MessageViewDto,
  type ReadingPreferencesDto,
  type ScopeDto,
  type SyncEventDto,
  type AddAccountInput_Deserialize,
  type AddAccountResultDto_Serialize,
  type DiscoveryDto_Serialize,
  type DiscoverySourceDto,
  type IpcError,
  type ActionDto,
  type ActionResultDto,
  type ComposeEventDto,
  type ComposeKindDto,
  type DraftAttachmentDto,
  type DraftDto_Serialize,
  type OutboxItemDto,
  type RecipientDto,
  type SavedDraftDto,
  type SearchHitDto_Serialize,
  type SearchPageDto_Serialize,
  type SearchScopeDto,
  type SettingsDto,
  type DraftsDto,
  type UpdateEventDto,
  type UpdateStatusDto_Serialize,
} from "./bindings";
import { IpcFailure } from "../errors";

export type {
  ComposeEventDto,
  ComposeKindDto,
  DraftAttachmentDto,
  OutboxItemDto,
  OutboxStateDto,
  RecipientDto,
  SavedDraftDto,
  SendFailureDto,
  AccountDto,
  ActionDto,
  ActionResultDto,
  AttachmentDto,
  BodyModeDto,
  ConversationCursorDto,
  ConversationDto,
  CorrespondentDto,
  FolderDto,
  MessageViewDto,
  ReadingPreferencesDto,
  ScopeDto,
  SyncEventDto,
  CertificateProblemDto,
  DiscoveredServerDto,
  DiscoverySourceDto,
  EndpointDto,
  ErrorCodeDto,
  FailureKindDto,
  FieldDto,
  SecurityDto,
  ServerDto,
  SearchScopeDto,
  SegmentDto,
  SettingsDto,
  DraftSummaryDto,
  DraftsDto,
  RemoteDraftDto,
  LanguageDto,
  UpdateCheckDto,
  UpdateEventDto,
} from "./bindings";
export type AddAccountInput = AddAccountInput_Deserialize;
export type AddAccountResult = AddAccountResultDto_Serialize;
export type Discovery = DiscoveryDto_Serialize;
export type ConversationPage = ConversationPageDto_Serialize;
export type ConversationMessage = ConversationMessageDto_Serialize;
export type SearchHit = SearchHitDto_Serialize;
export type SearchPage = SearchPageDto_Serialize;
/** Brouillon échangé avec l'éditeur (même forme dans les deux sens). */
export type Draft = DraftDto_Serialize;

export { IpcFailure } from "../errors";

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: IpcError };

async function unwrap<T>(pending: Promise<Result<T>>): Promise<T> {
  const result = await pending;
  if (result.status === "error") {
    const { code, field, detail } = result.error;
    throw new IpcFailure(code, field ?? null, detail ?? null);
  }
  return result.data;
}

export const api = {
  listAccounts: (): Promise<AccountDto[]> => unwrap(commands.listAccounts()),

  discoverSettings: (
    address: string,
    onProgress: (source: DiscoverySourceDto) => void,
  ): Promise<Discovery> => {
    const channel = new Channel<DiscoverySourceDto>();
    channel.onmessage = onProgress;
    return unwrap(commands.discoverSettings(address, channel));
  },

  addAccount: (input: AddAccountInput): Promise<AddAccountResult> =>
    unwrap(commands.addAccount(input)),

  trustCertificate: (host: string, fingerprint: string): Promise<void> =>
    unwrap(commands.trustCertificate(host, fingerprint)).then(() => undefined),

  removeAccount: (id: number): Promise<void> =>
    unwrap(commands.removeAccount(id)).then(() => undefined),

  allowFileSecrets: (): Promise<void> => unwrap(commands.allowFileSecrets()).then(() => undefined),

  listFolders: (account: number): Promise<FolderDto[]> => unwrap(commands.listFolders(account)),

  listConversations: (
    scope: ScopeDto,
    cursor: ConversationCursorDto | null,
    limit: number,
  ): Promise<ConversationPage> => unwrap(commands.listConversations(scope, cursor, limit)),

  conversationMessages: (conversation: number): Promise<ConversationMessage[]> =>
    unwrap(commands.conversationMessages(conversation)),

  openMessage: (message: number): Promise<MessageViewDto> => unwrap(commands.openMessage(message)),

  allowRemoteImages: (message: number): Promise<void> =>
    unwrap(commands.allowRemoteImages(message)).then(() => undefined),

  readingPreferences: (): Promise<ReadingPreferencesDto> => unwrap(commands.readingPreferences()),

  setPreferPlainText: (prefer: boolean): Promise<void> =>
    unwrap(commands.setPreferPlainText(prefer)).then(() => undefined),

  syncAccount: (account: number): Promise<void> =>
    unwrap(commands.syncAccount(account)).then(() => undefined),

  openExternal: (url: string): Promise<void> =>
    unwrap(commands.openExternal(url)).then(() => undefined),

  /** `title` : titre du sélecteur de dossier, localisé. */
  saveAttachment: (message: number, part: string, title: string): Promise<string | null> =>
    unwrap(commands.saveAttachment(message, part, title)),

  openAttachment: (message: number, part: string): Promise<void> =>
    unwrap(commands.openAttachment(message, part)).then(() => undefined),

  /** Recherche locale (hors ligne) ; `utcOffsetMinutes` : fuseau de
   * l'utilisateur, pour `before:` et `after:`. */
  searchMessages: (
    query: string,
    scope: SearchScopeDto,
    utcOffsetMinutes: number,
    offset: number,
    limit: number,
  ): Promise<SearchPage> =>
    unwrap(commands.searchMessages(query, scope, utcOffsetMinutes, offset, limit)),

  getSettings: (): Promise<SettingsDto> => unwrap(commands.getSettings()),

  saveSettings: (settings: SettingsDto): Promise<void> =>
    unwrap(commands.saveSettings(settings)).then(() => undefined),

  /** Notification du système ; le texte est revalidé côté Rust. */
  showNotification: (title: string, body: string): Promise<void> =>
    unwrap(commands.showNotification(title, body)).then(() => undefined),

  /** Action sur des conversations de la portée affichée. */
  performAction: (
    conversations: number[],
    scope: ScopeDto,
    action: ActionDto,
  ): Promise<ActionResultDto> => unwrap(commands.performAction(conversations, scope, action)),

  /** Annule des opérations en attente ; renvoie le nombre annulé. */
  undoActions: (ops: number[]): Promise<number> => unwrap(commands.undoActions(ops)),

  setArchiveFolder: (account: number, mailbox: number | null): Promise<void> =>
    unwrap(commands.setArchiveFolder(account, mailbox)).then(() => undefined),

  /** Brouillon de départ ; `attribution` : en-tête de citation localisé. */
  prepareCompose: (
    account: number,
    kind: ComposeKindDto,
    message: number | null,
    attribution: string,
  ): Promise<Draft> => unwrap(commands.prepareCompose(account, kind, message, attribution)),

  saveDraft: (draft: Draft): Promise<SavedDraftDto> => unwrap(commands.saveDraft(draft)),

  /** Met en file d'envoi ; renvoie l'identifiant dans la file. */
  sendDraft: (draft: Draft): Promise<number> => unwrap(commands.sendDraft(draft)),

  /** Programme l'envoi à `at` (secondes Unix). */
  scheduleDraft: (draft: Draft, at: number): Promise<number> =>
    unwrap(commands.scheduleDraft(draft, at)),

  /** Reprend un envoi programmé : son brouillon. */
  unscheduleSend: (account: number, id: number): Promise<Draft> =>
    unwrap(commands.unscheduleSend(account, id)),

  listDrafts: (account: number): Promise<DraftsDto> => unwrap(commands.listDrafts(account)),

  openDraft: (id: number): Promise<Draft> => unwrap(commands.openDraft(id)),

  discardDraft: (id: number): Promise<void> =>
    unwrap(commands.discardDraft(id)).then(() => undefined),

  outbox: (account: number): Promise<OutboxItemDto[]> => unwrap(commands.outbox(account)),

  retrySend: (account: number, id: number): Promise<void> =>
    unwrap(commands.retrySend(account, id)).then(() => undefined),

  cancelSend: (account: number, id: number): Promise<void> =>
    unwrap(commands.cancelSend(account, id)).then(() => undefined),

  suggestContacts: (account: number, text: string): Promise<RecipientDto[]> =>
    unwrap(commands.suggestContacts(account, text)),

  /** `title` : titre du sélecteur de fichiers, localisé. */
  pickAttachments: (account: number, title: string): Promise<DraftAttachmentDto[]> =>
    unwrap(commands.pickAttachments(account, title)),

  attachDropped: (account: number, token: number): Promise<DraftAttachmentDto[]> =>
    unwrap(commands.attachDropped(account, token)),

  onComposeEvent: async (listener: (event: ComposeEventDto) => void): Promise<() => void> =>
    events.composeEventDto.listen((e) => listener(e.payload)),

  /** Mises à jour : canal pris en charge, version annoncée (signature vérifiée au téléchargement). */
  updateStatus: (): Promise<UpdateStatusDto_Serialize> => commands.updateStatus(),

  /** Installe la mise à jour trouvée (l'installateur ferme l'application). */
  installUpdate: (): Promise<void> => unwrap(commands.installUpdate()).then(() => undefined),

  onUpdateEvent: async (listener: (event: UpdateEventDto) => void): Promise<() => void> =>
    events.updateEventDto.listen((e) => listener(e.payload)),

  /** Abonnement aux événements de synchronisation ; renvoie le retrait. */
  onSyncEvent: async (listener: (event: SyncEventDto) => void): Promise<() => void> =>
    events.syncEventDto.listen((e) => listener(e.payload)),
};

export type Api = typeof api;

/** Commandes des comptes (ajout, suppression). */
export type AccountApi = Pick<
  Api,
  | "listAccounts"
  | "discoverSettings"
  | "addAccount"
  | "trustCertificate"
  | "removeAccount"
  | "allowFileSecrets"
>;
