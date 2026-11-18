// État de la rédaction (runes Svelte 5) : brouillon ouvert, destinataires,
// pièces jointes, enregistrement automatique, envoi. Présentation
// seulement : tout est revalidé en Rust (adresses, sujet, tailles, HTML
// réassaini). Les fichiers joints sont choisis par Rust (sélecteur) ou
// désignés par un jeton (dépôt sur la fenêtre) : aucun chemin ici.

import type {
  Api,
  ComposeKindDto,
  ContactListItemDto,
  ConversationMessage,
  Draft,
  DraftAttachmentDto,
  RecipientDto,
  RecipientSuggestion,
} from "./ipc/api";
import { errorText, IpcFailure, recipientLimitText } from "./errors";
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
  | "parseRecipient"
  | "checkRecipients"
  | "suggestContacts"
  | "suggestContactLists"
  | "expandContactList"
  | "pickAttachments"
  | "attachDropped"
>;

export type RecipientField = "to" | "cc" | "bcc";

/** Clé d'une adresse pour reconnaître un doublon : à la casse près. */
const key = (address: string): string => address.toLowerCase();

export class ComposeState {
  readonly #api: ComposeApi;
  draft = $state<Draft | null>(null);
  error = $state<string | null>(null);
  busy = $state(false);
  suggestions = $state<RecipientSuggestion[]>([]);
  listSuggestions = $state<ContactListItemDto[]>([]);
  /** Destinataires refusés à l'ajout (limite du message dépassée),
   * affiché près des champs ; effacé au prochain ajout ou retrait. */
  recipientWarning = $state<string | null>(null);
  /** Modifié depuis le dernier enregistrement. */
  #dirty = false;
  #saveTimer: ReturnType<typeof setTimeout> | null = null;
  #suggestTimer: ReturnType<typeof setTimeout> | null = null;
  #query = 0;
  #saving: Promise<void> | null = null;
  /** Numéro du brouillon ouvert : une lecture en cours pour un brouillon
   * refermé ou remplacé n'en touche pas un autre. */
  #session = 0;

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
    this.#session++;
    if (this.#saveTimer !== null) clearTimeout(this.#saveTimer);
    if (this.#suggestTimer !== null) clearTimeout(this.#suggestTimer);
    this.#saveTimer = null;
    this.#suggestTimer = null;
    this.#dirty = false;
    this.draft = null;
    this.error = null;
    this.recipientWarning = null;
    this.suggestions = [];
    this.listSuggestions = [];
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

  /** Saisie d'un destinataire, lue et validée en Rust ; `false` si elle
   * est refusée ou si l'adresse est déjà présente. */
  async addRecipient(field: RecipientField, raw: string): Promise<boolean> {
    if (this.draft === null || raw.trim() === "") return false;
    const session = this.#session;
    this.recipientWarning = null;
    let parsed: RecipientDto;
    try {
      parsed = await this.#api.parseRecipient(raw);
    } catch {
      return false;
    }
    const inserted = await this.#insert(field, [parsed], session);
    if (inserted === null || inserted === 0) return false;
    this.suggestions = [];
    this.listSuggestions = [];
    return true;
  }

  /** Destinataires du brouillon, tous champs confondus. */
  #count(): number {
    const d = this.draft;
    return d === null ? 0 : d.to.length + d.cc.length + d.bcc.length;
  }

  /** `read` sans les adresses déjà présentes (à la casse près), chacune
   * une fois. */
  #unseen(read: RecipientDto[]): RecipientDto[] {
    if (this.draft === null) return [];
    // Ensemble local, jamais rendu : un objet sans prototype suffit (pas
    // de réactivité à suivre).
    const seen: Record<string, true> = Object.create(null) as Record<string, true>;
    for (const r of [...this.draft.to, ...this.draft.cc, ...this.draft.bcc]) {
      seen[key(r.address)] = true;
    }
    const added: RecipientDto[] = [];
    for (const r of read) {
      if (seen[key(r.address)]) continue;
      seen[key(r.address)] = true;
      added.push({ name: r.name, address: r.address });
    }
    return added;
  }

