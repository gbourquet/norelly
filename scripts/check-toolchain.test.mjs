// Tests du contrôle des versions figées (scripts/check-toolchain.mjs),
// lancés par scripts/check.sh avec `node --test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { parseToolchain, toolchainFileProblems, toolchainProblems } from "./check-toolchain.mjs";

const TOOLCHAIN = [
  "[toolchain]",
  'channel = "1.100.0"',
  'components = ["rustfmt", "clippy", "llvm-tools-preview"]',
  "",
].join("\n");

const DOCKERFILE = [
  "ARG RUST_VERSION=1.100.0",
  "RUN /tmp/rustup-init -y --profile minimal --default-toolchain ${RUST_VERSION} \\",
  "      --component rustfmt,clippy,llvm-tools-preview \\",
  "    && rm /tmp/rustup-init",
  "",
].join("\n");

/** Workflow minimal : un job dont les étapes sont données en YAML. */
function workflow(steps) {
  return ["jobs:", "  build:", "    steps:", ...steps.map((s) => `      ${s}`), ""].join("\n");
}

/** Workflow de plusieurs jobs : { nom: étapes }. */
function jobs(byJob) {
  return [
    "jobs:",
    ...Object.entries(byJob).flatMap(([name, steps]) => [
      `  ${name}:`,
      "    steps:",
      ...steps.map((s) => `      ${s}`),
    ]),
    "",
  ].join("\n");
}

const RUST_STEP = [
  "- uses: dtolnay/rust-toolchain@b725eda525362e7cf34af9a7243cac1e3f01a7ec # stable",
  "  with:",
  "    toolchain: 1.100.0",
  "    components: rustfmt, clippy, llvm-tools-preview",
];

function check(workflows, { toolchain = TOOLCHAIN, dockerfile = DOCKERFILE } = {}) {
  return toolchainProblems({ toolchain, workflows, dockerfile });
}

test("lit la version et les composants de rust-toolchain.toml", () => {
  assert.deepEqual(parseToolchain(TOOLCHAIN), {
    channel: "1.100.0",
    components: ["rustfmt", "clippy", "llvm-tools-preview"],
  });
});

test("une configuration entièrement figée ne signale rien", () => {
  const ci = workflow([
    ...RUST_STEP,
    "- uses: taiki-e/install-action@c509219cf42fba86913069efd0ade2458c59337b # v2",
    "  with:",
    "    tool: cargo-nextest@0.9.151,cargo-deny@0.20.2",
    "    fallback: none",
    "- run: ./scripts/check.sh full",
  ]);
  const nightly = workflow([
    "- uses: dtolnay/rust-toolchain@2c76c034f9e32a5d6e2c104dc4df23a22476db04 # nightly",
    "  with:",
    "    toolchain: nightly-2026-11-20",
    "- uses: taiki-e/install-action@c509219cf42fba86913069efd0ade2458c59337b # v2",
    "  with:",
    "    tool: cargo-fuzz@0.13.2",
    "    fallback: cargo-install",
    "  env:",
    "    RUSTUP_TOOLCHAIN: nightly-2026-11-20",
    "- run: cargo +nightly-2026-11-20 fuzz run cible",
    "- run: cargo install --locked tauri-driver --version 2.1.0 && echo fait # cargo install x",
  ]);
  assert.deepEqual(check({ "ci.yml": ci, "nightly.yml": nightly }), []);
});

test("rust-toolchain.toml exige une version exacte", () => {
  for (const channel of ["stable", "1.100", "nightly", "beta"]) {
    const toolchain = TOOLCHAIN.replace('"1.100.0"', `"${channel}"`);
    assert.deepEqual(check({}, { toolchain }), [
      `rust-toolchain.toml : version « ${channel} » non figée (attendu X.Y.Z)`,
    ]);
  }
});

test("signale l'action Rust sans chaîne explicite ou d'une autre version", () => {
  const without = workflow([RUST_STEP[0], "- run: cargo build"]);
  const other = workflow(RUST_STEP.map((l) => l.replace("toolchain: 1.100.0", "toolchain: stable")));
  assert.deepEqual(check({ "a.yml": without, "b.yml": other }), [
    "a.yml:4 : dtolnay/rust-toolchain sans « toolchain: 1.100.0 » (rust-toolchain.toml)",
    "b.yml:4 : dtolnay/rust-toolchain avec « stable » au lieu de 1.100.0 (rust-toolchain.toml)",
  ]);
});

