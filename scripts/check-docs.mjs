// Vérificateur de documentation (phase 11), lancé par scripts/check-docs.sh :
//   - liens internes valides dans tous les fichiers Markdown suivis par git ;
//   - documentation utilisateur : mêmes pages et mêmes captures en
//     français et en anglais ;
//   - captures présentes, au format PNG, toutes utilisées.
// La présence de chaque commande du registre dans l'aide-mémoire des
// raccourcis est vérifiée par app/src/docs.test.ts (registre réel).
//
// Sans dépendance : Node seul (fonctionne aussi sous Windows, Git Bash).
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** Liens et images d'un texte Markdown, hors blocs et extraits de code. */
export function markdownLinks(markdown) {
  const links = [];
  let fenced = false;
  for (const line of markdown.split("\n")) {
    if (/^\s*(```|~~~)/.test(line)) {
      fenced = !fenced;
      continue;
    }
    if (fenced) continue;
    const text = line.replace(/`[^`]*`/g, "");
    for (const match of text.matchAll(/(!?)\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)/g)) {
      links.push({ target: match[2], image: match[1] === "!" });
    }
  }
  return links;
}

/** Lien vers un fichier du dépôt (ni adresse externe, ni ancre seule). */
function isInternal(target) {
  return !/^[a-z][a-z0-9+.-]*:/i.test(target) && !target.startsWith("#");
}

/** Chemin, relatif à `root`, désigné par un lien interne de `file`. */
function resolveTarget(root, file, target) {
  const clean = decodeURIComponent(target.split("#")[0].split("?")[0]);
  return path.relative(root, path.resolve(root, path.dirname(file), clean));
}

/** Liens internes cassés des fichiers `files` (chemins relatifs à `root`). */
export function brokenLinks(root, files) {
  const problems = [];
  for (const file of files) {
    const markdown = readFileSync(path.join(root, file), "utf8");
    for (const { target } of markdownLinks(markdown)) {
      if (!isInternal(target)) continue;
      const rel = resolveTarget(root, file, target);
      if (rel.startsWith("..") || path.isAbsolute(rel)) {
        problems.push(`${file} : lien vers ${target} hors du dépôt`);
      } else if (!existsSync(path.join(root, rel))) {
        problems.push(`${file} : lien vers ${target} introuvable`);
      }
    }
  }
  return problems;
}

/** Fichiers d'un dossier, récursivement, en chemins relatifs (séparateur « / »). */
function listFiles(dir, prefix = "") {
  if (!existsSync(dir)) return [];
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) out.push(...listFiles(path.join(dir, entry.name), rel));
    else out.push(rel);
  }
  return out.sort();
}

const isPage = (f) => f.endsWith(".md");
const isCapture = (f) => f.startsWith("captures/");

/** Pages et captures présentes dans une langue et pas dans l'autre. */
export function parityProblems(frDir, enDir) {
  const fr = listFiles(frDir);
  const en = listFiles(enDir);
  const problems = [];
  for (const [kind, keep] of [
    ["page", isPage],
    ["capture", isCapture],
  ]) {
    const a = fr.filter(keep);
    const b = en.filter(keep);
    for (const f of a.filter((f) => !b.includes(f))) problems.push(`${kind} absente en anglais : ${f}`);
    for (const f of b.filter((f) => !a.includes(f))) problems.push(`${kind} absente en français : ${f}`);
  }
  return problems;
}

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

function isPng(file) {
  const bytes = readFileSync(file);
  return PNG_SIGNATURE.every((b, i) => bytes[i] === b);
}

/** Captures d'une documentation utilisateur : présentes, PNG, utilisées. */
export function captureProblems(dir) {
  const problems = [];
  const used = new Set();
  for (const page of listFiles(dir).filter(isPage)) {
    const markdown = readFileSync(path.join(dir, page), "utf8");
    for (const { target, image } of markdownLinks(markdown)) {
      if (!image || !isInternal(target)) continue;
      const rel = resolveTarget(dir, page, target).split(path.sep).join("/");
      used.add(rel);
      const file = path.join(dir, rel);
      if (!existsSync(file) || !statSync(file).isFile()) {
        problems.push(`${page} : capture ${target} absente`);
      } else if (!isPng(file)) {
        problems.push(`${page} : capture ${target} illisible (PNG attendu)`);
      }
    }
  }
  for (const capture of listFiles(dir).filter(isCapture)) {
    if (!used.has(capture)) problems.push(`capture jamais utilisée : ${capture}`);
  }
  if (used.size === 0) problems.push("aucune capture d'écran");
  return problems;
}

/** Vérification du dépôt ; renvoie la liste des problèmes. */
export function checkRepository(root) {
  const tracked = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard"], {
    cwd: root,
    encoding: "utf8",
  })
    .split("\n")
    .filter((f) => f.endsWith(".md") && !f.includes("node_modules/") && existsSync(path.join(root, f)));
  const user = path.join(root, "docs", "utilisateur");
  const problems = [...brokenLinks(root, tracked)];
  problems.push(...parityProblems(path.join(user, "fr"), path.join(user, "en")));
  for (const lang of ["fr", "en"]) {
    problems.push(...captureProblems(path.join(user, lang)).map((p) => `utilisateur/${lang} : ${p}`));
  }
  return problems;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const problems = checkRepository(root);
  for (const p of problems) console.error(`✗ ${p}`);
  if (problems.length > 0) {
    console.error(`\n${problems.length} problème(s) dans la documentation.`);
    process.exit(1);
  }
  console.log("Documentation : liens, pages et captures en ordre.");
}
