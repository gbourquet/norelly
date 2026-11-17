// Étiquettes (runes Svelte 5) : liste d'un compte, pastilles, menu
// « Étiqueter… » et réglages. Les noms, les encodages, les cases cochées,
// les refus du serveur et les couleurs (texte lisible compris) sont
// décidés en Rust ; ici, seulement l'affichage et les appels.

import type { ActionDto, Api, LabelChoice, LabelDto, LabelMenu, ScopeDto } from "./ipc/api";
import { errorText } from "./errors";
import { m } from "./paraglide/messages.js";

/** Pastilles au plus dans une ligne de la liste, puis « +N ». */
export const MAX_ROW_CHIPS = 2;

const BUILTIN: Record<number, () => string> = {
  1: m.label_builtin_1,
  2: m.label_builtin_2,
  3: m.label_builtin_3,
  4: m.label_builtin_4,
  5: m.label_builtin_5,
};

/** Nom affiché : étiquette de Thunderbird traduite, nom décodé par Rust,
 * ou mot-clé illisible tel quel (toujours affiché en texte échappé). */
export function labelText(label: LabelDto): string {
  switch (label.name.kind) {
    case "builtin":
      return BUILTIN[label.name.number]?.() ?? label.keyword;
    case "named":
      return label.name.name;
    case "raw":
      return label.name.keyword;
  }
}

/** Étiquette d'un mot-clé absent de la liste du compte : tel quel. */
function unknownLabel(keyword: string): LabelDto {
  return { keyword, name: { kind: "raw", keyword }, color: null };
}

function labelOf(keyword: string, labels: LabelDto[]): LabelDto {
  return labels.find((l) => l.keyword === keyword) ?? unknownLabel(keyword);
}

/** Pastilles d'une ligne : les `max` premières, puis celles du « +N ». */
export function rowChips(
  keywords: string[],
  labels: LabelDto[],
  max = MAX_ROW_CHIPS,
): { shown: LabelDto[]; hidden: LabelDto[] } {
  const all = keywords.map((k) => labelOf(k, labels));
  return { shown: all.slice(0, max), hidden: all.slice(max) };
}

/** Toutes les étiquettes des messages d'une conversation (volet de
 * lecture), sans doublon, dans l'ordre de la liste du compte. */
export function selectionChips(lists: string[][], labels: LabelDto[]): LabelDto[] {
  const keywords = lists.flat().filter((k, i, all) => all.indexOf(k) === i);
  const known = labels.filter((l) => keywords.includes(l.keyword));
  const unknown = keywords.filter((k) => !labels.some((l) => l.keyword === k)).map(unknownLabel);
  return [...known, ...unknown];
}

/** Couleur CSS acceptée : `#rrggbb` seulement (Rust la valide déjà ;
 * contrôle redoublé avant tout style). */
const HEX = /^#[0-9a-f]{6}$/;

/** Couleurs d'une pastille, ou `null` (pastille neutre, jetons du thème). */
export function chipColors(label: LabelDto): { background: string; text: string } | null {
  const c = label.color;
  if (c === null || !HEX.test(c.background) || !HEX.test(c.text)) return null;
  return { background: c.background, text: c.text };
}

/** Recherche lancée par un clic sur une pastille : `étiquette:"nom"` (ou
 * `label:` en anglais) ; un nom qui contient un guillemet se cherche par
 * son mot-clé (Rust reconnaît les deux). */
export function labelQuery(label: LabelDto): string {
  const name = labelText(label);
  const value = name.includes('"') ? label.keyword : name;
  return `${m.label_search_operator()}:"${value}"`;
}

/** Étiquettes de chaque compte, relues après une action ou une
 * synchronisation. */
export class LabelsState {
  readonly #api: Pick<Api, "listLabels">;
  byAccount = $state<Record<number, LabelDto[]>>({});
  #reads: Record<number, number> = {};

  constructor(api: Pick<Api, "listLabels">) {
    this.#api = api;
  }

  of(account: number): LabelDto[] {
    return this.byAccount[account] ?? [];
  }

