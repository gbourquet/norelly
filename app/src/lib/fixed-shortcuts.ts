// Raccourcis propres à la fenêtre de rédaction (envoi, fermeture, mise en
// forme de l'éditeur), hors du registre : affichés dans les infobulles et
// dans l'aide. Les touches de mise en forme sont celles de l'éditeur.

import { m } from "./paraglide/messages.js";
import { tooltip } from "./commands/describe";

export type FormatId = "bold" | "italic" | "strike" | "link" | "bullet" | "ordered" | "quote";

const FORMATS: { id: FormatId; label: () => string; shortcut?: string }[] = [
  { id: "bold", label: m.fmt_bold, shortcut: "Ctrl+B" },
  { id: "italic", label: m.fmt_italic, shortcut: "Ctrl+I" },
  { id: "strike", label: m.fmt_strike, shortcut: "Ctrl+Shift+S" },
  { id: "link", label: m.fmt_link },
  { id: "bullet", label: m.fmt_bullet, shortcut: "Ctrl+Shift+8" },
  { id: "ordered", label: m.fmt_ordered, shortcut: "Ctrl+Shift+7" },
  { id: "quote", label: m.fmt_quote, shortcut: "Ctrl+Shift+B" },
];

/** Touches affichées, sans passer par l'analyse du registre (qui ignore
 * Maj devant un chiffre ou un symbole). */
function keys(spec: string | undefined): string[] {
  if (spec === undefined) return [];
  const names: Record<string, () => string> = {
    Shift: m.key_shift,
    Enter: m.key_enter,
    Escape: m.key_escape,
  };
  return spec.split("+").map((k) => names[k]?.() ?? k);
}

/** Infobulle d'un bouton de mise en forme. */
export function formatTooltip(id: FormatId): string {
  const format = FORMATS.find((f) => f.id === id);
  if (format === undefined) return id;
  return tooltip(format.label(), keys(format.shortcut).join("+"));
}

/** Raccourcis de la fenêtre de rédaction, pour l'aide. */
export function composeWindowShortcuts(): { label: string; keys: string[] }[] {
  return [
    { label: m.help_send(), keys: keys("Ctrl+Enter") },
    { label: m.help_close_compose(), keys: keys("Escape") },
    ...FORMATS.filter((f) => f.shortcut !== undefined).map((f) => ({
      label: f.label(),
      keys: keys(f.shortcut),
    })),
  ];
}
