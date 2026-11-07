// Listes de diffusion du carnet (runes Svelte 5) : section « Listes » de
// la vue Contacts, formulaire de création ou de modification, ajout de
// contacts à une liste, suppression. Saisie revalidée et refus décidés
// par Rust (ADR-012) ; une liste regroupe des contacts, chacun écrit à son
// adresse préférée.

import type {
  Api,
  ContactItem,
  ContactList,
  ContactListCursorDto,
  ContactListFieldsDto,
  ContactListItemDto,
} from "./ipc/api";
import { errorText } from "./errors";

export type ContactListsApi = Pick<
  Api,
  | "listContactLists"
  | "getContactList"
  | "createContactList"
  | "updateContactList"
  | "deleteContactList"
  | "addToContactList"
  | "listContacts"
>;

/** Listes lues par page. */
export const LIST_PAGE = 100;
/** Membres proposés au plus (Rust refuse au-delà). */
export const MAX_LIST_MEMBERS = 100;
/** Contacts proposés au plus par la recherche du formulaire. */
const CANDIDATES = 20;

/** Liste vide. */
export function emptyListFields(): ContactListFieldsDto {
  return { name: "", description: "", members: [], bcc: false };
}

export class ContactListsState {
  readonly #api: ContactListsApi;
  /** La section « Listes » remplace la liste des contacts. */
  shown = $state(false);
  items = $state<ContactListItemDto[]>([]);
  next = $state<ContactListCursorDto | null>(null);
  text = $state("");
  loading = $state(false);
  error = $state<string | null>(null);
  selectedId = $state<number | null>(null);
  selected = $state<ContactList | null>(null);
  /** Numéro du dernier chargement : une réponse plus ancienne est
   * ignorée. */
  #generation = 0;

  readonly hasMore = $derived(this.next !== null);

  constructor(api: ContactListsApi) {
    this.#api = api;
  }

  /** Affiche la section (listes relues) ou revient aux contacts. */
  async show(shown: boolean): Promise<void> {
    this.shown = shown;
    if (shown) await this.load();
  }