  /** Relit les étiquettes d'un compte ; en cas d'échec, l'ancienne liste
   * reste affichée. */
  async load(account: number): Promise<void> {
    const read = (this.#reads[account] ?? 0) + 1;
    this.#reads[account] = read;
    try {
      const labels = await this.#api.listLabels(account);
      if (this.#reads[account] === read) this.byAccount[account] = labels;
    } catch {
      // Liste précédente gardée : une pastille sans nom n'apparaît pas.
    }
  }
}

/** Applique une action aux conversations visées (`MailState.act`). */
export type Act = (action: ActionDto, done: string) => Promise<void>;

/** Dialogue « Étiqueter… » : menu de la sélection relu après chaque
 * changement, création d'une étiquette posée tout de suite. */
export class LabelMenuState {
  readonly #api: Pick<Api, "labelMenu" | "createLabel">;
  readonly #act: Act;
  menu = $state<LabelMenu | null>(null);
  newName = $state("");
  error = $state<string | null>(null);
  busy = $state(false);
  #conversations: number[] = [];
  #scope: ScopeDto | null = null;

  constructor(api: Pick<Api, "labelMenu" | "createLabel">, act: Act) {
    this.#api = api;
    this.#act = act;
  }

  async load(conversations: number[], scope: ScopeDto): Promise<void> {
    this.#conversations = conversations;
    this.#scope = scope;
    this.error = null;
    this.newName = "";
    // Jamais le menu d'une autre sélection pendant le chargement.
    this.menu = null;
    await this.#reload();
  }

  #generation = 0;

  async #reload(): Promise<void> {
    if (this.#scope === null) return;
    const generation = ++this.#generation;
    try {
      const menu = await this.#api.labelMenu(this.#conversations, this.#scope);
      if (generation === this.#generation) this.menu = menu;
    } catch (e) {
      if (generation !== this.#generation) return;
      this.menu = null;
      this.error = errorText(e);
    }
  }

  /** Coche (pose) ou décoche (retire) une étiquette ; une étiquette
   * refusée par le serveur n'est pas posée. */
  async toggle(choice: LabelChoice): Promise<void> {
    if (!choice.allowed || this.busy) return;
    const value = !choice.checked;
    const label = labelText(choice.label);
    this.busy = true;
    try {
      await this.#act(
        { kind: "set_label", keyword: choice.label.keyword, value },
        value ? m.label_applied({ label }) : m.label_removed({ label }),
      );
      await this.#reload();
    } finally {
      this.busy = false;
    }
  }

  /** Crée l'étiquette `newName` sur chaque compte de la sélection (Rust
   * reprend celle qui porte déjà ce nom), puis la pose. */
  async create(): Promise<void> {
    const name = this.newName.trim();
    if (name === "" || this.menu === null || !this.menu.can_create || this.busy) return;
    this.busy = true;
    this.error = null;
    try {
      let created: LabelDto | null = null;
      for (const account of this.menu.accounts) {
        const label = await this.#api.createLabel(account, name, null, true);
        created ??= label;
      }
      if (created === null) return;
      await this.#act(
        { kind: "set_label", keyword: created.keyword, value: true },
        m.label_applied({ label: labelText(created) }),
      );
      this.newName = "";
      await this.#reload();
    } catch (e) {
      this.error = errorText(e);
    } finally {
      this.busy = false;
    }
  }
}

/** Réglages des étiquettes d'un compte : création, couleur, renommage,
 * suppression ; la liste du compte est relue après chaque changement. */
export class LabelSettingsState {
  readonly #api: Pick<Api, "createLabel" | "setLabelColor" | "renameLabel" | "deleteLabel">;
  readonly #labels: LabelsState;
  account = $state<number | null>(null);
  newName = $state("");
  error = $state<string | null>(null);
  notice = $state<string | null>(null);
  busy = $state(false);
  renaming = $state<{ keyword: string; name: string } | null>(null);
  deleting = $state<{ keyword: string; label: string; everywhere: boolean } | null>(null);

  constructor(
    api: Pick<Api, "createLabel" | "setLabelColor" | "renameLabel" | "deleteLabel">,
    labels: LabelsState,
  ) {
    this.#api = api;
    this.#labels = labels;
  }

  /** Exécute un changement, puis relit la liste du compte. */
  async #change(run: (account: number) => Promise<void>): Promise<boolean> {
    const account = this.account;
    if (account === null || this.busy) return false;
    this.busy = true;
    this.error = null;
    this.notice = null;
    try {
      await run(account);
      return true;
    } catch (e) {
      this.error = errorText(e);
      return false;
    } finally {
      await this.#labels.load(account);
      this.busy = false;
    }
  }

  async create(): Promise<void> {
    const name = this.newName.trim();
    if (name === "") return;
    if (await this.#change(async (a) => void (await this.#api.createLabel(a, name, null, false)))) {
      this.newName = "";
    }
  }

  /** Couleur choisie (`<input type="color">` : `#rrggbb`) ; `null` : sans
   * couleur. Une autre valeur n'est pas envoyée. */
  async setColor(label: LabelDto, color: string | null): Promise<void> {
    const value = color === null ? null : color.toLowerCase();
    if (value !== null && !HEX.test(value)) return;
    await this.#change((a) => this.#api.setLabelColor(a, label.keyword, value));
  }

  startRename(label: LabelDto): void {
    this.deleting = null;
    this.renaming = { keyword: label.keyword, name: labelText(label) };
  }

  async rename(): Promise<void> {
    const renaming = this.renaming;
    if (renaming === null) return;
    const done = await this.#change(
      async (a) => void (await this.#api.renameLabel(a, renaming.keyword, renaming.name)),
    );
    if (done) {
      this.renaming = null;
      this.notice = m.settings_label_renamed();
    }
  }

  startDelete(label: LabelDto): void {
    this.renaming = null;
    this.deleting = { keyword: label.keyword, label: labelText(label), everywhere: false };
  }

  async remove(): Promise<void> {
    const deleting = this.deleting;
    if (deleting === null) return;
    const done = await this.#change((a) =>
      this.#api.deleteLabel(a, deleting.keyword, deleting.everywhere),
    );
    if (done) this.deleting = null;
  }
}
