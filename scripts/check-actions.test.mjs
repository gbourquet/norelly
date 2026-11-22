// Tests du vérificateur des actions GitHub (scripts/check-actions.mjs),
// lancés par scripts/check.sh avec `node --test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { dependabotProblems, unpinnedActions } from "./check-actions.mjs";

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
