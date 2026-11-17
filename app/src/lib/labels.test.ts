import { afterEach, describe, expect, it, vi } from "vitest";
import type { LabelDto, LabelMenu } from "./ipc/api";
import { IpcFailure } from "./errors";
import { setLocale } from "./paraglide/runtime.js";
import {
  LabelMenuState,
  LabelSettingsState,
  LabelsState,
  MAX_ROW_CHIPS,
  chipColors,
  labelQuery,
  labelText,
  rowChips,
  selectionChips,
} from "./labels.svelte";

const named = (keyword: string, name: string, color: string | null = null): LabelDto => ({
  keyword,
  name: { kind: "named", name },
  color: color === null ? null : { background: color, text: "#ffffff" },
});

const builtin = (n: number): LabelDto => ({
  keyword: `$label${n}`,
  name: { kind: "builtin", number: n },
  color: null,
});

const raw = (keyword: string): LabelDto => ({
  keyword,
  name: { kind: "raw", keyword },
  color: null,
});

describe("nom d'une étiquette", () => {
  afterEach(() => setLocale("fr", { reload: false }));

  it("étiquettes de Thunderbird traduites, noms décodés, mots-clés illisibles tels quels", () => {
    setLocale("fr", { reload: false });
    expect([1, 2, 3, 4, 5].map((n) => labelText(builtin(n)))).toEqual([
      "Important",
      "Travail",
      "Personnel",
      "À faire",
      "Plus tard",
    ]);
    setLocale("en", { reload: false });
    expect(labelText(builtin(4))).toBe("To Do");
    expect(labelText(named("&AMAAIA-faire", "À faire"))).toBe("À faire");
    expect(labelText(raw("a&b"))).toBe("a&b");
  });
});

describe("pastilles", () => {
  const list = [named("Projet", "Projet"), named("Client", "Client"), builtin(1)];

  it("ligne : au plus deux pastilles, puis le nombre des autres", () => {
    expect(MAX_ROW_CHIPS).toBe(2);
    const chips = rowChips(["Projet", "Client", "$label1"], list);
    expect(chips.shown.map((l) => l.keyword)).toEqual(["Projet", "Client"]);
    expect(chips.hidden.map((l) => l.keyword)).toEqual(["$label1"]);
    expect(rowChips(["Client"], list).hidden).toEqual([]);
  });

  it("un mot-clé absent de la liste du compte s'affiche tel quel", () => {
    expect(rowChips(["Inconnu"], list).shown).toEqual([raw("Inconnu")]);
  });

  it("volet de lecture : toutes les étiquettes des messages, sans doublon, dans l'ordre de la liste", () => {
    expect(
      selectionChips([["Client"], ["Projet", "Client"], []], list).map((l) => l.keyword),
    ).toEqual(["Projet", "Client"]);
  });

  it("couleurs : seulement #rrggbb, sinon pastille neutre", () => {
    expect(chipColors(named("A", "A", "#1565c0"))).toEqual({
      background: "#1565c0",
      text: "#ffffff",
    });
    expect(chipColors(named("A", "A"))).toBeNull();
    const forged: LabelDto = {
      keyword: "A",
      name: { kind: "named", name: "A" },
      color: { background: "red; background-image: url(x)", text: "#000000" },
    };
    expect(chipColors(forged)).toBeNull();
    const badText: LabelDto = {
      keyword: "A",
      name: { kind: "named", name: "A" },
      color: { background: "#000000", text: "#fff" },
    };
    expect(chipColors(badText)).toBeNull();
  });

  it("recherche d'une pastille : opérateur de la langue, nom entre guillemets", () => {
    setLocale("fr", { reload: false });
    expect(labelQuery(named("&AMAAIA-faire", "À faire"))).toBe('étiquette:"À faire"');
    expect(labelQuery(builtin(2))).toBe('étiquette:"Travail"');
    setLocale("en", { reload: false });
    expect(labelQuery(builtin(2))).toBe('label:"Work"');
    // Un nom avec un guillemet se cherche par son mot-clé.
    expect(labelQuery(named("a&ACI-b", 'a"b'))).toBe('label:"a&ACI-b"');
    setLocale("fr", { reload: false });
  });
});

