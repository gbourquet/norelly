// Tests du vérificateur des actions GitHub (scripts/check-actions.mjs),
// lancés par scripts/check.sh avec `node --test`.
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { after, test } from "node:test";
import assert from "node:assert/strict";
import {
  checkRepository,
  dependabotProblems,
  pinnedActions,
  unpinnedActions,
  upstreamProblems,
} from "./check-actions.mjs";

const roots = [];
after(() => {
  for (const root of roots) rmSync(root, { recursive: true, force: true });
});

/** Arborescence temporaire : { "chemin/relatif": contenu }. */
function tree(files) {
  const root = mkdtempSync(path.join(tmpdir(), "check-actions-"));
  roots.push(root);
  for (const [rel, content] of Object.entries(files)) {
    const file = path.join(root, rel);
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(file, content);
  }
  return root;
}

const SHA = "fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09";

test("accepte une action figée par empreinte avec sa version en commentaire", () => {
  const workflow = [
    "jobs:",
    "  build:",
    "    steps:",
    `      - uses: actions/checkout@${SHA} # v5`,
    `      - name: Cache`,
    `        uses: actions/cache/restore@${SHA} # v6.1.0`,
    `      - uses: "pnpm/action-setup@${SHA}" # v6`,
    "      - uses: ./.github/actions/locale",
    "  appel:",
    `    uses: org/depot/.github/workflows/commun.yml@${SHA} # v1`,
  ].join("\n");
  assert.deepEqual(unpinnedActions(workflow, "ci.yml"), []);
});

test("signale une action figée par étiquette, branche ou empreinte courte", () => {
  const workflow = [
    "    steps:",
    "      - uses: actions/checkout@v5",
    "      - uses: dtolnay/rust-toolchain@stable",
    "      - uses: Swatinem/rust-cache@d1527c8 # v2",
    `      - uses: actions/setup-node@${SHA.toUpperCase()} # v5`,
    "      - uses: actions/upload-artifact",
  ].join("\n");
  assert.deepEqual(unpinnedActions(workflow, "ci.yml"), [
    "ci.yml:2 : actions/checkout@v5 n'est pas figée par une empreinte de 40 caractères",
    "ci.yml:3 : dtolnay/rust-toolchain@stable n'est pas figée par une empreinte de 40 caractères",
    "ci.yml:4 : Swatinem/rust-cache@d1527c8 n'est pas figée par une empreinte de 40 caractères",
    `ci.yml:5 : actions/setup-node@${SHA.toUpperCase()} n'est pas figée par une empreinte de 40 caractères`,
    "ci.yml:6 : actions/upload-artifact n'est pas figée par une empreinte de 40 caractères",
  ]);
});

test("exige la version en commentaire, que Dependabot met à jour avec l'empreinte", () => {
  const workflow = `      - uses: actions/checkout@${SHA}`;
  assert.deepEqual(unpinnedActions(workflow, "ci.yml"), [
    `ci.yml:1 : actions/checkout@${SHA} sans version en commentaire (# vX)`,
  ]);
});

test("image Docker : figée par empreinte sha256 seulement", () => {
  const digest = "a".repeat(64);
  const workflow = [
    `      - uses: docker://alpine@sha256:${digest}`,
    "      - uses: docker://alpine:3.20",
  ].join("\n");
  assert.deepEqual(unpinnedActions(workflow, "ci.yml"), [
    "ci.yml:2 : docker://alpine:3.20 n'est pas figée par une empreinte sha256",
  ]);
});

test("ignore les commentaires et le texte qui ne déclare pas d'action", () => {
  const workflow = [
    "      # - uses: actions/checkout@v5",
    "      - run: echo 'uses: actions/checkout@v5'",
  ].join("\n");
  assert.deepEqual(unpinnedActions(workflow, "ci.yml"), []);
});

const DEPENDABOT = [
  "version: 2",
  "updates:",
  '  - package-ecosystem: "github-actions"',
  '    directory: "/"',
  "    schedule:",
  '      interval: "weekly"',
  "    cooldown:",
  "      default-days: 7",
].join("\n");

test("Dependabot surveille les actions GitHub", () => {
  assert.deepEqual(dependabotProblems(DEPENDABOT), []);
});