test("signale l'action Rust sans les composants de rust-toolchain.toml", () => {
  const partial = workflow(RUST_STEP.map((l) => l.replace(", llvm-tools-preview", "")));
  const none = workflow(RUST_STEP.slice(0, 3));
  assert.deepEqual(check({ "a.yml": partial, "b.yml": none }), [
    "a.yml:4 : dtolnay/rust-toolchain sans le composant llvm-tools-preview (rust-toolchain.toml)",
    "b.yml:4 : dtolnay/rust-toolchain sans le composant rustfmt (rust-toolchain.toml)",
    "b.yml:4 : dtolnay/rust-toolchain sans le composant clippy (rust-toolchain.toml)",
    "b.yml:4 : dtolnay/rust-toolchain sans le composant llvm-tools-preview (rust-toolchain.toml)",
  ]);
});

test("un nightly doit être daté, et utilisé tel quel par cargo +…", () => {
  const floating = workflow([
    "- uses: dtolnay/rust-toolchain@2c76c034f9e32a5d6e2c104dc4df23a22476db04 # nightly",
    "- run: cargo +nightly fuzz run cible",
  ]);
  const mismatch = workflow([
    "- uses: dtolnay/rust-toolchain@2c76c034f9e32a5d6e2c104dc4df23a22476db04 # nightly",
    "  with:",
    "    toolchain: nightly-2026-11-20",
    "- run: cargo +nightly-2026-11-01 fuzz run cible",
  ]);
  assert.deepEqual(check({ "a.yml": floating, "b.yml": mismatch }), [
    "a.yml:4 : dtolnay/rust-toolchain sans « toolchain: 1.100.0 » (rust-toolchain.toml)",
    "a.yml:5 : cargo +nightly : chaîne non installée par une étape de ce job",
    "b.yml:4 : dtolnay/rust-toolchain avec « nightly-2026-11-20 » sans « cargo +nightly-2026-11-20 » dans ce job",
    "b.yml:7 : cargo +nightly-2026-11-01 : chaîne non installée par une étape de ce job",
  ]);
});

test("signale un nightly non daté même installé explicitement", () => {
  const ci = workflow([
    "- uses: dtolnay/rust-toolchain@2c76c034f9e32a5d6e2c104dc4df23a22476db04 # nightly",
    "  with:",
    "    toolchain: nightly",
    "- run: cargo +nightly fuzz run cible",
  ]);
  assert.deepEqual(check({ "a.yml": ci }), [
    "a.yml:4 : dtolnay/rust-toolchain avec « nightly » au lieu de 1.100.0 (rust-toolchain.toml)",
    "a.yml:7 : cargo +nightly : chaîne non installée par une étape de ce job",
  ]);
});

test("signale les outils de install-action sans version exacte", () => {
  const ci = workflow([
    "- uses: taiki-e/install-action@c509219cf42fba86913069efd0ade2458c59337b # v2",
    "  with:",
    "    tool: cargo-nextest, cargo-deny@0.20, cargo-llvm-cov@0.9.1",
    "    fallback: none",
    "- uses: taiki-e/install-action@c509219cf42fba86913069efd0ade2458c59337b # v2",
    "  with:",
    "    fallback: none",
  ]);
  assert.deepEqual(check({ "a.yml": ci }), [
    "a.yml:4 : install-action : cargo-nextest sans version exacte (outil@X.Y.Z)",
    "a.yml:4 : install-action : cargo-deny@0.20 sans version exacte (outil@X.Y.Z)",
    "a.yml:8 : install-action sans « tool: » (outils figés en outil@X.Y.Z)",
  ]);
});

test("signale un appel cargo +stable ou rustup sur une chaîne flottante", () => {
  const ci = workflow([...RUST_STEP, "- run: cargo +stable build", "- run: rustup default stable"]);
  assert.deepEqual(check({ "a.yml": ci }), [
    "a.yml:8 : cargo +stable : chaîne non installée par une étape de ce job",
    "a.yml:9 : chaîne « stable » non figée",
  ]);
});

test("l'image e2e installe la même chaîne et les mêmes composants", () => {
  const stable = DOCKERFILE.replace("${RUST_VERSION}", "stable");
  const older = DOCKERFILE.replace("RUST_VERSION=1.100.0", "RUST_VERSION=1.99.0");
  const partial = DOCKERFILE.replace(",llvm-tools-preview", "");
  const absent = "FROM ubuntu:24.04\n";
  assert.deepEqual(check({}, { dockerfile: stable }), [
    "app/e2e/Dockerfile : rustup-init sans « --default-toolchain ${RUST_VERSION} »",
    "app/e2e/Dockerfile : chaîne « stable » non figée",
  ]);
  assert.deepEqual(check({}, { dockerfile: older }), [
    "app/e2e/Dockerfile : RUST_VERSION=1.99.0 au lieu de 1.100.0 (rust-toolchain.toml)",
  ]);
  assert.deepEqual(check({}, { dockerfile: partial }), [
    "app/e2e/Dockerfile : rustup-init sans le composant llvm-tools-preview (rust-toolchain.toml)",
  ]);
  assert.deepEqual(check({}, { dockerfile: absent }), [
    "app/e2e/Dockerfile : « ARG RUST_VERSION=… » absent",
    "app/e2e/Dockerfile : rustup-init sans « --default-toolchain ${RUST_VERSION} »",
    "app/e2e/Dockerfile : rustup-init sans le composant rustfmt (rust-toolchain.toml)",
    "app/e2e/Dockerfile : rustup-init sans le composant clippy (rust-toolchain.toml)",
    "app/e2e/Dockerfile : rustup-init sans le composant llvm-tools-preview (rust-toolchain.toml)",
  ]);
});

