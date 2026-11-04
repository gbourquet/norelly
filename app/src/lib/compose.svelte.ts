// État de la rédaction (runes Svelte 5) : brouillon ouvert, destinataires,
// pièces jointes, enregistrement automatique, envoi. Présentation
// seulement : tout est revalidé en Rust (adresses, sujet, tailles, HTML
// réassaini). Les fichiers joints sont choisis par Rust (sélecteur) ou
// désignés par un jeton (dépôt sur la fenêtre) : aucun chemin ici.

import type {
  Api,
  ComposeKindDto,
  ConversationMessage,
  Draft,
  DraftAttachmentDto,
  RecipientDto,
} from "./ipc/api";
import { errorText } from "./errors";
import { longDate, personName } from "./format";
import { m } from "./paraglide/messages.js";

/** Pause de frappe avant l'enregistrement automatique. */
export const AUTOSAVE_MS = 2000;
/** Attente avant de demander des suggestions d'adresses. */
export const SUGGEST_MS = 150;

export type ComposeApi = Pick<
  Api,
  | "prepareCompose"
  | "saveDraft"
  | "sendDraft"
  | "scheduleDraft"
  | "discardDraft"
  | "suggestContacts"
  | "pickAttachments"
  | "attachDropped"
>;

export type RecipientField = "to" | "cc" | "bcc";

/** « Nom <adresse> », « "Dupont, Bob" <adresse> » ou « adresse ». La
 * validation réelle est faite en Rust. */
export function parseRecipient(raw: string): RecipientDto | null {
  const text = raw.trim();
  const angle = /^(.*)<([^<>\s]+)>$/.exec(text);
  const name = angle ? (angle[1] ?? "").trim().replace(/^"(.*)"$/, "$1") : "";
  const address = angle ? (angle[2] ?? "") : text;
  if (!/^[^\s@]+@[^\s@]+$/.test(address)) return null;
  return { name, address };
}

const same = (a: string, b: string): boolean => a.toLowerCase() === b.toLowerCase();

export class ComposeState {
  readonly #api: ComposeApi;
  draft = $state<Draft | null>(null);
  error = $state<string | null>(null);
  busy = $state(false);
  suggestions = $state<RecipientDto[]>([]);
  /** Modifié depuis le dernier enregistrement. */
  #dirty = false;
  #saveTimer: ReturnType<typeof setTimeout> | null = null;
  #suggestTimer: ReturnType<typeof setTimeout> | null = null;
  #query = 0;
  #saving: Promise<void> | null = null;

  constructor(api: ComposeApi) {
    this.#api = api;
  }

