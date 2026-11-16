// Échec d'une création, d'un renommage, d'un déplacement ou d'une
// suppression de dossier : texte affiché, et relecture de la liste quand le
// résultat est inconnu. La décision (résultat inconnu ou non) est prise par
// Rust.

import { IpcFailure, errorText } from "./errors";

/** Relit la liste des dossiers d'un compte. */
export type ReloadFolders = (account: number) => Promise<void>;

/** Texte d'un échec de modification de dossier ; résultat inconnu (la
 * modification a pu aboutir sur le serveur) : la liste des dossiers du
 * compte est d'abord relue, pour montrer l'état réel. */
export async function folderFailureText(
  e: unknown,
  account: number,
  reload: ReloadFolders | undefined,
): Promise<string> {
  if (e instanceof IpcFailure && e.code === "folder_outcome_unknown") {
    try {
      await reload?.(account);
    } catch {
      // Relecture impossible : l'événement de liste la rattrapera.
    }
  }
  return errorText(e);
}
