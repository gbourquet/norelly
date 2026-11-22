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

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const read = (file) => readFileSync(path.join(repo, file), "utf8");
  const problems = misalignedPackages(
    read("Cargo.lock"),
    read("fuzz/Cargo.lock"),
  );
  for (const p of problems) console.error(`✗ fuzz/Cargo.lock : ${p}`);
  if (problems.length > 0) {
    console.error(
      `\n${problems.length} paquet(s) du fuzzing désaligné(s) sur Cargo.lock.` +
        "\nResynchroniser : cp Cargo.lock fuzz/Cargo.lock && " +
        "cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 > /dev/null",
    );
    process.exit(1);
  }
  console.log("fuzz/Cargo.lock aligné sur Cargo.lock.");
}
