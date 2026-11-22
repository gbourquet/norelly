// Versions figées de la chaîne de compilation, lancé par scripts/check.sh :
//   - rust-toolchain.toml (seul fichier de chaîne du dépôt) donne une
//     version exacte de Rust (X.Y.Z) et ses composants ;
//   - chaque étape `dtolnay/rust-toolchain` des workflows installe cette
//     version et ces composants (entrée `with:`), ou un nightly daté
//     (nightly-AAAA-MM-JJ) utilisé par un `cargo +…` du même job ; aucune
//     autre action d'installation de Rust ;
//   - `cargo +<chaîne>` n'utilise qu'une chaîne installée par son job ;
//     `RUSTUP_TOOLCHAIN` et `rustup` ne nomment qu'une chaîne figée ;
//   - chaque outil de `taiki-e/install-action` est figé en outil@X.Y.Z,
//     avec un repli vérifié (`fallback: none` ou `cargo-install`) ;
//     `cargo install` et `cargo binstall` donnent une version exacte ;
//   - l'image e2e (app/e2e/Dockerfile) installe la même version et les
//     mêmes composants.
// Détection ligne à ligne (lignes continuées par « \ » réunies), sans
// analyseur YAML : un garde-fou contre les oublis, pas contre une
// modification volontaire, qui passe de toute façon par la revue.
// Procédure de montée de version : docs/technique/methode.md.
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { isEntryPoint } from "./entry-point.mjs";

const EXACT = /^\d+\.\d+\.\d+$/;
const DATED_NIGHTLY = /^nightly-\d{4}-\d{2}-\d{2}$/;
const VERIFIED_FALLBACKS = ["none", "cargo-install"];

/** Fichiers de chaîne autres que rust-toolchain.toml à la racine. */
export function toolchainFileProblems(files) {
  return files
    .filter((f) => /^rust-toolchain(\.toml)?$/.test(path.posix.basename(f)))
    .filter((f) => f !== "rust-toolchain.toml")
    .map((f) => `${f} : fichier de chaîne en plus de rust-toolchain.toml (racine)`);
}

/** Version et composants de rust-toolchain.toml. */
export function parseToolchain(text) {
  const channel = text.match(/^channel\s*=\s*"([^"]*)"/m)?.[1] ?? "";
  const list = text.match(/^components\s*=\s*\[([^\]]*)\]/m)?.[1] ?? "";
  const components = [...list.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  return { channel, components };
}

