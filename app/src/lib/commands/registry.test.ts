import { describe, expect, it, vi } from "vitest";
import {
  CommandRegistry,
  DuplicateCommandError,
  ShortcutConflictError,
  type Command,
} from "./registry";
import { formatShortcut, parseShortcut, shortcutFromEvent, ShortcutSyntaxError } from "./shortcut";

interface Ctx {
  selected: number | null;
}

const noop = (): void => {};

function cmd(id: string, extra: Partial<Command<Ctx>> = {}): Command<Ctx> {
  return { id, title: id, group: "Test", run: noop, ...extra };
}

function key(
  k: string,
  mods: Partial<Record<"ctrlKey" | "shiftKey" | "altKey" | "metaKey", boolean>> = {},
) {
  return shortcutFromEvent({
    key: k,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    metaKey: false,
    ...mods,
  });
}

describe("raccourcis", () => {
  it("analyse les modificateurs et normalise les lettres", () => {
    expect(parseShortcut("Ctrl+K")).toEqual({
      key: "k",
      ctrl: true,
      shift: false,
      alt: false,
      meta: false,
    });
    expect(parseShortcut("Mod+k")).toEqual(parseShortcut("ctrl+K"));
    expect(parseShortcut("Shift+I")).toMatchObject({ key: "i", shift: true });
    expect(parseShortcut("Esc").key).toBe("Escape");
    expect(parseShortcut("Ctrl++").key).toBe("+");
  });

  it("ignore Maj pour les caractères qui en dépendent (« ? »)", () => {
    expect(key("?", { shiftKey: true })).toEqual(parseShortcut("?"));
    expect(key("I", { shiftKey: true })).toEqual(parseShortcut("Shift+I"));
    expect(key("i")).not.toEqual(parseShortcut("Shift+I"));
  });

  it("refuse les raccourcis invalides", () => {
    expect(() => parseShortcut("")).toThrow(ShortcutSyntaxError);
    expect(() => parseShortcut("Ctrl+")).toThrow(ShortcutSyntaxError);
    expect(() => parseShortcut("Hyper+K")).toThrow(ShortcutSyntaxError);
    expect(() => parseShortcut("Ctrl+Banane")).toThrow(ShortcutSyntaxError);
  });

  it("formate pour l'affichage", () => {
    expect(formatShortcut(parseShortcut("Ctrl+Shift+K"))).toEqual(["Ctrl", "Maj", "K"]);
    expect(formatShortcut(parseShortcut("?"))).toEqual(["?"]);
    expect(formatShortcut(parseShortcut("Esc"))).toEqual(["Échap"]);
    expect(formatShortcut(parseShortcut("Delete"))).toEqual(["Suppr"]);
    expect(formatShortcut(parseShortcut("Enter"))).toEqual(["Entrée"]);
  });
});

describe("registre de commandes", () => {
  it("enregistre, liste dans l'ordre et retire", () => {
    const r = new CommandRegistry<Ctx>();
    const off = r.register(cmd("a"));
    r.register(cmd("b"));
    expect(r.all().map((c) => c.id)).toEqual(["a", "b"]);
    off();
    expect(r.all().map((c) => c.id)).toEqual(["b"]);
    expect(r.get("a")).toBeUndefined();
  });

  it("refuse un identifiant en double", () => {
    const r = new CommandRegistry<Ctx>();
    r.register(cmd("a"));
    expect(() => r.register(cmd("a"))).toThrow(DuplicateCommandError);
  });

  it("refuse un raccourci déjà pris, même écrit autrement", () => {
    const r = new CommandRegistry<Ctx>();
    r.register(cmd("palette", { shortcut: "Ctrl+K" }));
    expect(() => r.register(cmd("autre", { shortcut: "mod+k" }))).toThrow(ShortcutConflictError);
    // L'échec n'a rien enregistré.
    expect(r.get("autre")).toBeUndefined();
    // Raccourci libéré au retrait.
    r.unregister("palette");
    expect(() => r.register(cmd("autre", { shortcut: "Ctrl+K" }))).not.toThrow();
  });

  it("distingue Maj+I de I", () => {
    const r = new CommandRegistry<Ctx>();
    r.register(cmd("lu", { shortcut: "Shift+I" }));
    expect(() => r.register(cmd("autre", { shortcut: "I" }))).not.toThrow();
  });

  it("respecte les conditions d'activation", async () => {
    const r = new CommandRegistry<Ctx>();
    const run = vi.fn();
    r.register(cmd("remove", { shortcut: "Delete", when: (c) => c.selected !== null, run }));
    r.register(cmd("add"));
    expect(r.available({ selected: null }).map((c) => c.id)).toEqual(["add"]);
    expect(r.resolve(key("Delete"), { selected: null })).toBeUndefined();
    expect(await r.execute("remove", { selected: null })).toBe(false);
    expect(run).not.toHaveBeenCalled();

    expect(r.resolve(key("Delete"), { selected: 3 })?.id).toBe("remove");
    expect(await r.execute("remove", { selected: 3 })).toBe(true);
    expect(run).toHaveBeenCalledWith({ selected: 3 });
  });

  it("n'active que les raccourcis avec modificateur dans un champ de saisie", () => {
    const r = new CommandRegistry<Ctx>();
    r.register(cmd("aide", { shortcut: "?" }));
    r.register(cmd("palette", { shortcut: "Ctrl+K" }));
    const ctx = { selected: null };
    expect(r.resolve(key("?", { shiftKey: true }), ctx, true)).toBeUndefined();
    expect(r.resolve(key("?", { shiftKey: true }), ctx, false)?.id).toBe("aide");
    expect(r.resolve(key("k", { ctrlKey: true }), ctx, true)?.id).toBe("palette");
  });

  it("recherche par titre, groupe et mots-clés, sans les commandes masquées", () => {
    const r = new CommandRegistry<Ctx>();
    r.register(
      cmd("account.add", { title: "Ajouter un compte", group: "Comptes", keywords: ["imap"] }),
    );
    r.register(cmd("help", { title: "Aide des raccourcis", group: "Aide" }));
    r.register(cmd("hidden", { title: "Ajouter caché", group: "Comptes", inPalette: false }));
    const ctx = { selected: null };
    expect(r.search(ctx, "ajouter").map((c) => c.id)).toEqual(["account.add"]);
    expect(r.search(ctx, "comptes imap").map((c) => c.id)).toEqual(["account.add"]);
    expect(r.search(ctx, "").map((c) => c.id)).toEqual(["account.add", "help"]);
  });

  it("notifie les abonnés", () => {
    const r = new CommandRegistry<Ctx>();
    const listener = vi.fn();
    const off = r.subscribe(listener);
    const remove = r.register(cmd("a"));
    remove();
    expect(listener).toHaveBeenCalledTimes(2);
    off();
    r.register(cmd("b"));
    expect(listener).toHaveBeenCalledTimes(2);
  });
});
