// Synchronisation CardDAV du carnet (runes Svelte 5, ADR-014) : adresse du
// serveur, identifiant et mot de passe d'application, choix du carnet
// distant parmi ceux que Rust a trouvés, puis synchronisation. Toute
// décision (adresse, carnet relié, conflits) est prise en Rust ; le mot de
// passe n'est gardé ici que le temps de la saisie.

import type { Api, CardDavBookDto, CardDavReportDto, CardDavStatus } from "./ipc/api";
import { errorText } from "./errors";

export type CardDavApi = Pick<
  Api,
  "carddavStatus" | "findCarddavBooks" | "connectCarddav" | "syncCarddav" | "disconnectCarddav"
>;

/** Fiches laissées de côté par une synchronisation : celles qu'un carnet
 * plein n'a pas pu recevoir, et les autres (sans adresse, en double). */
export function leftAside(report: CardDavReportDto): { other: number; bookFull: number } {
  return { other: Math.max(0, report.skipped - report.book_full), bookFull: report.book_full };
}

export class CardDavState {
  readonly #api: CardDavApi;
  status = $state<CardDavStatus | null>(null);
  server = $state("");
  username = $state("");
  password = $state("");
  /** Carnets trouvés ; `null` tant qu'aucune recherche n'a abouti. */
  books = $state<CardDavBookDto[] | null>(null);
  /** Adresse du carnet choisi. */
  book = $state<string | null>(null);
  busy = $state(false);
  error = $state<string | null>(null);
  /** Un carnet distant est relié. */
  readonly connected = $derived(this.status?.account != null);
  readonly canSearch = $derived(
    !this.busy && this.server.trim() !== "" && this.username.trim() !== "" && this.password !== "",
  );

  constructor(api: CardDavApi) {
    this.#api = api;
  }

  /** Fenêtre ouverte : état relu, formulaire vide. */
  async open(): Promise<void> {
    this.password = "";
    this.books = null;
    this.book = null;
    this.error = null;
    this.busy = false;
    try {
      this.status = await this.#api.carddavStatus();
    } catch (e) {
      this.error = errorText(e);
    }
  }

  /** Adresse ou identifiant modifiés : les carnets trouvés ne valent plus. */
  edited(): void {
    this.books = null;
    this.book = null;
  }

  /** Cherche les carnets du compte ; le premier est choisi. */
  async search(): Promise<void> {
    if (!this.canSearch) return;
    this.busy = true;
    this.error = null;
    try {
      const books = await this.#api.findCarddavBooks(this.server, this.username, this.password);
      this.books = books;
      this.book = books[0]?.url ?? null;
    } catch (e) {
      this.error = errorText(e);
      this.books = null;
      this.book = null;
    } finally {
      this.busy = false;
    }
  }

  /** Relie le carnet choisi, puis synchronise ; renvoie le bilan, sinon
   * `null` (message affiché). */
  async connect(): Promise<CardDavReportDto | null> {
    const book = this.book;
    if (book === null || this.busy) return null;
    this.busy = true;
    this.error = null;
    try {
      this.status = await this.#api.connectCarddav(this.server, this.username, this.password, book);
      this.password = "";
      this.books = null;
      this.book = null;
    } catch (e) {
      this.error = errorText(e);
      return null;
    } finally {
      this.busy = false;
    }
    return this.sync();
  }

  /** Synchronise maintenant ; renvoie le bilan, sinon `null` (message
   * affiché, état enregistré relu). */
  async sync(): Promise<CardDavReportDto | null> {
    if (this.busy) return null;
    this.busy = true;
    this.error = null;
    let report: CardDavReportDto | null = null;
    try {
      report = await this.#api.syncCarddav();
    } catch (e) {
      this.error = errorText(e);
    }
    try {
      this.status = await this.#api.carddavStatus();
    } catch (e) {
      this.error ??= errorText(e);
    } finally {
      this.busy = false;
    }
    return report;
  }

  /** Oublie le carnet distant (les contacts restent). */
  async disconnect(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.error = null;
    try {
      await this.#api.disconnectCarddav();
      this.status = await this.#api.carddavStatus();
    } catch (e) {
      this.error = errorText(e);
    } finally {
      this.busy = false;
    }
  }

  /** Synchronisation au lancement, si un carnet est relié : bilan, sinon
   * `null` (échec enregistré, visible dans la fenêtre). */
  async background(): Promise<CardDavReportDto | null> {
    try {
      this.status = await this.#api.carddavStatus();
      if (this.status.account === null) return null;
      const report = await this.#api.syncCarddav();
      this.status = await this.#api.carddavStatus();
      return report;
    } catch {
      return null;
    }
  }

  /** Fenêtre fermée : le mot de passe saisi est oublié. */
  close(): void {
    this.password = "";
    this.books = null;
    this.book = null;
    this.error = null;
  }
}