describe("étiquettes des comptes", () => {
  it("chargées par compte, la dernière réponse l'emporte", async () => {
    const api = { listLabels: vi.fn(async () => [named("Projet", "Projet")]) };
    const state = new LabelsState(api);
    expect(state.of(1)).toEqual([]);
    await state.load(1);
    expect(state.of(1).map((l) => l.keyword)).toEqual(["Projet"]);
    expect(state.of(2)).toEqual([]);
    api.listLabels.mockRejectedValueOnce(new IpcFailure("storage"));
    await state.load(1);
    expect(state.of(1).map((l) => l.keyword)).toEqual(["Projet"]);
  });
});

function menu(over: Partial<LabelMenu> = {}): LabelMenu {
  return {
    choices: [
      { label: named("Projet", "Projet"), checked: true, allowed: true },
      { label: named("Client", "Client"), checked: false, allowed: false },
    ],
    can_create: true,
    available: true,
    accounts: [1],
    ...over,
  };
}

function choice(state: LabelMenuState, index: number) {
  const found = state.menu?.choices[index];
  if (found === undefined) throw new Error(`choix ${index}`);
  return found;
}

describe("menu « Étiqueter… »", () => {
  function setup(next: LabelMenu = menu()) {
    const api = {
      labelMenu: vi.fn(async () => next),
      createLabel: vi.fn(async (_a: number, name: string) => named(name, name)),
    };
    const act = vi.fn(async () => {});
    const state = new LabelMenuState(api, act);
    return { api, act, state };
  }

  it("charge le menu de la sélection et pose ou retire une étiquette", async () => {
    const { api, act, state } = setup();
    await state.load([7], { kind: "mailbox", id: 3 });
    expect(api.labelMenu).toHaveBeenCalledWith([7], { kind: "mailbox", id: 3 });
    expect(state.menu?.choices.length).toBe(2);
    await state.toggle(choice(state, 0));
    expect(act).toHaveBeenCalledWith(
      { kind: "set_label", keyword: "Projet", value: false },
      "Étiquette « Projet » retirée.",
    );
    // Menu relu après l'action.
    expect(api.labelMenu).toHaveBeenCalledTimes(2);
  });

  it("une étiquette refusée par le serveur n'est pas posée", async () => {
    const { act, state } = setup();
    await state.load([7], { kind: "mailbox", id: 3 });
    await state.toggle(choice(state, 1));
    expect(act).not.toHaveBeenCalled();
  });

  it("crée une étiquette sur chaque compte de la sélection, puis la pose", async () => {
    const { api, act, state } = setup(menu({ accounts: [1, 2] }));
    await state.load([7, 8], { kind: "unified", role: "inbox" });
    state.newName = "  Urgent ";
    await state.create();
    // Une étiquette du même nom déjà sur un compte est reprise (Rust).
    expect(api.createLabel).toHaveBeenNthCalledWith(1, 1, "Urgent", null, true);
    expect(api.createLabel).toHaveBeenNthCalledWith(2, 2, "Urgent", null, true);
    expect(act).toHaveBeenCalledWith(
      { kind: "set_label", keyword: "Urgent", value: true },
      "Étiquette « Urgent » posée.",
    );
    expect(state.newName).toBe("");
  });

  it("le menu d'une autre sélection ne montre jamais l'ancien pendant son chargement", async () => {
    const { api, state } = setup();
    await state.load([7], { kind: "mailbox", id: 3 });
    expect(state.menu).not.toBeNull();
    let release: (value: LabelMenu) => void = () => {};
    api.labelMenu.mockImplementationOnce(
      () => new Promise<LabelMenu>((resolve) => (release = resolve)),
    );
    const loading = state.load([8], { kind: "mailbox", id: 3 });
    expect(state.menu).toBeNull();
    release(menu({ choices: [] }));
    await loading;
    expect(state.menu?.choices).toEqual([]);
    api.labelMenu.mockRejectedValueOnce(new IpcFailure("storage"));
    await state.load([9], { kind: "mailbox", id: 3 });
    expect(state.menu).toBeNull();
    expect(state.error).toBe("Erreur du stockage local.");
  });

  it("nom refusé : message d'erreur, rien n'est posé", async () => {
    const { api, act, state } = setup();
    api.createLabel.mockRejectedValueOnce(new IpcFailure("label_name_too_long", "label_name"));
    await state.load([7], { kind: "mailbox", id: 3 });
    state.newName = "x";
    await state.create();
    expect(act).not.toHaveBeenCalled();
    expect(state.error).toBe("Ce nom d'étiquette est trop long.");
  });

  it("création impossible ou nom vide : rien n'est envoyé", async () => {
    const { api, state } = setup(menu({ can_create: false }));
    await state.load([7], { kind: "mailbox", id: 3 });
    state.newName = "Urgent";
    await state.create();
    state.newName = "   ";
    await state.create();
    expect(api.createLabel).not.toHaveBeenCalled();
  });
});

