#!/usr/bin/env bash
# Vérification de la documentation (phase 11), lancée par ./scripts/check.sh :
#   - tests du vérificateur lui-même (scripts/check-docs.test.mjs) ;
#   - liens internes valides dans tous les fichiers Markdown du dépôt ;
#   - documentation utilisateur : mêmes pages et mêmes captures en français
#     et en anglais, captures présentes (PNG) et toutes utilisées ;
#   - chaque commande du registre présente dans l'aide-mémoire des
#     raccourcis, dans les deux langues (app/src/docs.test.ts, registre réel) ;
#   - documentation fonctionnelle sans vocabulaire technique.
# Les captures elles-mêmes sont produites par `pnpm -C app e2e`
# (app/e2e/captures), jamais à la main.
set -euo pipefail
cd "$(dirname "$0")/.."

node --test scripts/check-docs.test.mjs
node scripts/check-docs.mjs
pnpm -C app i18n
pnpm -C app exec vitest run src/docs.test.ts
