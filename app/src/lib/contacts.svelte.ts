// Carnet d'adresses (runes Svelte 5) : liste filtrée et paginée, fiche
// ouverte, contacts cochés, formulaire d'ajout ou de modification et
// suppression. Saisie revalidée et refus décidés par Rust (ADR-012).

import type {
  Api,
  ContactCursorDto,
  ContactDto,
  ContactFieldsDto,
  ContactItem,
  RecipientDto,
} from "./ipc/api";
import { errorText } from "./errors";

export type ContactsApi = Pick<
  Api,
  "listContacts" | "getContact" | "createContact" | "updateContact" | "deleteContacts"
>;

/** Contacts lus par page. */
export const CONTACT_PAGE = 100;

/** Lignes d'adresses et de téléphones proposées au plus (Rust refuse
 * au-delà). */
const MAX_ROWS = 20;

/** Fiche vide. */
export function emptyFields(): ContactFieldsDto {
  return {
    display_name: "",
    given_name: "",
    family_name: "",
    nickname: "",
    organization: "",
    notes: "",
    emails: [],
    phones: [],
    favorite: false,
  };
}

/** Destinataire d'un contact : sans nom si son nom affiché est son
 * adresse (fiche sans nom). */
function recipient(name: string, address: string): RecipientDto {
  return { name: name === address ? "" : name, address };
}

export class ContactsState {
  readonly #api: ContactsApi;
  items = $state<ContactItem[]>([]);
  next = $state<ContactCursorDto | null>(null);
  text = $state("");
  favoritesOnly = $state(false);
  loading = $state(false);
  error = $state<string | null>(null);
  selectedId = $state<number | null>(null);
  selected = $state<ContactDto | null>(null);
  marked = $state<number[]>([]);
  /** Numéro du dernier chargement : une réponse plus ancienne est
   * ignorée. */
  #generation = 0;

  readonly hasMore = $derived(this.next !== null);

  constructor(api: ContactsApi) {
    this.#api = api;
  }

