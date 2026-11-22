// Vérificateur des actions GitHub, lancé par scripts/check.sh :
//   - toute action des workflows (`uses:`) est figée par une empreinte de
//     commit complète (40 caractères hexadécimaux), suivie de sa version en
//     commentaire (`# v5`) que Dependabot met à jour avec l'empreinte ; une
//     image Docker est figée par son empreinte sha256 ; une action locale
//     (`./…`) fait partie du dépôt ;
//   - Dependabot surveille l'écosystème `github-actions`, pour que la
//     sortie d'une nouvelle version soit proposée d'elle-même.
//
// Sans dépendance : Node seul (fonctionne aussi sous Windows, Git Bash).
import { existsSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const USES = /^\s*(?:-\s+)?uses:\s*(["']?)([^"'\s#]+)\1\s*(#.*)?$/;
const PINNED = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_./-]+@[0-9a-f]{40}$/;
const DOCKER_PINNED = /^docker:\/\/[^@\s]+@sha256:[0-9a-f]{64}$/;

/** Actions non figées d'un workflow ; `file` sert aux messages. */
export function unpinnedActions(text, file) {
  const problems = [];
  text.split("\n").forEach((line, i) => {
    const m = line.match(USES);
    if (!m) return;
    const [, , ref, comment] = m;
    const where = `${file}:${i + 1}`;
    if (ref.startsWith("./")) return;
    if (ref.startsWith("docker://")) {
      if (!DOCKER_PINNED.test(ref)) problems.push(`${where} : ${ref} n'est pas figée par une empreinte sha256`);
      return;
    }
    if (!PINNED.test(ref)) {
      problems.push(`${where} : ${ref} n'est pas figée par une empreinte de 40 caractères`);
    } else if (!comment || !/^#\s*\S/.test(comment)) {
      problems.push(`${where} : ${ref} sans version en commentaire (# vX)`);
    }
  });
  return problems;
}

/** Problèmes de `.github/dependabot.yml` (`undefined` : fichier absent). */
export function dependabotProblems(text) {
  if (text === undefined) {
    return [".github/dependabot.yml absent : les mises à jour des actions ne sont pas proposées"];
  }
  const problems = [];
  if (!/^version:\s*2\s*$/m.test(text)) problems.push(".github/dependabot.yml : « version: 2 » attendu");
  if (!/^\s*-?\s*package-ecosystem:\s*["']?github-actions["']?\s*(#.*)?$/m.test(text)) {
    problems.push(".github/dependabot.yml : aucune surveillance de l'écosystème github-actions");
  }
  return problems;
}

/** Vérification du dépôt ; renvoie la liste des problèmes. */
export function checkRepository(root) {
  const workflows = path.join(root, ".github", "workflows");
  const problems = [];
  const files = existsSync(workflows)
    ? readdirSync(workflows).filter((f) => f.endsWith(".yml") || f.endsWith(".yaml"))
    : [];
  for (const f of files.sort()) {
    problems.push(...unpinnedActions(readFileSync(path.join(workflows, f), "utf8"), `.github/workflows/${f}`));
  }
  const config = path.join(root, ".github", "dependabot.yml");
  problems.push(...dependabotProblems(existsSync(config) ? readFileSync(config, "utf8") : undefined));
  return problems;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const problems = checkRepository(root);
  for (const p of problems) console.error(`✗ ${p}`);
  if (problems.length > 0) {
    console.error(`\n${problems.length} problème(s) dans les actions GitHub.`);
    process.exit(1);
  }
  console.log("Actions GitHub : figées par empreinte, mises à jour surveillées par Dependabot.");
}
