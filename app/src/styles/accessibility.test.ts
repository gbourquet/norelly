import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

// Passe d'accessibilité : contrastes des jetons de couleur
// (WCAG 2.1 AA) dans les deux thèmes, et focus visible partout.

const css = readFileSync(join(__dirname, "tokens.css"), "utf8");

function block(source: string): Record<string, string> {
  return Object.fromEntries(
    [...source.matchAll(/--(color-[\w-]+):\s*(#[0-9a-f]{6})\s*;/gi)].map((m) => [
      m[1] ?? "",
      (m[2] ?? "").toLowerCase(),
    ]),
  );
}

const light = block(css.slice(0, css.indexOf("@media (prefers-color-scheme: dark)")));
const dark = {
  ...light,
  ...block(
    css.slice(
      css.indexOf("@media (prefers-color-scheme: dark)"),
      css.indexOf("@media (prefers-reduced-motion"),
    ),
  ),
};

function luminance(hex: string): number {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  const [r, g, b] = c.map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)) as [
    number,
    number,
    number,
  ];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/** Texte, fond, contraste minimal. */
const PAIRS: [string, string, number][] = [
  ["color-text", "color-bg", 4.5],
  ["color-text", "color-surface", 4.5],
  ["color-text", "color-hover", 4.5],
  ["color-text", "color-selected", 4.5],
  ["color-text-muted", "color-bg", 4.5],
  ["color-text-muted", "color-surface", 4.5],
  ["color-text-muted", "color-hover", 4.5],
  ["color-text-muted", "color-selected", 4.5],
  ["color-text-on-accent", "color-accent", 4.5],
  ["color-accent", "color-bg", 4.5],
  ["color-danger", "color-bg", 4.5],
  ["color-danger", "color-danger-bg", 4.5],
  ["color-warning", "color-warning-bg", 4.5],
  ["color-success", "color-bg", 4.5],
  // Indicateur de focus (élément non textuel).
  ["color-focus", "color-bg", 3],
  ["color-focus", "color-surface", 3],
  ["color-focus", "color-selected", 3],
];

describe("contrastes (WCAG AA)", () => {
  for (const [name, theme] of [
    ["clair", light],
    ["sombre", dark],
  ] as const) {
    it(`thème ${name}`, () => {
      const failures = PAIRS.filter(([fg, bg, min]) => {
        const a = theme[fg];
        const b = theme[bg];
        return a === undefined || b === undefined || contrast(a, b) < min;
      }).map(
        ([fg, bg, min]) =>
          `${fg} sur ${bg} : ${contrast(theme[fg] ?? "#000000", theme[bg] ?? "#000000").toFixed(2)} < ${min}`,
      );
      expect(failures).toEqual([]);
    });
  }
});

describe("focus visible", () => {
  it("un composant qui retire le contour fournit sa propre règle :focus-visible", () => {
    const dir = join(__dirname, "..", "components");
    const offenders = readdirSync(dir)
      .filter((f) => f.endsWith(".svelte"))
      .filter((f) => {
        const source = readFileSync(join(dir, f), "utf8");
        return /outline:\s*none/.test(source) && !/:focus-visible/.test(source);
      });
    expect(offenders).toEqual([]);
  });
});