  /** Ouvre l'éditeur ; `message` : message auquel on répond (dernier de
   * la conversation ouverte). */
  async open(
    account: number,
    kind: ComposeKindDto,
    message: ConversationMessage | null,
  ): Promise<void> {
    this.#reset();
    let attribution = "";
    if (kind !== "new" && message !== null) {
      const values = { date: longDate(message.date), name: personName(message.from) };
      attribution =
        kind === "forward" ? m.compose_forward_intro(values) : m.compose_attribution(values);
    }
    try {
      this.draft = await this.#api.prepareCompose(
        account,
        kind,
        kind === "new" ? null : (message?.id ?? null),
        attribution,
      );
    } catch (e) {
      this.error = errorText(e);
    }
  }

  #reset(): void {
    if (this.#saveTimer !== null) clearTimeout(this.#saveTimer);
    if (this.#suggestTimer !== null) clearTimeout(this.#suggestTimer);
    this.#saveTimer = null;
    this.#suggestTimer = null;
    this.#dirty = false;
    this.draft = null;
    this.error = null;
    this.suggestions = [];
  }

  #changed(): void {
    this.#dirty = true;
    if (this.#saveTimer !== null) clearTimeout(this.#saveTimer);
    this.#saveTimer = setTimeout(() => {
      this.#saveTimer = null;
      void this.flush();
    }, AUTOSAVE_MS);
  }

  /** Modification par l'éditeur ou un champ : enregistrement différé. */
  edit(change: Partial<Pick<Draft, "subject" | "html">>): void {
    if (this.draft === null) return;
    this.draft = { ...this.draft, ...change };
    this.#changed();
  }

  addRecipient(field: RecipientField, raw: string): boolean {
    const parsed = parseRecipient(raw);
    if (this.draft === null || parsed === null) return false;
    const all = [...this.draft.to, ...this.draft.cc, ...this.draft.bcc];
    if (all.some((r) => same(r.address, parsed.address))) return false;
    this.draft = { ...this.draft, [field]: [...this.draft[field], parsed] };
    this.suggestions = [];
    this.#changed();
    return true;
  }

  removeRecipient(field: RecipientField, index: number): void {
    if (this.draft === null) return;
    this.draft = { ...this.draft, [field]: this.draft[field].filter((_, i) => i !== index) };
    this.#changed();
  }

  /** Saisie d'un destinataire : suggestions après une courte pause. */
  query(text: string): void {
    if (this.#suggestTimer !== null) clearTimeout(this.#suggestTimer);
    const ticket = ++this.#query;
    const account = this.draft?.account;
    if (text.trim() === "" || account === undefined) {
      this.suggestions = [];
      return;
    }
    this.#suggestTimer = setTimeout(() => {
      this.#suggestTimer = null;
      this.#api
        .suggestContacts(account, text)
        .then((found) => {
          // Réponse périmée (saisie poursuivie) : ignorée.
          if (ticket === this.#query) this.suggestions = found;
        })
        .catch(() => {
          if (ticket === this.#query) this.suggestions = [];
        });
    }, SUGGEST_MS);
  }

  async #attach(pending: Promise<DraftAttachmentDto[]>): Promise<void> {
    try {
      const added = await pending;
      if (this.draft === null || added.length === 0) return;
      this.draft = { ...this.draft, attachments: [...this.draft.attachments, ...added] };
      this.#changed();
    } catch (e) {
      this.error = errorText(e);
    }
  }

  async pickAttachments(title: string): Promise<void> {
    if (this.draft === null) return;
    await this.#attach(this.#api.pickAttachments(this.draft.account, title));
  }

  async attachDropped(token: number): Promise<void> {
    if (this.draft === null) return;
    await this.#attach(this.#api.attachDropped(this.draft.account, token));
  }

  removeAttachment(index: number): void {
    if (this.draft === null) return;
    this.draft = {
      ...this.draft,
      attachments: this.draft.attachments.filter((_, i) => i !== index),
    };
    this.#changed();
  }

  /** Enregistre tout de suite si nécessaire. */
  async flush(): Promise<void> {
    if (this.#saveTimer !== null) {
      clearTimeout(this.#saveTimer);
      this.#saveTimer = null;
    }
    if (this.#saving !== null) await this.#saving;
    if (this.draft === null || !this.#dirty) return;
    const draft = $state.snapshot(this.draft);
    this.#dirty = false;
    this.#saving = this.#api
      .saveDraft(draft)
      .then((saved) => {
        if (this.draft !== null && this.draft.id === null) {
          this.draft = { ...this.draft, id: saved.id };
        }
      })
      .catch((e: unknown) => {
        this.#dirty = true;
        this.error = errorText(e);
      })
      .finally(() => {
        this.#saving = null;
      });
    await this.#saving;
  }

  /** Rouvre un brouillon enregistré (liste des brouillons, envoi
   * programmé repris). */
  resume(draft: Draft): void {
    this.#reset();
    this.draft = draft;
  }

  /** Envoie (`at` : échéance d'un envoi différé, secondes Unix) ; `true`
   * si le message est parti dans la file. */
  async send(at: number | null = null): Promise<boolean> {
    if (this.draft === null || this.busy) return false;
    if (this.#saveTimer !== null) clearTimeout(this.#saveTimer);
    this.#saveTimer = null;
    if (this.#saving !== null) await this.#saving;
    this.busy = true;
    try {
      const draft = $state.snapshot(this.draft);
      if (at === null) await this.#api.sendDraft(draft);
      else await this.#api.scheduleDraft(draft, at);
      this.#reset();
      return true;
    } catch (e) {
      this.error = errorText(e);
      return false;
    } finally {
      this.busy = false;
    }
  }

  /** Ferme l'éditeur en gardant le brouillon. */
  async close(): Promise<void> {
    await this.flush();
    this.#reset();
  }

  /** Abandonne le brouillon. */
  async discard(): Promise<void> {
    if (this.#saveTimer !== null) clearTimeout(this.#saveTimer);
    this.#saveTimer = null;
    if (this.#saving !== null) await this.#saving;
    const id = this.draft?.id ?? null;
    this.#reset();
    if (id !== null) {
      try {
        await this.#api.discardDraft(id);
      } catch (e) {
        this.error = errorText(e);
      }
    }
  }

  /** Pièces jointes affichées. */
  get attachments(): DraftAttachmentDto[] {
    return this.draft?.attachments ?? [];
  }
}