test("signale une configuration Dependabot absente ou sans les actions GitHub", () => {
  assert.deepEqual(dependabotProblems(undefined), [
    ".github/dependabot.yml absent : les mises à jour des actions ne sont pas proposées",
  ]);
  assert.deepEqual(dependabotProblems(DEPENDABOT.replace("github-actions", "cargo")), [
    ".github/dependabot.yml : aucune surveillance de l'écosystème github-actions",
  ]);
  assert.deepEqual(dependabotProblems(DEPENDABOT.replace("version: 2", "version: 1")), [
    ".github/dependabot.yml : « version: 2 » attendu",
  ]);
});

test("échoue fermé sur une forme de uses: qu'il ne sait pas lire", () => {
  const workflow = [
    "      - uses: actions/checkout@v5 # v5\r",
    "      - { uses: actions/checkout@v5 }",
    `      - name: x`,
    `        "uses": actions/checkout@v5`,
    "      - uses: >-",
    "          actions/checkout@v5",
  ].join("\n");
  assert.deepEqual(unpinnedActions(workflow, "ci.yml"), [
    "ci.yml:1 : actions/checkout@v5 n'est pas figée par une empreinte de 40 caractères",
    "ci.yml:2 : forme de « uses: » non reconnue, à écrire « uses: <action>@<empreinte> # vX »",
    "ci.yml:4 : actions/checkout@v5 n'est pas figée par une empreinte de 40 caractères",
    "ci.yml:5 : forme de « uses: » non reconnue, à écrire « uses: <action>@<empreinte> # vX »",
  ]);
});

test("le commentaire doit être une version, sauf canal suivi de dtolnay/rust-toolchain", () => {
  const workflow = [
    `      - uses: actions/checkout@${SHA} # à voir`,
    `      - uses: actions/checkout@${SHA} # v5.0.1`,
    `      - uses: actions/checkout@${SHA} # 5.0`,
    `      - uses: dtolnay/rust-toolchain@${SHA} # stable`,
    `      - uses: dtolnay/rust-toolchain@${SHA} # nightly`,
    `      - uses: actions/checkout@${SHA} # stable`,
  ].join("\n");
  assert.deepEqual(unpinnedActions(workflow, "ci.yml"), [
    `ci.yml:1 : actions/checkout@${SHA} sans version en commentaire (# vX)`,
    `ci.yml:6 : actions/checkout@${SHA} sans version en commentaire (# vX)`,
  ]);
});

test("l'entrée github-actions de Dependabot vise la racine et a un rythme", () => {
  const cargo = [
    '  - package-ecosystem: "cargo"',
    '    directory: "/"',
    "    schedule:",
    '      interval: "weekly"',
  ].join("\n");
  const noDir = DEPENDABOT.replace('directory: "/"', 'directory: "/app"');
  const noSchedule = [DEPENDABOT.replace('    schedule:\n      interval: "weekly"\n', ""), cargo].join("\n");
  assert.deepEqual(dependabotProblems(noDir), [
    '.github/dependabot.yml : l\'entrée github-actions doit viser directory: "/"',
  ]);
  assert.deepEqual(dependabotProblems(noSchedule), [
    ".github/dependabot.yml : l'entrée github-actions n'a pas de rythme (schedule.interval)",
  ]);
});

test("refuse aussi ancre, espace avant les deux-points et # dans une valeur entre guillemets", () => {
  const workflow = [
    `      - uses: &x actions/checkout@${SHA} # v5`,
    "      - uses : actions/checkout@v5",
    `      - uses: "actions/checkout@${SHA}#x" # v5`,
  ].join("\n");
  assert.deepEqual(unpinnedActions(workflow, "ci.yml"), [
    "ci.yml:1 : forme de « uses: » non reconnue, à écrire « uses: <action>@<empreinte> # vX »",
    "ci.yml:2 : actions/checkout@v5 n'est pas figée par une empreinte de 40 caractères",
    "ci.yml:3 : forme de « uses: » non reconnue, à écrire « uses: <action>@<empreinte> # vX »",
  ]);
});

test("action locale : dans le dépôt et présente", () => {
  const workflow = [
    "      - uses: ./.github/actions/presente",
    "      - uses: ./.github/actions/absente",
    "      - uses: ./../ailleurs",
  ].join("\n");
  const status = (p) => (p === ".github/actions/presente" ? "ok" : "absent");
  assert.deepEqual(unpinnedActions(workflow, "ci.yml", status), [
    "ci.yml:2 : action locale ./.github/actions/absente introuvable",
    "ci.yml:3 : action locale ./../ailleurs hors du dépôt",
  ]);
});

