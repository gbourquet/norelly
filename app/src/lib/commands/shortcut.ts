// Raccourcis clavier : analyse ("Ctrl+K", "Shift+I", "?"), normalisation et
// correspondance avec un événement clavier.

import { m } from "../paraglide/messages.js";

export interface Shortcut {
  /** Touche normalisée : lettre en minuscule, sinon `KeyboardEvent.key`. */
  key: string;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  meta: boolean;
}

export class ShortcutSyntaxError extends Error {}

const NAMED_KEYS: Record<string, string> = {
  esc: "Escape",
  escape: "Escape",
  enter: "Enter",
  space: " ",
  tab: "Tab",
  up: "ArrowUp",
  down: "ArrowDown",
  left: "ArrowLeft",
  right: "ArrowRight",
  delete: "Delete",
  backspace: "Backspace",
};

/** Lettre seule (la majuscule dépend de Maj, normalisée à part). */
function isLetter(key: string): boolean {
  return key.length === 1 && key.toLowerCase() !== key.toUpperCase();
}

/**
 * Normalise une touche : lettres en minuscules ; Maj n'est significatif que
 * pour les lettres et les touches nommées (sur un caractère comme « ? », Maj
 * fait déjà partie du caractère et dépend de la disposition du clavier).
 */
function normalize(s: Shortcut): Shortcut {
  const key = isLetter(s.key) ? s.key.toLowerCase() : s.key;
  const printable = key.length === 1 && !isLetter(key);
  return { ...s, key, shift: printable ? false : s.shift };
}

/** Analyse « Ctrl+Shift+K », « Mod+K » (Ctrl, ou Cmd sous macOS), « ? ». */
export function parseShortcut(spec: string): Shortcut {
  const parts = spec.split("+");
  // « Ctrl++ » : la touche est « + ».
  if (spec.endsWith("++")) {
    parts.splice(parts.length - 2, 2, "+");
  }
  const raw = parts.pop();
  if (raw === undefined || raw === "") {
    throw new ShortcutSyntaxError(`raccourci invalide : « ${spec} »`);
  }
  const s: Shortcut = { key: "", ctrl: false, shift: false, alt: false, meta: false };
  for (const mod of parts) {
    switch (mod.toLowerCase()) {
      case "ctrl":
      case "mod":
        s.ctrl = true;
        break;
      case "shift":
        s.shift = true;
        break;
      case "alt":
        s.alt = true;
        break;
      case "meta":
      case "cmd":
        s.meta = true;
        break;
      default:
        throw new ShortcutSyntaxError(`modificateur inconnu « ${mod} » dans « ${spec} »`);
    }
  }
  const named = NAMED_KEYS[raw.toLowerCase()];
  if (raw.length > 1 && named === undefined && !/^F\d{1,2}$/.test(raw)) {
    throw new ShortcutSyntaxError(`touche inconnue « ${raw} » dans « ${spec} »`);
  }
  s.key = named ?? raw;
  return normalize(s);
}

/** Raccourci d'un événement clavier. */
export function shortcutFromEvent(
  e: Pick<KeyboardEvent, "key" | "ctrlKey" | "shiftKey" | "altKey" | "metaKey">,
): Shortcut {
  return normalize({
    key: e.key,
    ctrl: e.ctrlKey,
    shift: e.shiftKey,
    alt: e.altKey,
    meta: e.metaKey,
  });
}

/** Clé de comparaison stable. */
export function shortcutId(s: Shortcut): string {
  return [
    s.ctrl ? "Ctrl" : "",
    s.alt ? "Alt" : "",
    s.shift ? "Shift" : "",
    s.meta ? "Meta" : "",
    s.key,
  ]
    .filter(Boolean)
    .join("+");
}

/** Forme affichable : « Ctrl+K », « Maj+I », « ? ». */
export function formatShortcut(s: Shortcut): string[] {
  const keys: string[] = [];
  if (s.ctrl) keys.push("Ctrl");
  if (s.alt) keys.push("Alt");
  if (s.shift) keys.push(m.key_shift());
  if (s.meta) keys.push(m.key_meta());
  const label = s.key === " " ? m.key_space() : s.key === "Escape" ? m.key_escape() : s.key;
  keys.push(label.length === 1 ? label.toUpperCase() : label);
  return keys;
}

/** Vrai si le raccourci comporte un modificateur « fort » (utilisable dans un champ de saisie). */
export function hasCommandModifier(s: Shortcut): boolean {
  return s.ctrl || s.alt || s.meta;
}
