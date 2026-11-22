// Alignement de fuzz/Cargo.lock sur le Cargo.lock racine, lancé par
// ./scripts/check.sh. Le fuzzing doit porter sur les versions livrées :
// tout paquet du verrou du fuzzing dont le nom figure dans le verrou racine
// doit y figurer avec la même version, la même source et la même empreinte.
// Les paquets inconnus de la racine (libfuzzer-sys, arbitrary…) sont propres
// au fuzzing ; les paquets sans source sont les crates du dépôt.
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Paquets d'un Cargo.lock : blocs `[[package]]` aux clés `name`, `version`,
// `source` et `checksum` (format écrit par cargo, une clé par ligne).
export function lockPackages(text) {
  const packages = [];
  let current = null;
  for (const line of text.split(/\r?\n/)) {
    if (line === "[[package]]") {
      current = {
        name: undefined,
        version: undefined,
        source: undefined,
        checksum: undefined,
      };
      packages.push(current);
      continue;
    }
    if (line.startsWith("[")) {
      current = null;
      continue;
    }
    const m = /^(name|version|source|checksum) = "([^"]*)"$/.exec(line);
    if (current && m) current[m[1]] = m[2];
  }
  return packages;
}

export function misalignedPackages(rootText, fuzzText) {
  const root = lockPackages(rootText);
  const problems = [];
  for (const p of lockPackages(fuzzText)) {
    if (p.source === undefined) continue;
    const sameName = root.filter((r) => r.name === p.name);
    if (sameName.length === 0) continue;
    const aligned = sameName.some(
      (r) =>
        r.version === p.version &&
        r.source === p.source &&
        r.checksum === p.checksum,
    );
    if (!aligned) {
      problems.push(
        `${p.name} ${p.version} (racine : ${sameName.map((r) => r.version).join(", ")})`,
      );
    }
  }
  return problems;
}

// Problèmes à signaler : un verrou illisible (aucun paquet reconnu, bloc
// incomplet) ou sans paquet commun avec l'autre ferait passer le contrôle à
// tort ; il échoue donc, avant toute comparaison des versions.
export function lockProblems(rootText, fuzzText) {
  const locks = [
    ["Cargo.lock", lockPackages(rootText)],
    ["fuzz/Cargo.lock", lockPackages(fuzzText)],
  ];
  const problems = [];
  for (const [file, packages] of locks) {
    if (packages.length === 0) problems.push(`${file} : aucun paquet lu`);
    else if (packages.some((p) => !p.name || !p.version))
      problems.push(`${file} : paquet sans nom ou sans version`);
  }
  if (problems.length > 0) return problems;
  const rootNames = new Set(locks[0][1].map((p) => p.name));
  if (!locks[1][1].some((p) => p.source !== undefined && rootNames.has(p.name)))
    return ["aucun paquet commun à Cargo.lock et fuzz/Cargo.lock"];
  return misalignedPackages(rootText, fuzzText).map(
    (p) => `fuzz/Cargo.lock : ${p}`,
  );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const read = (file) => readFileSync(path.join(repo, file), "utf8");
  const problems = lockProblems(read("Cargo.lock"), read("fuzz/Cargo.lock"));
  for (const p of problems) console.error(`✗ ${p}`);
  if (problems.length > 0) {
    console.error(
      `\n${problems.length} problème(s) dans le verrou du fuzzing.` +
        "\nResynchroniser : cp Cargo.lock fuzz/Cargo.lock && " +
        "cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 > /dev/null",
    );
    process.exit(1);
  }
  console.log("fuzz/Cargo.lock aligné sur Cargo.lock.");
}
