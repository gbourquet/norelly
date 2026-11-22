#!/usr/bin/env bash
# Nom du dossier de build de l'e2e (CARGO_TARGET_DIR, dans le cache de
# app/e2e/run.sh) propre à une copie de travail : `target-` suivi d'une
# empreinte courte du chemin réel de ses sources.
#
# Toutes les copies de travail (worktrees) sont montées sur /work dans le
# conteneur : avec un seul dossier de build, cargo pourrait reprendre les
# fichiers compilés d'une autre branche, ses dates de modification ne
# suffisant pas à les distinguer. Le registre de cargo reste partagé
# (archives .crate revérifiées contre Cargo.lock).
#
# Usage : target-name.sh <dossier des sources>
# Utilisé par app/e2e/run.sh et par la CI (clé du chemin des caches e2e).
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage : target-name.sh <dossier des sources>" >&2
  exit 2
fi
real=$(CDPATH='' cd -- "$1" && pwd -P)
printf 'target-%s\n' "$(printf '%s' "$real" | sha256sum | cut -c1-12)"