  /** Première page, selon le filtre en cours. */
  async load(): Promise<void> {
    const generation = ++this.#generation;
    this.loading = true;
    try {
      const page = await this.#api.listContacts(this.text, this.favoritesOnly, null, CONTACT_PAGE);
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

  /** Page suivante, ajoutée à la liste. */
  async loadMore(): Promise<void> {
    const cursor = this.next;
    if (cursor === null || this.loading) return;
    const generation = this.#generation;
    this.loading = true;
    try {
      const page = await this.#api.listContacts(
        this.text,
        this.favoritesOnly,
        cursor,
        CONTACT_PAGE,
      );
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

  async showFavorites(only: boolean): Promise<void> {
    this.favoritesOnly = only;
    await this.load();
  }

  /** Ouvre la fiche `id` (`null` : aucune). */
  async select(id: number | null): Promise<void> {
    this.selectedId = id;
    if (id === null) {
      this.selected = null;
      return;
    }
    try {
      const contact = await this.#api.getContact(id);
      if (this.selectedId === id) this.selected = contact;
    } catch (e) {
      if (this.selectedId !== id) return;
      this.selectedId = null;
      this.selected = null;
      this.error = errorText(e);
    }
  }

  toggleMark(id: number): void {
    this.marked = this.marked.includes(id)
      ? this.marked.filter((m) => m !== id)
      : [...this.marked, id];
  }

  clearMarks(): void {
    this.marked = [];
  }

  /** Contacts visés par une action : les cochés, sinon la fiche ouverte. */
  targets(): number[] {
    if (this.marked.length > 0) return [...this.marked];
    return this.selectedId === null ? [] : [this.selectedId];
  }

  /** Destinataires « Écrire à » : nom et adresse préférée de chaque
   * contact visé (fiche relue s'il n'est pas dans la liste affichée ;
   * contact disparu : ignoré). */
  async recipients(ids: number[]): Promise<RecipientDto[]> {
    const result: RecipientDto[] = [];
    for (const id of ids) {
      const shown = this.items.find((i) => i.id === id);
      if (shown !== undefined) {
        result.push(recipient(shown.name, shown.address));
        continue;
      }
      try {
        const contact = await this.#api.getContact(id);
        const preferred =
          contact.fields.emails.find((e) => e.preferred) ?? contact.fields.emails[0];
        if (preferred !== undefined) result.push(recipient(contact.name, preferred.address));
      } catch {
        // Supprimé entre-temps : rien à écrire.
      }
    }
    return result;
  }

  /** Ajoute la fiche ouverte aux favoris, ou l'en retire ; renvoie le
   * nouvel état (`null` : rien d'ouvert ou échec, erreur affichée). */
  async toggleFavorite(): Promise<boolean | null> {
    const current = this.selected;
    if (current === null) return null;
    const fields = { ...$state.snapshot(current.fields), favorite: !current.fields.favorite };
    try {
      const updated = await this.#api.updateContact(current.id, fields);
      await this.saved(updated);
      return updated.fields.favorite;
    } catch (e) {
      this.error = errorText(e);
      return null;
    }
  }

  /** Après un enregistrement : liste relue, fiche ouverte. */
  async saved(contact: ContactDto): Promise<void> {
    this.selectedId = contact.id;
    this.selected = contact;
    await this.load();
  }

  /** Après une suppression : fiches retirées de la sélection et des cases,
   * liste relue. */
  async removed(ids: number[]): Promise<void> {
    this.marked = this.marked.filter((id) => !ids.includes(id));
    if (this.selectedId !== null && ids.includes(this.selectedId)) {
      this.selectedId = null;
      this.selected = null;
    }
    await this.load();
  }
}

export class ContactFormState {
  readonly #api: ContactsApi;
  fields = $state<ContactFieldsDto>(emptyFields());
  /** Fiche modifiée ; `null` : nouveau contact. */
  id = $state<number | null>(null);
  busy = $state(false);
  error = $state<string | null>(null);

  readonly canSubmit = $derived(
    !this.busy && this.fields.emails.some((e) => e.address.trim() !== ""),
  );
  readonly canAddEmail = $derived(this.fields.emails.length < MAX_ROWS);
  readonly canAddPhone = $derived(this.fields.phones.length < MAX_ROWS);

  constructor(api: ContactsApi) {
    this.#api = api;
  }

  /** Formulaire de la fiche `contact` (copie), ou vide avec une adresse à
   * saisir. */
  reset(contact: ContactDto | null): void {
    this.id = contact?.id ?? null;
    this.fields =
      contact === null ? emptyFields() : structuredClone($state.snapshot(contact.fields));
    if (this.fields.emails.length === 0) this.addEmail();
    this.error = null;
    this.busy = false;
  }

  /** Nouveau contact pré-rempli avec un correspondant (« Ajouter au
   * carnet ») : son nom tel qu'affiché, son adresse préférée. */
  prefill(person: { name: string; address: string }): void {
    this.reset(null);
    this.fields.display_name = person.name;
    const first = this.fields.emails[0];
    if (first !== undefined) first.address = person.address;
  }

  addEmail(): void {
    if (!this.canAddEmail) return;
    const preferred = this.fields.emails.length === 0;
    this.fields.emails.push({ address: "", label: "", preferred });
  }

  /** Retire une adresse (la dernière ligne reste) ; l'adresse préférée
   * retirée, la première le devient. */
  removeEmail(index: number): void {
    const emails = this.fields.emails;
    if (emails.length <= 1 || emails[index] === undefined) return;
    const wasPreferred = emails[index].preferred;
    emails.splice(index, 1);
    const first = emails[0];
    if (wasPreferred && first !== undefined) first.preferred = true;
  }

  setPreferred(index: number): void {
    this.fields.emails.forEach((e, i) => (e.preferred = i === index));
  }

  addPhone(): void {
    if (this.canAddPhone) this.fields.phones.push({ number: "", label: "" });
  }

  removePhone(index: number): void {
    this.fields.phones.splice(index, 1);
  }

  /** Fiche envoyée : sans les lignes laissées vides. */
  #payload(): ContactFieldsDto {
    const fields = $state.snapshot(this.fields);
    return {
      ...fields,
      emails: fields.emails.filter((e) => e.address.trim() !== ""),
      phones: fields.phones.filter((p) => p.number.trim() !== "" || p.label.trim() !== ""),
    };
  }

  /** Enregistre la fiche ; renvoie la fiche enregistrée, sinon `null`
   * (erreur affichée, saisie gardée). */
  async submit(): Promise<ContactDto | null> {
    if (!this.canSubmit) return null;
    this.busy = true;
    this.error = null;
    try {
      const payload = this.#payload();
      return this.id === null
        ? await this.#api.createContact(payload)
        : await this.#api.updateContact(this.id, payload);
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
  }
}

export class DeleteContactsState {
  readonly #api: ContactsApi;
  busy = $state(false);
  error = $state<string | null>(null);

  constructor(api: ContactsApi) {
    this.#api = api;
  }

  reset(): void {
    this.busy = false;
    this.error = null;
  }

  /** Supprime les fiches `ids` ; renvoie le nombre supprimé, sinon `null`
   * (erreur affichée). */
  async submit(ids: number[]): Promise<number | null> {
    this.busy = true;
    this.error = null;
    try {
      return await this.#api.deleteContacts(ids);
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
  }
}
