#!/usr/bin/env bash
# Régénère les bindings TypeScript du contrat IPC (app/src/lib/ipc/bindings.ts)
# à partir des commandes Rust (tauri-specta). Le test
# `bindings_are_up_to_date` échoue tant qu'ils sont périmés.
set -euo pipefail
cd "$(dirname "$0")/.."
NORELLY_UPDATE_BINDINGS=1 cargo nextest run -p norelly-app --locked bindings_are_up_to_date
echo "Bindings régénérés : app/src/lib/ipc/bindings.ts"
