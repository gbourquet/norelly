// Tests du vérificateur de documentation (scripts/check-docs.mjs), lancés
// par scripts/check-docs.sh avec `node --test`, sur des arborescences
// temporaires.
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  brokenLinks,
  captureProblems,
  parityProblems,
  markdownLinks,
  technicalProblems,
} from "./check-docs.mjs";

/** Arborescence temporaire : { "chemin/relatif": contenu }. */
function tree(files) {
  const root = mkdtempSync(path.join(tmpdir(), "check-docs-"));
  for (const [rel, content] of Object.entries(files)) {
    const file = path.join(root, rel);
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(file, content);
  }
  return root;
}

const PNG = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);

test("extrait les liens et les images, hors blocs de code", () => {
  const md = [
    "Voir [le plan](../PLAN.md#phase-11) et ![accueil](captures/welcome.png).",
    "Externe : [site](https://example.com), [courriel](mailto:a@b.c), [ancre](#haut).",
    "```",
    "[pas un lien](absent.md)",
    "```",
    "Code `[pas non plus](absent2.md)` en ligne.",
  ].join("\n");
  assert.deepEqual(
    markdownLinks(md).map((l) => [l.target, l.image]),
    [
      ["../PLAN.md#phase-11", false],
      ["captures/welcome.png", true],
      ["https://example.com", false],
      ["mailto:a@b.c", false],
      ["#haut", false],
    ],
  );
});

test("signale les liens internes vers un fichier absent", () => {
  const root = tree({
    "docs/a.md": "[b](b.md) [c](sous/c.md#titre) [x](x.md) [site](https://example.com) [ici](#haut)",
    "docs/b.md": "[retour](./a.md) [dossier](sous/)",
    "docs/sous/c.md": "[haut](../../README.md)",
    "README.md": "",
  });
  const problems = brokenLinks(root, ["docs/a.md", "docs/b.md", "docs/sous/c.md", "README.md"]);
  assert.deepEqual(problems, ["docs/a.md : lien vers x.md introuvable"]);
});

test("refuse un lien qui sort du dépôt", () => {
  const root = tree({ "a.md": "[dehors](../../etc/passwd)" });
  assert.deepEqual(brokenLinks(root, ["a.md"]), [
    "a.md : lien vers ../../etc/passwd hors du dépôt",
  ]);
});

test("exige les mêmes pages et les mêmes captures en français et en anglais", () => {
  const root = tree({
    "fr/README.md": "",
    "fr/install.md": "",
    "fr/captures/welcome.png": PNG,
    "fr/captures/inbox.png": PNG,
    "en/README.md": "",
    "en/faq.md": "",
    "en/captures/welcome.png": PNG,
  });
  assert.deepEqual(parityProblems(path.join(root, "fr"), path.join(root, "en")), [
    "page absente en anglais : install.md",
    "page absente en français : faq.md",
    "capture absente en anglais : captures/inbox.png",
  ]);
});

test("exige des captures présentes, au format PNG, et toutes utilisées", () => {
  const root = tree({
    "fr/README.md": "![a](captures/a.png) ![b](captures/b.png) ![c](captures/c.png)",
    "fr/captures/a.png": PNG,
    "fr/captures/b.png": "pas une image",
    "fr/captures/orpheline.png": PNG,
  });
  assert.deepEqual(captureProblems(path.join(root, "fr")), [
    "README.md : capture captures/b.png illisible (PNG attendu)",
    "README.md : capture captures/c.png absente",
    "capture jamais utilisée : captures/orpheline.png",
  ]);
});

test("exige au moins une capture par documentation utilisateur", () => {
  const root = tree({ "fr/README.md": "Pas d'image." });
  assert.deepEqual(captureProblems(path.join(root, "fr")), ["aucune capture d'écran"]);
});

test("refuse le vocabulaire technique dans la documentation fonctionnelle", () => {
  const root = tree({
    "fonctionnel/a.md": [
      "# Comptes",
      "Le serveur est trouvé à partir du domaine de l'adresse, en TLS ou STARTTLS.",
      "Sécurité d'abord, décidée en Rust (ADR-001).",
      "Règle portée par `crates/domain/src/address.rs`.",
      "Liste blanche dans app/src/lib/agent.ts, revalidée par l'IPC.",
      "Voir [la traçabilité](../technique/tracabilite.md).",
    ].join("\n"),
    "fonctionnel/b.md": "Recherche hors ligne, tolérante à la casse.",
  });
  assert.deepEqual(technicalProblems(path.join(root, "fonctionnel")), [
    "a.md:3 : « Rust »",
    "a.md:3 : « ADR-001 »",
    "a.md:4 : « crates/ »",
    "a.md:4 : « address.rs »",
    "a.md:5 : « app/src/ »",
    "a.md:5 : « agent.ts »",
    "a.md:5 : « IPC »",
  ]);
});
