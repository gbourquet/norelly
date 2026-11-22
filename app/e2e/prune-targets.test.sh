#!/usr/bin/env bash
# Tests de app/e2e/prune-targets.sh (retrait des dossiers de build de l'e2e
# dont la copie de travail n'existe plus), lancés par ./scripts/check.sh.
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/prune-targets.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failures=0
check() { # libellé, obtenu, attendu
  if [ "$2" = "$3" ]; then
    echo "ok    $1"
  else
    echo "ÉCHEC $1 : « $2 » au lieu de « $3 »" >&2
    failures=$((failures + 1))
  fi
}
present() { if [ -e "$1" ] || [ -L "$1" ]; then echo présent; else echo absent; fi; }
verdict() { if "$@" >/dev/null 2>&1; then echo accepté; else echo refusé; fi; }

cache="$tmp/cache"
mkdir -p "$cache" "$tmp/sources/vivante" "$tmp/ailleurs"
build() { # nom, contenu de .source (absent si vide)
  mkdir -p "$cache/$1"
  touch "$cache/$1/fichier"
  if [ -n "$2" ]; then printf '%s' "$2" > "$cache/$1/.source"; fi
}
build target-aaaaaaaaaaaa "$tmp/sources/retiree"
build target-bbbbbbbbbbbb "$tmp/sources/vivante"
build target-cccccccccccc ""
build target "$tmp/sources/retiree"
build target-court "$tmp/sources/retiree"
build cargo "$tmp/sources/retiree"
build target-dddddddddddd "$tmp/sources/avec
saut de ligne"
mkdir -p "$tmp/ailleurs/target-eeeeeeeeeeee"
printf '%s' "$tmp/sources/retiree" > "$tmp/ailleurs/target-eeeeeeeeeeee/.source"
ln -s "$tmp/ailleurs/target-eeeeeeeeeeee" "$cache/target-eeeeeeeeeeee"

"$script" "$cache" >/dev/null || true

check "copie de travail retirée : dossier retiré" "$(present "$cache/target-aaaaaaaaaaaa")" absent
check "copie de travail présente : dossier gardé" "$(present "$cache/target-bbbbbbbbbbbb")" présent
check "origine inconnue : dossier gardé" "$(present "$cache/target-cccccccccccc")" présent
check "ancien dossier target/ jamais touché" "$(present "$cache/target")" présent
check "nom hors de la forme attendue : gardé" "$(present "$cache/target-court")" présent
check "registre de cargo jamais touché" "$(present "$cache/cargo")" présent
check "chemin avec saut de ligne retiré" "$(present "$cache/target-dddddddddddd")" absent
# Sous Windows (Git Bash), `ln -s` copie le dossier au lieu de le lier.
if [ -L "$cache/target-eeeeeeeeeeee" ]; then
  check "lien symbolique gardé" "$(present "$cache/target-eeeeeeeeeeee")" présent
  check "cible d'un lien symbolique jamais suivie" "$(present "$tmp/ailleurs/target-eeeeeeeeeeee/.source")" présent
else
  echo "-     liens symboliques : indisponibles, ignoré"
fi
check "cache absent accepté" "$(verdict "$script" "$tmp/absent")" accepté
check "argument obligatoire" "$(verdict "$script")" refusé

if [ "$failures" -ne 0 ]; then
  echo "$failures échec(s)" >&2
  exit 1
fi
