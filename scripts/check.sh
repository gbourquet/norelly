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
step "Tests Rust (unitaires)"; cargo nextest run --workspace --locked --no-tests=warn
step "Doctests";               cargo test --workspace --doc --locked
step "cargo deny";             cargo deny --locked check
step "Règle de dépendance du domaine"; ./scripts/check-domain-deps.sh
step "Front : svelte-check";   pnpm -C app check
step "Front : lint";           pnpm -C app lint
step "Front : tests";          pnpm -C app test

if [ "$mode" = "full" ]; then
  step "Build workspace";      cargo build --workspace --locked
  step "Tests d'intégration";  cargo nextest run --workspace --locked --profile integration
fi

printf '\nOK (%s)\n' "$mode"
