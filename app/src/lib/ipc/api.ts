// Accès au contrat IPC généré (bindings.ts) : les erreurs deviennent des
// exceptions `IpcFailure`. Aucune logique de sécurité ici : le Rust
// revalide tout.

import { Channel } from "@tauri-apps/api/core";
import {
  commands,
  type AccountDto,
  type AddAccountInput_Deserialize,
  type AddAccountResultDto_Serialize,
  type DiscoveryDto_Serialize,
  type DiscoverySourceDto,
} from "./bindings";

export type {
  AccountDto,
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
};

export type Api = typeof api;
