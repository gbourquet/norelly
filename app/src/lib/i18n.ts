// Langue de l'interface (ADR-011) : la langue du système par défaut ; le
// français est la langue source, l'anglais le repli pour toute autre
// locale. Aucune décision de sécurité ici.

import { locales, setLocale, getLocale } from "./paraglide/runtime.js";

export type Locale = (typeof locales)[number];

/** Locale de l'interface pour une liste de langues préférées (BCP 47). */
export function pickLocale(preferred: readonly string[]): Locale {
  for (const tag of preferred) {
    const base = tag.toLowerCase().split(/[-_]/)[0];
    const found = locales.find((l) => l === base);
    if (found !== undefined) return found;
  }
  return "en";
}

/** Applique la langue du système (au démarrage). */
export function applySystemLocale(nav: Pick<Navigator, "languages" | "language">): Locale {
  const preferred = nav.languages.length > 0 ? nav.languages : [nav.language];
  const locale = pickLocale(preferred);
  setLocale(locale, { reload: false });
  document.documentElement.lang = locale;
  return locale;
}

/** Locale active, pour `Intl`. */
export function activeLocale(): Locale {
  return getLocale();
}

/** Applique la langue choisie dans les paramètres (`system` : celle du
 * système). */
export function applyLanguage(
  language: "system" | "fr" | "en",
  nav: Pick<Navigator, "languages" | "language">,
): Locale {
  if (language === "system") return applySystemLocale(nav);
  setLocale(language, { reload: false });
  document.documentElement.lang = language;
  return language;
}