  /** Première page, selon le filtre en cours. */
  async load(): Promise<void> {
    const generation = ++this.#generation;
    this.loading = true;
    try {
      const page = await this.#api.listContactLists(this.text, null, LIST_PAGE);
      if (generation !== this.#generation) return;
      this.items = page.items;
      this.next = page.next;
      this.error = null;
    } catch (e) {
      if (generation === this.#generation) this.error = errorText(e);
    } finally {
      if (generation === this.#generation) this.loading = false;
    }
  }

  /** Page suivante, ajoutée à la suite. */
  async loadMore(): Promise<void> {
    const cursor = this.next;
    if (cursor === null || this.loading) return;
    const generation = this.#generation;
    this.loading = true;
    try {
      const page = await this.#api.listContactLists(this.text, cursor, LIST_PAGE);
      if (generation !== this.#generation) return;
      this.items = [...this.items, ...page.items];
      this.next = page.next;
    } catch (e) {
      if (generation === this.#generation) this.error = errorText(e);
    } finally {
      if (generation === this.#generation) this.loading = false;
    }
  }

  async search(text: string): Promise<void> {
    this.text = text;
    await this.load();
  }

  /** Ouvre la liste `id` (`null` : aucune). */
  async select(id: number | null): Promise<void> {
    this.selectedId = id;
    if (id === null) {
      this.selected = null;
      return;
    }
    try {
      const list = await this.#api.getContactList(id);
      if (this.selectedId === id) this.selected = list;
    } catch (e) {
      if (this.selectedId !== id) return;
      this.selectedId = null;
      this.selected = null;
      this.error = errorText(e);
    }
  }

  /** Après un enregistrement : liste ouverte, listes relues. */
  async saved(list: ContactList): Promise<void> {
    this.selectedId = list.id;
    this.selected = list;
    await this.load();
  }

  /** Après une suppression : liste fermée, listes relues. */
  async removed(id: number): Promise<void> {
    if (this.selectedId === id) {
      this.selectedId = null;
      this.selected = null;
    }
    await this.load();
  }
}

export class ContactListFormState {
  readonly #api: ContactListsApi;
  fields = $state<ContactListFieldsDto>(emptyListFields());
  /** Liste modifiée ; `null` : nouvelle liste. */
  id = $state<number | null>(null);
  /** Membres affichés (nom, adresse préférée), dans l'ordre. */
  members = $state<ContactItem[]>([]);
  /** Contacts trouvés par la recherche du formulaire. */
  candidates = $state<ContactItem[]>([]);
  busy = $state(false);
  error = $state<string | null>(null);
  #query = 0;

  readonly canSubmit = $derived(!this.busy && this.fields.name.trim() !== "");
  readonly canAdd = $derived(this.members.length < MAX_LIST_MEMBERS);

  constructor(api: ContactListsApi) {
    this.#api = api;
  }

  /** Formulaire de `list` (copie), ou vide. */
  reset(list: ContactList | null): void {
    this.id = list?.id ?? null;
    this.fields = list === null ? emptyListFields() : structuredClone($state.snapshot(list.fields));
    this.members = list === null ? [] : structuredClone($state.snapshot(list.members));
    this.candidates = [];
    this.error = null;
    this.busy = false;
    this.#query++;
  }

  /** Contacts dont le nom ou une adresse contient `text` (vide : aucun) ;
   * une réponse dépassée par une saisie plus récente est ignorée. */
  async find(text: string): Promise<void> {
    const ticket = ++this.#query;
    if (text.trim() === "") {
      this.candidates = [];
      return;
    }
    try {
      const page = await this.#api.listContacts(text, false, null, CANDIDATES);
      if (ticket === this.#query) this.candidates = page.items;
    } catch (e) {
      if (ticket === this.#query) this.error = errorText(e);
    }
  }

  isMember(id: number): boolean {
    return this.members.some((m) => m.id === id);
  }

  /** Ajoute un contact (à la fin), s'il n'est pas déjà membre et s'il
   * reste de la place. */
  add(contact: ContactItem): void {
    if (!this.canAdd || this.isMember(contact.id)) return;
    this.members.push(contact);
  }

  remove(id: number): void {
    this.members = this.members.filter((m) => m.id !== id);
  }

  /** Enregistre la liste ; renvoie la liste enregistrée, sinon `null`
   * (erreur affichée, saisie gardée). */
  async submit(): Promise<ContactList | null> {
    if (!this.canSubmit) return null;
    this.busy = true;
    this.error = null;
    try {
      const payload = {
        ...$state.snapshot(this.fields),
        members: this.members.map((m) => m.id),
      };
      return this.id === null
        ? await this.#api.createContactList(payload)
        : await this.#api.updateContactList(this.id, payload);
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
  }
}

export class AddToListState {
  readonly #api: ContactListsApi;
  lists = $state<ContactListItemDto[]>([]);
  /** Liste choisie (la première par défaut). */
  chosen = $state<number | null>(null);
  loading = $state(false);
  busy = $state(false);
  error = $state<string | null>(null);

  constructor(api: ContactListsApi) {
    this.#api = api;
  }

  /** Listes proposées (la première page, dans l'ordre du carnet). */
  async open(): Promise<void> {
    this.lists = [];
    this.chosen = null;
    this.error = null;
    this.busy = false;
    this.loading = true;
    try {
      const page = await this.#api.listContactLists("", null, LIST_PAGE);
      this.lists = page.items;
      this.chosen = page.items[0]?.id ?? null;
    } catch (e) {
      this.error = errorText(e);
    } finally {
      this.loading = false;
    }
  }

  /** Ajoute `contacts` à la liste choisie ; renvoie la liste modifiée,
   * sinon `null` (erreur affichée). */
  async submit(contacts: number[]): Promise<ContactList | null> {
    const id = this.chosen;
    if (id === null || contacts.length === 0) return null;
    this.busy = true;
    this.error = null;
    try {
      return await this.#api.addToContactList(id, contacts);
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
  }
}

export class DeleteContactListState {
  readonly #api: ContactListsApi;
  busy = $state(false);
  error = $state<string | null>(null);

  constructor(api: ContactListsApi) {
    this.#api = api;
  }

  reset(): void {
    this.busy = false;
    this.error = null;
  }

  /** Supprime la liste `id` ; `true` si elle a été supprimée, `false` si
   * elle n'existait plus, `null` en cas d'échec (erreur affichée). */
  async submit(id: number): Promise<boolean | null> {
    this.busy = true;
    this.error = null;
    try {
      return await this.#api.deleteContactList(id);
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
  }
}
