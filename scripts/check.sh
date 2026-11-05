#!/usr/bin/env bash
# Vérifications du projet.
#   ./scripts/check.sh        rapide, sans Docker
#   ./scripts/check.sh full   + build du front et du workspace + tests d'intégration (Docker)
#
# Ce script fait partie des conditions de fin des phases (docs/PLAN.md) :
# il est interdit de le ramollir (retirer une étape, ignorer un échec) pour
# faire passer une phase.
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
step "Règle de dépendance du domaine"; ./scripts/check-domain-deps.sh
step "SQL : requêtes paramétrées"; ./scripts/check-sql.sh
step "Documentation";          ./scripts/check-docs.sh
# Paquets Linux (desktop-file-utils, appstream) : vérifiés sous Linux.
if [ "$(uname -s)" = Linux ]; then step "Métadonnées de bureau"; ./scripts/check-packaging.sh; fi
step "Front : svelte-check";   pnpm -C app check
step "Front : lint";           pnpm -C app lint
step "Front : tests (couverture)"; pnpm -C app coverage

if [ "$mode" = "full" ]; then
  step "Build workspace";      cargo build --workspace --locked
  step "Tests d'intégration";  cargo nextest run --workspace --locked --profile integration
fi

printf '\nOK (%s)\n' "$mode"
