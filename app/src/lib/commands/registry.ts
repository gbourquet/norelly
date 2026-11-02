// Registre de commandes central (ADR-005) : alimente la palette Ctrl+K, les
// raccourcis clavier, l'aide (« ? ») et, plus tard, les menus contextuels.
//
// Le registre ne prend aucune décision de sécurité : une commande ne fait
// qu'appeler l'interface ou une commande IPC, revalidée côté Rust.

import { hasCommandModifier, parseShortcut, shortcutId, type Shortcut } from "./shortcut";

export interface Command<C> {
  /** Identifiant stable, par exemple « account.add ». */
  id: string;
  /** Libellé affiché dans la palette et l'aide. */
  title: string;
  /** Regroupement dans la palette et l'aide. */
  group: string;
  /** Raccourci, par exemple « Ctrl+K », « Shift+I », « ? ». */
  shortcut?: string;
  /** Mots-clés supplémentaires pour la recherche. */
  keywords?: string[];
  /** Condition d'activation ; absente : toujours active. */
  when?: (ctx: C) => boolean;
  /** Visible dans la palette (vrai par défaut). */
  inPalette?: boolean;
  run: (ctx: C) => void | Promise<void>;
}

export interface RegisteredCommand<C> extends Command<C> {
  parsedShortcut?: Shortcut;
}

export class DuplicateCommandError extends Error {}
export class ShortcutConflictError extends Error {}

export class CommandRegistry<C> {
  readonly #commands = new Map<string, RegisteredCommand<C>>();
  readonly #byShortcut = new Map<string, string>();
  readonly #listeners = new Set<() => void>();

  /**
   * Enregistre une commande ; renvoie la fonction qui la retire.
   * Un identifiant ou un raccourci déjà pris est refusé (exception) : un
   * raccourci ne déclenche jamais deux commandes.
   */
  register(command: Command<C>): () => void {
    if (this.#commands.has(command.id)) {
      throw new DuplicateCommandError(`commande déjà enregistrée : ${command.id}`);
    }
    const registered: RegisteredCommand<C> = { ...command };
    if (command.shortcut !== undefined) {
      const parsed = parseShortcut(command.shortcut);
      const key = shortcutId(parsed);
      const owner = this.#byShortcut.get(key);
      if (owner !== undefined) {
        throw new ShortcutConflictError(
          `raccourci ${command.shortcut} déjà utilisé par ${owner} (demandé par ${command.id})`,
        );
      }
      registered.parsedShortcut = parsed;
      this.#byShortcut.set(key, command.id);
    }
    this.#commands.set(command.id, registered);
    this.#notify();
    return () => this.unregister(command.id);
  }

  unregister(id: string): void {
    const command = this.#commands.get(id);
    if (command === undefined) return;
    if (command.parsedShortcut !== undefined) {
      this.#byShortcut.delete(shortcutId(command.parsedShortcut));
    }
    this.#commands.delete(id);
    this.#notify();
  }

  get(id: string): RegisteredCommand<C> | undefined {
    return this.#commands.get(id);
  }

  /** Toutes les commandes, dans l'ordre d'enregistrement. */
  all(): RegisteredCommand<C>[] {
    return [...this.#commands.values()];
  }

  isEnabled(command: Command<C>, ctx: C): boolean {
    return command.when === undefined || command.when(ctx);
  }

  /** Commandes actives dans ce contexte. */
  available(ctx: C): RegisteredCommand<C>[] {
    return this.all().filter((c) => this.isEnabled(c, ctx));
  }

  /** Commandes proposées par la palette, filtrées par une recherche. */
  search(ctx: C, query: string): RegisteredCommand<C>[] {
    const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
    return this.available(ctx)
      .filter((c) => c.inPalette !== false)
      .filter((c) => {
        const haystack = [c.title, c.group, c.id, ...(c.keywords ?? [])].join(" ").toLowerCase();
        return terms.every((t) => haystack.includes(t));
      });
  }

  /**
   * Commande active associée à un raccourci. `inEditable` : la frappe a
   * lieu dans un champ de saisie ; seuls les raccourcis avec Ctrl, Alt ou
   * Méta y sont actifs (sinon taper « ? » dans un champ ouvrirait l'aide).
   */
  resolve(shortcut: Shortcut, ctx: C, inEditable = false): RegisteredCommand<C> | undefined {
    if (inEditable && !hasCommandModifier(shortcut)) return undefined;
    const id = this.#byShortcut.get(shortcutId(shortcut));
    const command = id === undefined ? undefined : this.#commands.get(id);
    return command !== undefined && this.isEnabled(command, ctx) ? command : undefined;
  }

  /** Exécute une commande si elle est active ; renvoie vrai si exécutée. */
  async execute(id: string, ctx: C): Promise<boolean> {
    const command = this.#commands.get(id);
    if (command === undefined || !this.isEnabled(command, ctx)) return false;
    await command.run(ctx);
    return true;
  }

  /** Abonnement aux modifications du registre. */
  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  #notify(): void {
    for (const listener of this.#listeners) listener();
  }
}

/** Regroupe des commandes par `group`, dans l'ordre de première apparition. */
export function groupCommands<T extends { group: string }>(commands: T[]): [string, T[]][] {
  const groups: [string, T[]][] = [];
  for (const command of commands) {
    const existing = groups.find(([name]) => name === command.group);
    if (existing) existing[1].push(command);
    else groups.push([command.group, [command]]);
  }
  return groups;
}