test("install-action sans repli vérifié (cargo-binstall par défaut)", () => {
  const tool = "    tool: cargo-fuzz@0.13.2";
  const step = "- uses: taiki-e/install-action@c509219cf42fba86913069efd0ade2458c59337b # v2";
  const absent = workflow([step, "  with:", tool]);
  const binstall = workflow([step, "  with:", tool, "    fallback: cargo-binstall"]);
  const message = (n) =>
    `${n}:4 : install-action sans « fallback: none » ni « fallback: cargo-install » (cargo-binstall : binaire non vérifié)`;
  assert.deepEqual(check({ "a.yml": absent, "b.yml": binstall }), [message("a.yml"), message("b.yml")]);
});

test("un nightly daté n'est accepté que pour un cargo +… du même job", () => {
  const nightlyStep = [
    "- uses: dtolnay/rust-toolchain@2c76c034f9e32a5d6e2c104dc4df23a22476db04 # nightly",
    "  with:",
    "    toolchain: nightly-2026-11-20",
  ];
  const ci = jobs({
    fuzz: nightlyStep,
    other: ["- run: cargo +nightly-2026-11-20 fuzz run cible"],
  });
  assert.deepEqual(check({ "a.yml": ci }), [
    "a.yml:4 : dtolnay/rust-toolchain avec « nightly-2026-11-20 » sans « cargo +nightly-2026-11-20 » dans ce job",
    "a.yml:9 : cargo +nightly-2026-11-20 : chaîne non installée par une étape de ce job",
  ]);
});

test("formes d'écriture : guillemets, casse, espaces, lignes continuées", () => {
  const ci = workflow([
    '- uses: "Dtolnay/Rust-Toolchain@b725eda525362e7cf34af9a7243cac1e3f01a7ec" # stable',
    "  env:",
    "    toolchain: 1.100.0",
    "- uses: actions-rust-lang/setup-rust-toolchain@0123456789abcdef0123456789abcdef01234567 # v1",
    "- run: cargo  +stable build",
    "- run: |",
    "    rustup toolchain install \\",
    "      stable",
    "- run: curl -sSf https://sh.rustup.rs | sh",
  ]);
  assert.deepEqual(check({ "a.yml": ci }), [
    "a.yml:4 : dtolnay/rust-toolchain sans « toolchain: 1.100.0 » (rust-toolchain.toml)",
    "a.yml:7 : action Rust actions-rust-lang/setup-rust-toolchain non prise en charge (dtolnay/rust-toolchain figé)",
    "a.yml:8 : cargo +stable : chaîne non installée par une étape de ce job",
    "a.yml:10 : chaîne « stable » non figée",
    "a.yml:12 : installation de rustup par script (sh.rustup.rs)",
  ]);
});

test("RUSTUP_TOOLCHAIN : version exacte ou nightly daté seulement", () => {
  const ci = workflow([
    ...RUST_STEP,
    "  env:",
    "    RUSTUP_TOOLCHAIN: stable",
    "- run: RUSTUP_TOOLCHAIN=1.100.0 cargo build",
  ]);
  assert.deepEqual(check({ "a.yml": ci }), ["a.yml:9 : RUSTUP_TOOLCHAIN « stable » non figé"]);
});

test("cargo install exige une version exacte et --locked ; cargo-binstall refusé", () => {
  const ci = workflow([
    "- run: cargo install cargo-xyz",
    "- run: cargo install cargo-xyz --version 1.2.3",
    "- run: cargo binstall cargo-xyz@1.2.3",
    "- run: cargo install --locked --version 1.2.3 a && cargo install b",
    "- run: cargo-binstall -y foo",
  ]);
  const install =
    "« cargo install » sans version exacte (--version X.Y.Z ou outil@X.Y.Z) et --locked";
  const binstall = "cargo-binstall refusé (binaire non vérifié) : cargo install --locked --version X.Y.Z";
  assert.deepEqual(check({ "a.yml": ci }), [
    `a.yml:4 : ${install}`,
    `a.yml:5 : ${install}`,
    `a.yml:6 : ${binstall}`,
    `a.yml:7 : ${install}`,
    `a.yml:8 : ${binstall}`,
  ]);
});