test("actions composites locales : Dependabot doit surveiller leur dossier", () => {
  const dirs = { localActionDirs: [".github/actions/setup", "ci/outils"] };
  assert.deepEqual(dependabotProblems(DEPENDABOT, dirs), [
    '.github/dependabot.yml : l\'entrée github-actions doit aussi viser "/.github/actions/setup" (action composite locale)',
    '.github/dependabot.yml : l\'entrée github-actions doit aussi viser "/ci/outils" (action composite locale)',
  ]);
  const both = DEPENDABOT.replace(
    'directory: "/"',
    'directories:\n      - "/"\n      - "/.github/actions/*"\n      - "/ci/outils"',
  );
  assert.deepEqual(dependabotProblems(both, dirs), []);
});

test("refuse une configuration Dependabot qui coupe ou retarde trop les propositions", () => {
  const ignore = (pattern, types) =>
    `${DEPENDABOT}\n    ignore:\n      - dependency-name: "${pattern}"${types ? `\n        update-types: ${types}` : ""}`;
  const message = (pattern) =>
    `.github/dependabot.yml : ignore à motif (dependency-name: "${pattern}") hors versions majeures seules`;
  assert.deepEqual(dependabotProblems(`${DEPENDABOT}\n    open-pull-requests-limit: 0`), [
    ".github/dependabot.yml : open-pull-requests-limit: 0 coupe les propositions de mise à jour",
  ]);
  assert.deepEqual(dependabotProblems(ignore("*")), [message("*")]);
  assert.deepEqual(dependabotProblems(ignore("actions/*")), [message("actions/*")]);
  assert.deepEqual(dependabotProblems(ignore("*", '["version-update:semver-major", "version-update:semver-minor"]')), [
    message("*"),
  ]);
  assert.deepEqual(dependabotProblems(ignore("*", '["version-update:semver-major"]')), []);
  assert.deepEqual(dependabotProblems(ignore("dtolnay/rust-toolchain")), []);
});

test("exige un délai avant de proposer une version, entre 1 et 30 jours", () => {
  const message = ".github/dependabot.yml : l'entrée github-actions doit avoir cooldown.default-days entre 1 et 30";
  assert.deepEqual(dependabotProblems(DEPENDABOT.replace("    cooldown:\n      default-days: 7", "")), [message]);
  assert.deepEqual(dependabotProblems(DEPENDABOT.replace("default-days: 7", "default-days: 0")), [message]);
  assert.deepEqual(dependabotProblems(DEPENDABOT.replace("default-days: 7", "default-days: 31")), [message]);
  assert.deepEqual(dependabotProblems(DEPENDABOT.replace("default-days: 7", "default-days: 30")), []);
});

test("refuse retours chariot seuls, sauts de ligne Unicode, clé explicite ou échappée", () => {
  assert.deepEqual(unpinnedActions("steps:\r      - uses: actions/checkout@v5\r", "a.yml"), [
    "a.yml:2 : actions/checkout@v5 n'est pas figée par une empreinte de 40 caractères",
  ]);
  assert.deepEqual(unpinnedActions(`steps:\u2028      - uses: x/y@${SHA} # v1`, "b.yml"), [
    "b.yml : saut de ligne Unicode (U+0085, U+2028, U+2029) refusé",
  ]);
  const workflow = ["      - ? uses", "        : actions/checkout@v5", '      - "u\\x73es": actions/checkout@v5'].join("\n");
  assert.deepEqual(unpinnedActions(workflow, "c.yml"), [
    "c.yml:1 : forme de « uses: » non reconnue, à écrire « uses: <action>@<empreinte> # vX »",
    "c.yml:3 : forme de « uses: » non reconnue, à écrire « uses: <action>@<empreinte> # vX »",
  ]);
});