  /** Ajoute à `field` ceux de `read` absents du brouillon, si Rust accepte
   * le total ; revérifié tant qu'un autre ajout l'a changé pendant la
   * vérification. Nombre ajouté, ou `null` si rien ne l'est (limite
   * dépassée, avertissement affiché ; brouillon refermé ou remplacé). */
  async #insert(
    field: RecipientField,
    read: RecipientDto[],
    session: number,
  ): Promise<number | null> {
    let checked: number | null = null;
    for (;;) {
      if (this.draft === null || session !== this.#session) return null;
      const added = this.#unseen(read);
      if (added.length === 0) return 0;
      const total = this.#count() + added.length;
      if (total === checked) {
        this.draft = { ...this.draft, [field]: [...this.draft[field], ...added] };
        this.recipientWarning = null;
        this.#changed();
        return added.length;
      }
      if (!(await this.#fits(total))) return null;
      checked = total;
    }
  }

  /** Le message peut-il compter `count` destinataires, décidé par Rust ;
   * sinon l'avertissement indique la limite et le nombre, pour prévenir
   * dès l'ajout plutôt qu'à l'enregistrement ou à l'envoi. */
  async #fits(count: number): Promise<boolean> {
    try {
      await this.#api.checkRecipients(count);
      return true;
    } catch (e) {
      if (e instanceof IpcFailure && e.code === "too_many_recipients") {
        this.recipientWarning = recipientLimitText(count);
      } else {
        this.recipientWarning = null;
        this.error = errorText(e);
      }
      return false;
    }
  }

  /** Ajoute des destinataires déjà connus (« Écrire à »), relus par Rust
   * comme une saisie (nom imitant une adresse écarté) ; une adresse déjà
   * présente ou refusée est ignorée. `false` (rien d'ajouté) si le total
   * dépasserait la limite de destinataires (avertissement affiché) ou si
   * le brouillon a changé entre-temps. */
  async addRecipients(field: RecipientField, recipients: RecipientDto[]): Promise<boolean> {
    const session = this.#session;
    this.recipientWarning = null;
    const read: RecipientDto[] = [];
    for (const r of recipients) {
      try {
        read.push(await this.#api.parseRecipient(r.name ? `${r.name} <${r.address}>` : r.address));
      } catch {
        // Adresse refusée par Rust : ignorée.
      }
    }
    return (await this.#insert(field, read, session)) !== null;
  }

  /** Liste de diffusion choisie dans les suggestions : ses membres,
   * dépliés par Rust (adresse préférée de chacun), deviennent des puces
   * relues comme une saisie, dans le champ décidé par Rust (la copie
   * cachée pour une liste « en Cci », quel que soit `field`). `false` si
   * la liste est refusée (erreur affichée) ou si elle ferait dépasser la
   * limite de destinataires (rien d'inséré, avertissement affiché). */
  async addList(field: RecipientField, id: number): Promise<boolean> {
    if (this.draft === null) return false;
    const session = this.#session;
    try {
      const expanded = await this.#api.expandContactList(id, field);
      if (session !== this.#session) return false;
      if (!(await this.addRecipients(expanded.field, expanded.recipients))) {
        // Refusée (limite) : les propositions ne masquent pas l'avertissement.
        this.suggestions = [];
        this.listSuggestions = [];
        return false;
      }
    } catch (e) {
      this.error = errorText(e);
      return false;
    }
    this.suggestions = [];
    this.listSuggestions = [];
    return true;
  }

  removeRecipient(field: RecipientField, index: number): void {
    if (this.draft === null) return;
    this.draft = { ...this.draft, [field]: this.draft[field].filter((_, i) => i !== index) };
    this.recipientWarning = null;
    this.#changed();
  }

  /** Saisie d'un destinataire : suggestions après une courte pause. */
  query(text: string): void {
    if (this.#suggestTimer !== null) clearTimeout(this.#suggestTimer);
    const ticket = ++this.#query;
    const account = this.draft?.account;
    if (text.trim() === "" || account === undefined) {
      this.suggestions = [];
      this.listSuggestions = [];
      return;
    }
    this.#suggestTimer = setTimeout(() => {
      this.#suggestTimer = null;
      this.#api
        .suggestContactLists(text)
        .then((found) => {
          if (ticket === this.#query) this.listSuggestions = found;
        })
        .catch(() => {
          if (ticket === this.#query) this.listSuggestions = [];
        });
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
