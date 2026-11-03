import { describe, expect, it } from "vitest";
import fr from "../../messages/fr.json";
import en from "../../messages/en.json";
import { activeLocale, applySystemLocale, pickLocale } from "./i18n";

// Test exécuté par ./scripts/check.sh (via `pnpm -C app test`) : une chaîne
// d'interface = une clé dans chaque catalogue (ADR-011).

type Catalog = Record<string, string>;
const catalogs: Record<string, Catalog> = { fr, en };

function messages(catalog: Catalog): [string, string][] {
  return Object.entries(catalog).filter(([k]) => k !== "$schema");
}

/** Paramètres `{nom}` de chaque message, triés. */
function params(catalog: Catalog): Record<string, string[]> {
  return Object.fromEntries(
    messages(catalog).map(([k, v]) => [
      k,
      [...v.matchAll(/\{(\w+)\}/g)].map((x) => x[1] ?? "").sort(),
    ]),
  );
}

describe("catalogues de traduction", () => {
  it("fr.json et en.json ont exactement les mêmes clés", () => {
    const keys = (c: Catalog) =>
      messages(c)
        .map(([k]) => k)
        .sort();
    expect(keys(en)).toEqual(keys(fr));
  });

  it("chaque message a les mêmes paramètres dans les deux langues", () => {
    expect(params(en)).toEqual(params(fr));
  });

  it("aucun message n'est vide", () => {
    for (const [locale, catalog] of Object.entries(catalogs)) {
      for (const [k, v] of messages(catalog)) expect(v.trim(), `${locale}:${k}`).not.toBe("");
    }
  });
});

describe("choix de la langue", () => {
  it("retient la première langue disponible", () => {
    expect(pickLocale(["fr-FR", "en-US"])).toBe("fr");
    expect(pickLocale(["de-DE", "en-GB", "fr"])).toBe("en");
    expect(pickLocale(["fr_CA"])).toBe("fr");
  });

  it("se replie sur l'anglais pour toute autre langue", () => {
    expect(pickLocale(["de-DE", "ja"])).toBe("en");
    expect(pickLocale([])).toBe("en");
  });
});

describe("langue du système", () => {
  it("applique la première langue disponible et marque le document", () => {
    expect(applySystemLocale({ languages: ["en-US", "fr"], language: "fr" })).toBe("en");
    expect(activeLocale()).toBe("en");
    expect(document.documentElement.lang).toBe("en");
    // Liste vide : langue principale du navigateur.
    expect(applySystemLocale({ languages: [], language: "fr-FR" })).toBe("fr");
    expect(activeLocale()).toBe("fr");
    expect(document.documentElement.lang).toBe("fr");
  });
});
