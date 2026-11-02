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
} from "./bindings";

export type {
  AccountDto,
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
  SecurityDto,
  ServerDto,
} from "./bindings";
export type AddAccountInput = AddAccountInput_Deserialize;
export type AddAccountResult = AddAccountResultDto_Serialize;
export type Discovery = DiscoveryDto_Serialize;
export type ConversationPage = ConversationPageDto_Serialize;
export type ConversationMessage = ConversationMessageDto_Serialize;

export class IpcFailure extends Error {}

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: { message: string } };

async function unwrap<T>(pending: Promise<Result<T>>): Promise<T> {
  const result = await pending;
  if (result.status === "error") throw new IpcFailure(result.error.message);
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

  saveAttachment: (message: number, part: string): Promise<string | null> =>
    unwrap(commands.saveAttachment(message, part)),

  openAttachment: (message: number, part: string): Promise<void> =>
    unwrap(commands.openAttachment(message, part)).then(() => undefined),

  /** Abonnement aux événements de synchronisation ; renvoie le retrait. */
  onSyncEvent: async (listener: (event: SyncEventDto) => void): Promise<() => void> =>
    events.syncEventDto.listen((e) => listener(e.payload)),
};

export type Api = typeof api;

/** Commandes des comptes (ajout, suppression). */
export type AccountApi = Pick<
  Api,
  "listAccounts" | "discoverSettings" | "addAccount" | "trustCertificate" | "removeAccount"
>;
