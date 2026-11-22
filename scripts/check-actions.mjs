// Vérificateur des actions GitHub, lancé par scripts/check.sh :
//   - toute action (`uses:`) des workflows et des actions composites du
//     dépôt est figée par une empreinte de commit complète (40 caractères
//     hexadécimaux), suivie de sa version en commentaire (`# v5`) que
//     Dependabot met à jour avec l'empreinte ; une image Docker est figée
//     par son empreinte sha256 ; une action locale (`./…`) existe dans le
//     dépôt ; une forme de `uses:` non reconnue est refusée (échec fermé) ;
//   - Dependabot surveille l'écosystème `github-actions`, pour que la
//     sortie d'une nouvelle version soit proposée d'elle-même ;
//   - avec `--upstream` (réseau, `check.sh full`) : chaque empreinte est
//     celle d'une étiquette du dépôt amont qui correspond à la version du
//     commentaire (ni commit d'un fork, ni version annoncée à tort).
//
// Sans dépendance : Node seul (fonctionne aussi sous Windows, Git Bash).
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, realpathSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Clé `uses` en position de clé YAML (début de ligne, élément de liste,
// mapping en flux) : toute ligne qui en déclare une doit être lisible par
// USES, sinon elle est refusée.
const USES_KEY = /(?:^\s*(?:-\s+)*|[{,]\s*)(["']?)uses\1\s*:/;
const USES = /^\s*(?:-\s+)?(["']?)uses\1\s*:\s*(["']?)([A-Za-z0-9.][^"'\s#]*)\2\s*(#.*)?$/;
const PINNED = /^([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)(?:\/[A-Za-z0-9_./-]+)?@([0-9a-f]{40})$/;
const DOCKER_PINNED = /^docker:\/\/[^@\s]+@sha256:[0-9a-f]{64}$/;
const VERSION_COMMENT = /^#\s*(v?\d+(?:\.\d+)*)(?:\s|$)/;
// dtolnay/rust-toolchain se fige sur la branche d'un canal (pas
// d'étiquette de version) ; Dependabot l'ignore (.github/dependabot.yml).
const CHANNEL_COMMENT = /^#\s*(stable|beta|nightly)(?:\s|$)/;
const CHANNEL_REPO = "dtolnay/rust-toolchain";

/** Déclarations `uses:` d'un fichier : { where, ref, comment } ou { where, unreadable }. */
function usesLines(text, file) {
  const found = [];
  text.split("\n").forEach((raw, i) => {
    const line = raw.replace(/\r$/, "");
    if (/^\s*#/.test(line) || !USES_KEY.test(line)) return;
    const where = `${file}:${i + 1}`;
    const m = line.match(USES);
    if (!m) found.push({ where, unreadable: true });
    else found.push({ where, ref: m[3], comment: m[4] ?? "" });
  });
  return found;
}

/**
 * Actions non figées d'un fichier ; `file` sert aux messages, `exists`
 * dit si un chemin relatif à la racine du dépôt est une action locale.
 */
export function unpinnedActions(text, file, exists = () => true) {
  const problems = [];
  for (const { where, ref, comment, unreadable } of usesLines(text, file)) {
    if (unreadable) {
      problems.push(`${where} : forme de « uses: » non reconnue, à écrire « uses: <action>@<empreinte> # vX »`);
      continue;
    }
    if (ref.startsWith("./")) {
      const local = path.posix.normalize(ref);
      if (local === ".." || local.startsWith("../")) problems.push(`${where} : action locale ${ref} hors du dépôt`);
      else if (!exists(local)) problems.push(`${where} : action locale ${ref} introuvable`);
      continue;
    }
    if (ref.startsWith("docker://")) {
      if (!DOCKER_PINNED.test(ref)) problems.push(`${where} : ${ref} n'est pas figée par une empreinte sha256`);
      continue;
    }
    const pin = ref.match(PINNED);
    const channel = pin?.[1] === CHANNEL_REPO && CHANNEL_COMMENT.test(comment);
    if (!pin) {
      problems.push(`${where} : ${ref} n'est pas figée par une empreinte de 40 caractères`);
    } else if (!VERSION_COMMENT.test(comment) && !channel) {
      problems.push(`${where} : ${ref} sans version en commentaire (# vX)`);
    }
  }
  return problems;
}

/** Actions figées d'un fichier, dépôt amont réduit à `propriétaire/dépôt`. */
export function pinnedActions(text, file) {
  const pins = [];
  for (const { where, ref, comment, unreadable } of usesLines(text, file)) {
    const pin = !unreadable && ref.match(PINNED);
    if (!pin) continue;
    const channel = pin[1] === CHANNEL_REPO && CHANNEL_COMMENT.test(comment);
    const version = (channel ? comment.match(CHANNEL_COMMENT) : comment.match(VERSION_COMMENT))?.[1];
    if (version) pins.push({ where, repo: pin[1], sha: pin[2], version, channel });
  }
  return pins;
}

/**
 * Empreintes qui ne sont celles d'aucune étiquette amont de leur version
 * (`v5` : `v5` ou `v5.x…`). `refs` : dépôt → [{ sha, name }] tel que lu
 * par `git ls-remote --tags` (étiquettes annotées : nom suivi de `^{}`).
 * Un canal suivi (`# stable`) n'a pas d'étiquette : il n'est pas vérifié ici.
 */
export function upstreamProblems(pins, refs) {
  const problems = [];
  for (const { where, repo, sha, version, channel } of pins) {
    if (channel) continue;
    const tagged = (refs.get(repo) ?? []).some(({ sha: s, name }) => {
      const tag = name.replace(/^refs\/tags\//, "").replace(/\^\{\}$/, "");
      return s === sha && (tag === version || tag.startsWith(`${version}.`));
    });
    if (!tagged) {
      problems.push(`${where} : ${repo}@${sha} n'est l'empreinte d'aucune étiquette ${version} du dépôt amont`);
    }
  }
  return problems;
}

/** Bloc (lignes) d'un élément de liste YAML qui commence à `start`. */
function itemBlock(lines, start) {
  const indent = lines[start].search(/-/);
  let end = start + 1;
  while (end < lines.length) {
    const at = lines[end].search(/\S/);
    if (at >= 0 && at <= indent && !/^\s*#/.test(lines[end])) break;
    end++;
  }
  return lines.slice(start, end);
}

/**
 * Problèmes de `.github/dependabot.yml` (`undefined` : fichier absent) ;
 * `localActions` : le dépôt a des actions composites sous `.github/actions`.
 */
export function dependabotProblems(text, { localActions = false } = {}) {
  if (text === undefined) {
    return [".github/dependabot.yml absent : les mises à jour des actions ne sont pas proposées"];
  }
  const problems = [];
  const name = ".github/dependabot.yml";
  if (!/^version:\s*2\s*$/m.test(text)) problems.push(`${name} : « version: 2 » attendu`);
  const lines = text.replace(/\r/g, "").split("\n");
  const start = lines.findIndex((l) => /^\s*-\s+package-ecosystem:\s*["']?github-actions["']?\s*(#.*)?$/.test(l));
  if (start < 0) {
    problems.push(`${name} : aucune surveillance de l'écosystème github-actions`);
    return problems;
  }
  const block = itemBlock(lines, start);
  const entry = block.join("\n");
  const dir = /^\s*(?:directory:\s*|-\s+)["']?\/["']?\s*(#.*)?$/m;
  if (!dir.test(entry)) problems.push(`${name} : l'entrée github-actions doit viser directory: "/"`);
  if (localActions && !/^\s*-\s+["']?\/\.github\/actions\/\*["']?\s*(#.*)?$/m.test(entry)) {
    problems.push(
      `${name} : l'entrée github-actions doit aussi viser "/.github/actions/*" (actions composites locales)`,
    );
  }
  if (!/^\s*interval:\s*["']?\w/m.test(entry)) {
    problems.push(`${name} : l'entrée github-actions n'a pas de rythme (schedule.interval)`);
  }
  if (/^\s*open-pull-requests-limit:\s*["']?0["']?\s*(#.*)?$/m.test(entry)) {
    problems.push(`${name} : open-pull-requests-limit: 0 coupe les propositions de mise à jour`);
  }
  block.forEach((line, i) => {
    if (!/^\s*-\s+dependency-name:\s*["']?\*["']?\s*(#.*)?$/.test(line)) return;
    if (!itemBlock(block, i).some((l) => /^\s*update-types:/.test(l))) {
      problems.push(`${name} : ignore de toutes les actions (dependency-name: "*") sans update-types`);
    }
  });
  return problems;
}

/** Fichiers YAML à contrôler : workflows et actions composites de `.github`, action racine. */
function actionFiles(root) {
  const files = [];
  const walk = (rel) => {
    for (const entry of readdirSync(path.join(root, rel), { withFileTypes: true })) {
      const child = path.posix.join(rel, entry.name);
      if (entry.isDirectory()) walk(child);
      else if (/\.ya?ml$/.test(entry.name) && child !== ".github/dependabot.yml") files.push(child);
    }
  };
  if (existsSync(path.join(root, ".github"))) walk(".github");
  for (const f of ["action.yml", "action.yaml"]) if (existsSync(path.join(root, f))) files.push(f);
  return files.sort();
}

/** Vérification du dépôt ; renvoie la liste des problèmes. */
export function checkRepository(root) {
  const isAction = (rel) => {
    const target = path.join(root, rel);
    if (!existsSync(target)) return false;
    if (statSync(target).isFile()) return /\.ya?ml$/.test(rel);
    return ["action.yml", "action.yaml"].some((f) => existsSync(path.join(target, f)));
  };
  const problems = [];
  for (const f of actionFiles(root)) {
    problems.push(...unpinnedActions(readFileSync(path.join(root, f), "utf8"), f, isAction));
  }
  const config = path.join(root, ".github", "dependabot.yml");
  const localActions = existsSync(path.join(root, ".github", "actions"));
  problems.push(...dependabotProblems(existsSync(config) ? readFileSync(config, "utf8") : undefined, { localActions }));
  return problems;
}

/** Étiquettes amont d'un dépôt GitHub (`git ls-remote --tags`), trois essais. */
function upstreamTags(repo) {
  let error;
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      const out = execFileSync("git", ["ls-remote", "--tags", `https://github.com/${repo}.git`], {
        encoding: "utf8",
        timeout: 30_000,
        env: { ...process.env, GIT_TERMINAL_PROMPT: "0" },
        stdio: ["ignore", "pipe", "pipe"],
      });
      return out
        .split("\n")
        .filter(Boolean)
        .map((l) => {
          const [sha, name] = l.split("\t");
          return { sha, name };
        });
    } catch (e) {
      error = e;
    }
  }
  throw new Error(`étiquettes de ${repo} illisibles : ${error?.message ?? error}`);
}

/** Vérification des empreintes contre les dépôts amont (réseau). */
export function checkUpstream(root) {
  const pins = actionFiles(root).flatMap((f) => pinnedActions(readFileSync(path.join(root, f), "utf8"), f));
  const refs = new Map();
  const problems = [];
  for (const repo of new Set(pins.filter((p) => !p.channel).map((p) => p.repo))) {
    try {
      refs.set(repo, upstreamTags(repo));
    } catch (e) {
      problems.push(e.message);
    }
  }
  return [...problems, ...upstreamProblems(pins, refs)];
}

const self = realpathSync(fileURLToPath(import.meta.url));
if (process.argv[1] && realpathSync(path.resolve(process.argv[1])) === self) {
  const root = path.resolve(path.dirname(self), "..");
  const upstream = process.argv.includes("--upstream");
  const problems = upstream ? checkUpstream(root) : checkRepository(root);
  for (const p of problems) console.error(`✗ ${p}`);
  if (problems.length > 0) {
    console.error(`\n${problems.length} problème(s) dans les actions GitHub.`);
    process.exit(1);
  }
  console.log(
    upstream
      ? "Actions GitHub : chaque empreinte est celle d'une étiquette amont de sa version."
      : "Actions GitHub : figées par empreinte, mises à jour surveillées par Dependabot.",
  );
}