test("parcourt workflows, actions composites où qu'elles soient et action racine", () => {
  const steps = (...uses) => ["jobs:", "  a:", "    steps:", ...uses.map((u) => `      - uses: ${u}`)].join("\n");
  const root = tree({
    ".github/workflows/ci.yml": steps(`actions/checkout@${SHA} # v5`, "./ci/setup", "./absente", "./.github/actions/lien"),
    ".github/dependabot.yml": DEPENDABOT,
    "ci/setup/action.yml": ["runs:", "  using: composite", "  steps:", "    - uses: a/b@v1", "    - uses: ./ci/setup"].join(
      "\n",
    ),
    "action.yml": ["runs:", "  using: composite", "  steps:", "    - uses: c/d@v2"].join("\n"),
  });
  mkdirSync(path.join(root, ".github", "actions"), { recursive: true });
  symlinkSync(path.join(root, "ci", "setup"), path.join(root, ".github", "actions", "lien"));
  assert.deepEqual(checkRepository(root), [
    ".github/actions/lien : lien symbolique refusé sous .github",
    ".github/workflows/ci.yml:6 : action locale ./absente introuvable",
    "action.yml:4 : c/d@v2 n'est pas figée par une empreinte de 40 caractères",
    "ci/setup/action.yml:4 : a/b@v1 n'est pas figée par une empreinte de 40 caractères",
    '.github/dependabot.yml : l\'entrée github-actions doit aussi viser "/.github/actions/lien" (action composite locale)',
    '.github/dependabot.yml : l\'entrée github-actions doit aussi viser "/ci/setup" (action composite locale)',
  ]);
});

test("une action locale qui sort du dépôt par lien symbolique est refusée", () => {
  const outside = tree({ "action.yml": "runs:\n  using: composite\n  steps: []\n" });
  const root = tree({
    ".github/workflows/ci.yml": "jobs:\n  a:\n    steps:\n      - uses: ./outils\n",
    ".github/dependabot.yml": DEPENDABOT,
  });
  symlinkSync(outside, path.join(root, "outils"));
  assert.deepEqual(checkRepository(root), [".github/workflows/ci.yml:4 : action locale ./outils hors du dépôt"]);
});

test("relève les actions figées, chemin d'action réduit au dépôt", () => {
  const workflow = [
    `      - uses: actions/cache/restore@${SHA} # v6.1.0`,
    `      - uses: dtolnay/rust-toolchain@${SHA} # stable`,
    "      - uses: ./.github/actions/locale",
  ].join("\n");
  assert.deepEqual(pinnedActions(workflow, "ci.yml"), [
    { where: "ci.yml:1", repo: "actions/cache", sha: SHA, version: "v6.1.0", channel: false },
    { where: "ci.yml:2", repo: "dtolnay/rust-toolchain", sha: SHA, version: "stable", channel: true },
  ]);
});

test("l'empreinte est celle d'une étiquette amont qui correspond à la version du commentaire", () => {
  const other = "b".repeat(40);
  const refs = new Map([
    [
      "actions/checkout",
      [
        { sha: SHA, name: "refs/tags/v5.1.0" },
        { sha: "c".repeat(40), name: "refs/tags/v4.2.0" },
        { sha: "c".repeat(40), name: "refs/tags/v4" },
      ],
    ],
    [
      "actions/cache",
      [
        { sha: "d".repeat(40), name: "refs/tags/v6.1.0" },
        { sha: other, name: "refs/tags/v6.1.0^{}" },
      ],
    ],
  ]);
  const pins = [
    { where: "a:1", repo: "actions/checkout", sha: SHA, version: "v5", channel: false },
    { where: "a:2", repo: "actions/checkout", sha: SHA, version: "v5.1.0", channel: false },
    { where: "a:3", repo: "actions/cache", sha: other, version: "v6.1.0", channel: false },
    { where: "a:4", repo: "actions/checkout", sha: SHA, version: "v4", channel: false },
    { where: "a:5", repo: "actions/checkout", sha: other, version: "v5", channel: false },
    { where: "a:6", repo: "actions/inconnue", sha: SHA, version: "v1", channel: false },
    { where: "a:7", repo: "dtolnay/rust-toolchain", sha: SHA, version: "stable", channel: true },
  ];
  assert.deepEqual(upstreamProblems(pins, refs), [
    `a:4 : actions/checkout@${SHA} n'est l'empreinte d'aucune étiquette v4 du dépôt amont`,
    `a:5 : actions/checkout@${other} n'est l'empreinte d'aucune étiquette v5 du dépôt amont`,
    `a:6 : actions/inconnue@${SHA} n'est l'empreinte d'aucune étiquette v1 du dépôt amont`,
  ]);
});