/** Valeur d'une ligne YAML `clé: valeur`, sans guillemets ni commentaire. */
function value(line) {
  return line
    .replace(/^[^:]*:\s*/, "")
    .replace(/\s+#.*$/, "")
    .trim()
    .replace(/^(["'])(.*)\1$/, "$2");
}

function indentOf(line) {
  return line.length - line.trimStart().length;
}

/** Lignes de l'étape dont la clé `uses:` est à la ligne `start`. */
function stepLines(lines, start) {
  const col = lines[start].indexOf("uses:");
  const block = [];
  for (let i = start + 1; i < lines.length; i++) {
    const line = lines[i];
    if (line.trim() === "") continue;
    const indent = indentOf(line);
    if (indent < col || (indent === col && line.trimStart().startsWith("- "))) break;
    block.push(line);
  }
  return block;
}

/** Valeur de l'entrée `key` du bloc `with:` d'une étape, ou null. */
function input(block, key) {
  const start = block.findIndex((l) => /^\s*with:\s*(#.*)?$/.test(l));
  if (start === -1) return null;
  const col = indentOf(block[start]);
  for (const line of block.slice(start + 1)) {
    if (line.trim() === "") continue;
    if (indentOf(line) <= col) break;
    if (new RegExp(`^\\s*${key}:`).test(line)) return value(line);
  }
  return null;
}

/** Lignes logiques (« \ » final réuni à la suivante), hors commentaires. */
function logicalLines(lines) {
  const out = [];
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].trimStart().startsWith("#")) continue;
    const no = i + 1;
    let text = lines[i];
    while (/\\\s*$/.test(text) && i + 1 < lines.length) {
      i += 1;
      text = text.replace(/\\\s*$/, " ") + lines[i];
    }
    out.push({ no, text });
  }
  return out;
}

/** Problèmes communs aux lignes de commandes (workflows et Dockerfile). */
function commandProblems(text, channel) {
  const problems = [];
  const floating = text.match(/\brustup\b.*?\b(stable|beta|nightly)\b(?!-\d)/);
  if (floating) problems.push(`chaîne « ${floating[1]} » non figée`);
  for (const m of text.matchAll(/\bRUSTUP_TOOLCHAIN\s*[:=]\s*["']?([^"'\s]*)/g)) {
    if (m[1] !== channel && !DATED_NIGHTLY.test(m[1])) {
      problems.push(`RUSTUP_TOOLCHAIN « ${m[1]} » non figé`);
    }
  }
  if (/sh\.rustup\.rs/.test(text)) problems.push("installation de rustup par script (sh.rustup.rs)");
  const install = text.match(/\bcargo\s+(install|binstall)\b(.*)/);
  if (install) {
    const args = install[2];
    const versioned = /--version[\s=]+["']?\d+\.\d+\.\d+\b/.test(args) || /@\d+\.\d+\.\d+\b/.test(args);
    if (install[1] === "install" && !(versioned && /--locked\b/.test(args))) {
      problems.push("« cargo install » sans version exacte (--version X.Y.Z ou outil@X.Y.Z) et --locked");
    } else if (install[1] === "binstall" && !versioned) {
      problems.push("« cargo binstall » sans version exacte (--version X.Y.Z ou outil@X.Y.Z)");
    }
  }
  return problems;
}

/** Job de chaque ligne (index du job sous `jobs:`, -1 hors job). */
function jobOfLines(lines) {
  let inJobs = false;
  let job = -1;
  return lines.map((line) => {
    if (/^\S/.test(line)) {
      inJobs = /^jobs:\s*(#.*)?$/.test(line);
      job = -1;
    } else if (inJobs && /^ {2}["']?[\w-]+["']?:\s*(#.*)?$/.test(line)) {
      job += 1;
    }
    return inJobs ? job : -1;
  });
}

/** Problèmes d'un workflow (`name` : nom affiché, `text` : contenu). */
function workflowProblems(name, text, { channel, components }) {
  const problems = [];
  const report = (no, message) => problems.push({ no, message: `${name}:${no} : ${message}` });
  const lines = text.split("\n");
  const jobOf = jobOfLines(lines);
  const installed = new Map(); // job -> chaînes installées
  const nightlies = []; // { job, no, toolchain }
  const add = (job, toolchain) => installed.set(job, (installed.get(job) ?? new Set()).add(toolchain));

  lines.forEach((line, i) => {
    const no = i + 1;
    const uses = line.match(/^\s*(?:- )?uses:\s*["']?([^"'\s@]+)@/);
    if (!uses) return;
    const action = uses[1].toLowerCase();
    if (action === "dtolnay/rust-toolchain") {
      const block = stepLines(lines, i);
      const toolchain = input(block, "toolchain");
      if (toolchain === null) {
        report(no, `dtolnay/rust-toolchain sans « toolchain: ${channel} » (rust-toolchain.toml)`);
      } else if (DATED_NIGHTLY.test(toolchain)) {
        add(jobOf[i], toolchain);
        nightlies.push({ job: jobOf[i], no, toolchain });
      } else if (toolchain !== channel) {
        report(no, `dtolnay/rust-toolchain avec « ${toolchain} » au lieu de ${channel} (rust-toolchain.toml)`);
      } else {
        add(jobOf[i], toolchain);
        const listed = (input(block, "components") ?? "").split(",").map((c) => c.trim());
        for (const c of components.filter((c) => !listed.includes(c))) {
          report(no, `dtolnay/rust-toolchain sans le composant ${c} (rust-toolchain.toml)`);
        }
      }
    } else if (action === "taiki-e/install-action") {
      const block = stepLines(lines, i);
      const tools = input(block, "tool");
      if (tools === null) {
        report(no, "install-action sans « tool: » (outils figés en outil@X.Y.Z)");
      } else {
        for (const tool of tools.split(/[\s,]+/).filter(Boolean)) {
          if (!EXACT.test(tool.split("@")[1] ?? "")) {
            report(no, `install-action : ${tool} sans version exacte (outil@X.Y.Z)`);
          }
        }
      }
      if (!VERIFIED_FALLBACKS.includes(input(block, "fallback"))) {
        report(
          no,
          "install-action sans « fallback: none » ni « fallback: cargo-install » (cargo-binstall : binaire non vérifié)",
        );
      }
    } else if (/setup-rust|rust-toolchain|actions-rs\/toolchain|rustup/.test(action)) {
      report(no, `action Rust ${uses[1]} non prise en charge (dtolnay/rust-toolchain figé)`);
    }
  });

  const used = new Set(); // "job toolchain"
  for (const { no, text: line } of logicalLines(lines)) {
    const job = jobOf[no - 1];
    for (const m of line.matchAll(/\bcargo\s+\+([\w.-]+)/g)) {
      used.add(`${job} ${m[1]}`);
      if (!installed.get(job)?.has(m[1])) {
        report(no, `cargo +${m[1]} : chaîne non installée par une étape de ce job`);
      }
    }
    for (const message of commandProblems(line, channel)) report(no, message);
  }
  for (const { job, no, toolchain } of nightlies) {
    if (!used.has(`${job} ${toolchain}`)) {
      report(no, `dtolnay/rust-toolchain avec « ${toolchain} » sans « cargo +${toolchain} » dans ce job`);
    }
  }
  return problems.sort((a, b) => a.no - b.no).map((p) => p.message);
}

/** Problèmes de l'image e2e. */
function dockerfileProblems(text, { channel, components }) {
  const name = "app/e2e/Dockerfile";
  const lines = logicalLines(text.split("\n"));
  const code = lines.map((l) => l.text).join("\n");
  const problems = [];
  const versions = [...code.matchAll(/^\s*ARG\s+RUST_VERSION=(\S*)/gm)].map((m) => m[1]);
  if (versions.length === 0) {
    problems.push(`${name} : « ARG RUST_VERSION=… » absent`);
  } else if (versions.length > 1) {
    problems.push(`${name} : « ARG RUST_VERSION=… » défini plusieurs fois`);
  } else if (versions[0] !== channel) {
    problems.push(`${name} : RUST_VERSION=${versions[0]} au lieu de ${channel} (rust-toolchain.toml)`);
  }
  if (!/--default-toolchain\s+"?\$\{RUST_VERSION\}"?(\s|$)/.test(code)) {
    problems.push(`${name} : rustup-init sans « --default-toolchain \${RUST_VERSION} »`);
  }
  const listed = (code.match(/--component\s+([\w,-]+)/)?.[1] ?? "").split(",");
  for (const c of components.filter((c) => !listed.includes(c))) {
    problems.push(`${name} : rustup-init sans le composant ${c} (rust-toolchain.toml)`);
  }
  for (const { text: line } of lines) {
    for (const message of commandProblems(line, channel)) problems.push(`${name} : ${message}`);
  }
  return problems;
}

/**
 * Problèmes de versions flottantes.
 * `toolchain` : contenu de rust-toolchain.toml ; `workflows` : { nom: contenu } ;
 * `dockerfile` : contenu de app/e2e/Dockerfile.
 */
export function toolchainProblems({ toolchain, workflows, dockerfile }) {
  const parsed = parseToolchain(toolchain);
  if (!EXACT.test(parsed.channel)) {
    return [`rust-toolchain.toml : version « ${parsed.channel} » non figée (attendu X.Y.Z)`];
  }
  return [
    ...Object.entries(workflows).flatMap(([name, text]) => workflowProblems(name, text, parsed)),
    ...dockerfileProblems(dockerfile, parsed),
  ];
}

if (isEntryPoint(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const dir = path.join(root, ".github", "workflows");
  const workflows = Object.fromEntries(
    readdirSync(dir)
      .filter((f) => /\.ya?ml$/.test(f))
      .sort()
      .map((f) => [`.github/workflows/${f}`, readFileSync(path.join(dir, f), "utf8")]),
  );
  const files = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard"], {
    cwd: root,
    encoding: "utf8",
  })
    .split("\n")
    .filter(Boolean);
  const problems = [
    ...toolchainFileProblems(files),
    ...toolchainProblems({
      toolchain: readFileSync(path.join(root, "rust-toolchain.toml"), "utf8"),
      workflows,
      dockerfile: readFileSync(path.join(root, "app", "e2e", "Dockerfile"), "utf8"),
    }),
  ];
  for (const p of problems) console.error(`✗ ${p}`);
  if (problems.length > 0) {
    console.error(`\n${problems.length} version(s) non figée(s) (procédure : docs/technique/methode.md).`);
    process.exit(1);
  }
  console.log("Chaîne Rust et outils de la CI : versions figées et cohérentes.");
}
