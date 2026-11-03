// Mise en forme des dates et tailles (affichage seulement), dans la locale
// active de l'interface (ADR-011) : jamais de locale écrite en dur.

import { activeLocale } from "./i18n";
import { m } from "./paraglide/messages.js";

/** Formateurs mis en cache par locale. */
const cache = new Map<string, Record<"time" | "dayMonth" | "full" | "long", Intl.DateTimeFormat>>();

function formats(locale = activeLocale()) {
  let f = cache.get(locale);
  if (f === undefined) {
    f = {
      time: new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit" }),
      dayMonth: new Intl.DateTimeFormat(locale, { day: "numeric", month: "short" }),
      full: new Intl.DateTimeFormat(locale, { day: "2-digit", month: "2-digit", year: "numeric" }),
      long: new Intl.DateTimeFormat(locale, { dateStyle: "full", timeStyle: "short" }),
    };
    cache.set(locale, f);
  }
  return f;
}

/** Date courte d'une ligne de liste (secondes Unix). */
export function shortDate(seconds: number, now = new Date()): string {
  const d = new Date(seconds * 1000);
  const f = formats();
  if (d.toDateString() === now.toDateString()) return f.time.format(d);
  if (d.getFullYear() === now.getFullYear()) return f.dayMonth.format(d);
  return f.full.format(d);
}

/** Date complète (en-tête d'un message). */
export function longDate(seconds: number): string {
  return formats().long.format(new Date(seconds * 1000));
}

/** Taille lisible : « 12 Ko », « 3,4 Mo » (« 12 KB », « 3.4 MB » en anglais). */
export function fileSize(bytes: number): string {
  if (bytes < 1024) return m.size_bytes({ size: bytes.toLocaleString(activeLocale()) });
  const units = [m.size_kb, m.size_mb, m.size_gb] as const;
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = value < 10 ? 1 : 0;
  const size = value.toLocaleString(activeLocale(), { maximumFractionDigits: digits });
  return (units[unit] ?? m.size_gb)({ size });
}

/** Nom affichable d'un correspondant. */
export function personName(p: { name: string; address: string } | null | undefined): string {
  if (p === null || p === undefined) return m.common_unknown_sender();
  return p.name !== "" ? p.name : p.address;
}

/** Décalage du fuseau local par rapport à UTC, en minutes (pour
 * `before:` et `after:` dans une recherche). */
export function localUtcOffset(now = new Date()): number {
  return -now.getTimezoneOffset();
}
