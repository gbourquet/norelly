#!/usr/bin/env bash
# Retire du cache de l'e2e les dossiers de build (target-<empreinte>/, voir
# app/e2e/target-name.sh) dont la copie de travail n'existe plus.
#
# Chaque dossier de build note dans `.source` le chemin réel des sources
# qui l'ont produit (écrit par app/e2e/run.sh). Seul est retiré un dossier
# ordinaire (jamais un lien symbolique) au nom `target-` + 12 hexadécimaux,
# dont `.source` est un fichier ordinaire contenant un chemin absolu qui
# n'existe plus. Dossier d'origine inconnue, ancien `target/`, registre de
# cargo et `home/` : jamais touchés.
#
# Usage : prune-targets.sh <dossier du cache>
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage : prune-targets.sh <dossier du cache>" >&2
  exit 2
fi
cache=$1
[ -d "$cache" ] || exit 0

shopt -s nullglob
for dir in "$cache"/target-*; do
  name=${dir##*/}
  [[ $name =~ ^target-[0-9a-f]{12}$ ]] || continue
  [ -d "$dir" ] && [ ! -L "$dir" ] || continue
  marker="$dir/.source"
  [ -f "$marker" ] && [ ! -L "$marker" ] || continue
  # Le « x » final garde les sauts de ligne de fin du chemin.
  source=$(cat -- "$marker" && printf x)
  source=${source%x}
  [[ $source == /* ]] || continue
  if [ ! -e "$source" ] && [ ! -L "$source" ]; then
    echo "==> Dossier de build retiré (copie de travail absente : $source) : $dir"
    rm -rf -- "$dir"
  fi
done