test("rustc, rustdoc et cargo +… (même entre guillemets) : chaîne installée par le job", () => {
  const ci = workflow([...RUST_STEP, "- run: rustc +nightly -vV", '- run: cargo "+nightly" build', "- run: rustdoc +1.100.0 x.rs"]);
  assert.deepEqual(check({ "a.yml": ci }), [
    "a.yml:8 : rustc +nightly : chaîne non installée par une étape de ce job",
    "a.yml:9 : cargo +nightly : chaîne non installée par une étape de ce job",
  ]);
});

test("jobs reconnus malgré un commentaire en colonne 0 ou une autre indentation", () => {
  const ci = [
    "jobs:",
    "    fuzz:",
    "        steps:",
    "            - uses: dtolnay/rust-toolchain@2c76c034f9e32a5d6e2c104dc4df23a22476db04 # nightly",
    "              with:",
    "                toolchain: nightly-2026-11-20",
    "# commentaire",
    "    other:",
    "        steps:",
    "            - run: cargo +nightly-2026-11-20 fuzz run cible",
    "",
  ].join("\n");
  assert.deepEqual(check({ "a.yml": ci }), [
    "a.yml:4 : dtolnay/rust-toolchain avec « nightly-2026-11-20 » sans « cargo +nightly-2026-11-20 » dans ce job",
    "a.yml:10 : cargo +nightly-2026-11-20 : chaîne non installée par une étape de ce job",
  ]);
});

test("RUSTUP_TOOLCHAIN daté : installé par le job, et compte comme utilisation", () => {
  const nightlyStep = [
    "- uses: dtolnay/rust-toolchain@2c76c034f9e32a5d6e2c104dc4df23a22476db04 # nightly",
    "  with:",
    "    toolchain: nightly-2026-11-20",
  ];
  const ci = jobs({
    fuzz: [...nightlyStep, "- run: cargo install --locked cargo-fuzz@0.13.2", "  env:", "    RUSTUP_TOOLCHAIN: nightly-2026-11-20"],
    other: ["- run: RUSTUP_TOOLCHAIN=nightly-2026-11-20 cargo build"],
  });
  assert.deepEqual(check({ "a.yml": ci }), [
    "a.yml:12 : RUSTUP_TOOLCHAIN « nightly-2026-11-20 » : chaîne non installée par une étape de ce job",
  ]);
});

test("image e2e : un seul RUST_VERSION, hors commentaires, sans chaîne flottante", () => {
  const twice = DOCKERFILE + "ARG RUST_VERSION=stable\n";
  const commented = DOCKERFILE.replace(
    "--default-toolchain ${RUST_VERSION}",
    "--default-toolchain stable",
  ).concat("# --default-toolchain ${RUST_VERSION}\n");
  const extra = DOCKERFILE + "RUN rustup default stable\nRUN cargo install tauri-driver --locked\n";
  assert.deepEqual(check({}, { dockerfile: twice }), [
    "app/e2e/Dockerfile : « ARG RUST_VERSION=… » défini plusieurs fois",
  ]);
  assert.deepEqual(check({}, { dockerfile: commented }), [
    "app/e2e/Dockerfile : rustup-init sans « --default-toolchain ${RUST_VERSION} »",
    "app/e2e/Dockerfile : chaîne « stable » non figée",
  ]);
  assert.deepEqual(check({}, { dockerfile: extra }), [
    "app/e2e/Dockerfile : chaîne « stable » non figée",
    "app/e2e/Dockerfile : « cargo install » sans version exacte (--version X.Y.Z ou outil@X.Y.Z) et --locked",
  ]);
});

test("un seul fichier de chaîne : rust-toolchain.toml à la racine", () => {
  assert.deepEqual(
    toolchainFileProblems(["rust-toolchain.toml", "fuzz/rust-toolchain", "a/rust-toolchain.toml", "x.toml"]),
    [
      "fuzz/rust-toolchain : fichier de chaîne en plus de rust-toolchain.toml (racine)",
      "a/rust-toolchain.toml : fichier de chaîne en plus de rust-toolchain.toml (racine)",
    ],
  );
  assert.deepEqual(toolchainFileProblems(["rust-toolchain", "rust-toolchain.toml"]), [
    "rust-toolchain : fichier de chaîne en plus de rust-toolchain.toml (racine)",
  ]);
});
