#!/usr/bin/env bash
# Vérifications du projet.
#   ./scripts/check.sh        rapide, sans Docker
#   ./scripts/check.sh full   + build du front et du workspace + tests d'intégration (Docker)
#
# Ce script conditionne toute PR : il est interdit de le ramollir (retirer
# une étape, ignorer un échec) pour faire passer une modification.
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-fast}"
step() { printf '\n==> %s\n' "$*"; }

# Le shell Tauri embarque app/dist à la compilation : le front est construit en premier.
step "Front : build";          pnpm -C app build
step "Format Rust";            cargo fmt --all --check
step "Clippy";                 cargo clippy --workspace --all-targets --locked -- -D warnings
# Module de mise à jour (installateur NSIS seulement, ADR-006) : compilé et
# testé aussi avec sa fonctionnalité.
step "Clippy (updater)";       cargo clippy -p norelly-app --all-targets --features updater --locked -- -D warnings
# Couverture (cargo-llvm-cov) : seuil de lignes des tests unitaires, mesuré
# le 2026-11-03, à relever, jamais à baisser. Avec rustup, les outils de
# `llvm-tools-preview` ; sans rustup seulement (Rust du système), ceux du
# système, de même version majeure que rustc.
if ! command -v rustup >/dev/null; then
  if [ -z "${LLVM_COV:-}" ] && command -v llvm-cov >/dev/null; then export LLVM_COV="$(command -v llvm-cov)"; fi
  if [ -z "${LLVM_PROFDATA:-}" ] && command -v llvm-profdata >/dev/null; then export LLVM_PROFDATA="$(command -v llvm-profdata)"; fi
fi
step "Tests Rust (unitaires, couverture)"
cargo llvm-cov nextest --workspace --locked --no-tests=warn --summary-only --fail-under-lines 86
step "Tests du shell (updater)"; cargo nextest run -p norelly-app --features updater --locked
step "Doctests";               cargo test --workspace --doc --locked
step "cargo deny";             cargo deny --locked check
# Workspace de fuzzing (fuzz/, hors du workspace principal) : même politique
# (deny.toml racine). --locked : fuzz/Cargo.lock doit rester synchronisé
# avec les crates du workspace qu'il fuzze.
step "cargo deny (fuzz)";      cargo deny --manifest-path fuzz/Cargo.toml --config deny.toml --locked check
# Le fuzzing porte sur les versions livrées : fuzz/Cargo.lock reprend celles
# du Cargo.lock racine (scripts/check-fuzz-lock.mjs).
# Deux commandes distinctes : dans une liste `a && b`, set -e ignore l'échec de `a`.
step "Fuzz : verrou aligné";   node --test scripts/check-fuzz-lock.test.mjs
node scripts/check-fuzz-lock.mjs
step "Règle de dépendance du domaine"; ./scripts/check-domain-deps.sh
step "SQL : requêtes paramétrées"; ./scripts/check-sql.sh
step "Documentation";          ./scripts/check-docs.sh
# Âge de l'instantané apt de l'image e2e : avertissement seulement au-delà
# de MAX_AGE_DAYS ; date absente ou invalide : échec.
step "Instantané apt de l'e2e"
node --test scripts/check-apt-snapshot.test.mjs
node scripts/check-apt-snapshot.mjs
step "e2e : dossiers de build par copie de travail"; ./app/e2e/target-name.test.sh; ./app/e2e/prune-targets.test.sh; ./app/e2e/verify-crates.test.sh
step "Chaîne Rust et outils de la CI figés"
node --test scripts/check-toolchain.test.mjs
node scripts/check-toolchain-cli.mjs
step "Actions GitHub figées"
node --test scripts/check-actions.test.mjs
node scripts/check-actions.mjs
# Paquets Linux (desktop-file-utils, appstream) : vérifiés sous Linux.
if [ "$(uname -s)" = Linux ]; then step "Métadonnées de bureau"; ./scripts/check-packaging.sh; fi
# Fichiers d'une version et SHA256SUMS (release.yml, sous Ubuntu) : outils GNU.
if [ "$(uname -s)" = Linux ]; then step "Scripts de publication"; ./scripts/release-assets.test.sh; fi
step "Front : svelte-check";   pnpm -C app check
step "Front : lint";           pnpm -C app lint
step "Front : tests (couverture)"; pnpm -C app coverage

if [ "$mode" = "full" ]; then
  # Réseau : chaque empreinte d'action est celle d'une étiquette amont de
  # la version annoncée en commentaire.
  step "Actions GitHub : empreintes amont"; node scripts/check-actions.mjs --upstream
  step "Build workspace";      cargo build --workspace --locked
  step "Tests d'intégration";  cargo nextest run --workspace --locked --profile integration
fi

printf '\nOK (%s)\n' "$mode"
