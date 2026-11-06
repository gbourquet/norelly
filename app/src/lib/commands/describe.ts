// Description d'une commande pour un élément visible (bouton, entrée de
// menu) : libellé, touches, infobulle « Libellé — Touche » et
// disponibilité. Tout vient du registre (ADR-005).

import { m } from "../paraglide/messages.js";
import type { CommandRegistry } from "./registry";
import { formatShortcut } from "./shortcut";

export interface CommandDescription {
  label: string;
  /** Raccourci affichable (« Maj+I ») ; vide s'il n'y en a pas. */
  keys: string;
  tooltip: string;
  enabled: boolean;
}

/** Infobulle « Libellé — Touches » ; le libellé seul sans raccourci. */
export function tooltip(label: string, keys: string): string {
  return keys === "" ? label : m.tooltip_shortcut({ label, keys });
}

/** `label` remplace le titre de la commande (bascule selon l'état). */
export function describeCommand<C>(
  registry: CommandRegistry<C>,
  id: string,
  ctx: C,
  label?: string,
): CommandDescription | null {
  const command = registry.get(id);
  if (command === undefined) return null;
  const text = label ?? command.title;
  const keys = command.parsedShortcut ? formatShortcut(command.parsedShortcut).join("+") : "";
  return {
    label: text,
    keys,
    tooltip: tooltip(text, keys),
    enabled: registry.isEnabled(command, ctx),
  };
}