describe("réglages des étiquettes", () => {
  function setup() {
    const api = {
      listLabels: vi.fn(async () => [named("Projet", "Projet")]),
      createLabel: vi.fn(async (_a: number, name: string) => named(name, name)),
      setLabelColor: vi.fn(async () => {}),
      renameLabel: vi.fn(async (_a: number, _k: string, name: string) => named(name, name)),
      deleteLabel: vi.fn(async () => {}),
    };
    const labels = new LabelsState(api);
    const state = new LabelSettingsState(api, labels);
    return { api, labels, state };
  }

  it("crée, colore, renomme et supprime, puis relit la liste du compte", async () => {
    const { api, state } = setup();
    state.account = 1;
    state.newName = "Client";
    await state.create();
    expect(api.createLabel).toHaveBeenCalledWith(1, "Client", null, false);
    expect(state.newName).toBe("");
    await state.setColor(named("Projet", "Projet"), "#ABCDEF");
    expect(api.setLabelColor).toHaveBeenCalledWith(1, "Projet", "#abcdef");
    await state.setColor(named("Projet", "Projet"), null);
    expect(api.setLabelColor).toHaveBeenLastCalledWith(1, "Projet", null);
    state.startRename(named("Projet", "Projet"));
    expect(state.renaming).toEqual({ keyword: "Projet", name: "Projet" });
    if (state.renaming === null) throw new Error("renommage");
    state.renaming.name = "Projets";
    await state.rename();
    expect(api.renameLabel).toHaveBeenCalledWith(1, "Projet", "Projets");
    expect(state.renaming).toBeNull();
    state.startDelete(named("Projets", "Projets"));
    if (state.deleting === null) throw new Error("suppression");
    state.deleting.everywhere = true;
    await state.remove();
    expect(api.deleteLabel).toHaveBeenCalledWith(1, "Projets", true);
    expect(state.deleting).toBeNull();
    expect(api.listLabels).toHaveBeenCalledTimes(5);
  });

  it("couleur invalide : rien n'est envoyé", async () => {
    const { api, state } = setup();
    state.account = 1;
    await state.setColor(named("Projet", "Projet"), "rouge");
    expect(api.setLabelColor).not.toHaveBeenCalled();
  });

  it("erreur du domaine affichée", async () => {
    const { api, state } = setup();
    state.account = 1;
    api.renameLabel.mockRejectedValueOnce(new IpcFailure("label_exists", "label_name"));
    state.startRename(named("Projet", "Projet"));
    await state.rename();
    expect(state.error).toBe("Une étiquette porte déjà ce nom.");
    expect(state.renaming).not.toBeNull();
  });
});
