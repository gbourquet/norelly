#!/usr/bin/env bash
# Tests de app/e2e/target-name.sh (nom du dossier de build de l'e2e propre
# à une copie de travail), lancés par ./scripts/check.sh.
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/target-name.sh"
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
verdict() { if "$@" >/dev/null 2>&1; then echo accepté; else echo refusé; fi; }

mkdir -p "$tmp/a" "$tmp/b" "$tmp/avec espace"
ln -s "$tmp/a" "$tmp/lien"
a=$("$script" "$tmp/a" || true)
b=$("$script" "$tmp/b" || true)

check "forme target-<12 hexadécimaux>" "$([[ $a =~ ^target-[0-9a-f]{12}$ ]] && echo oui || echo non)" oui
check "stable d'un appel à l'autre" "$("$script" "$tmp/a" || true)" "$a"
check "propre à chaque copie de travail" "$([ "$a" != "$b" ] && echo différent || echo identique)" différent
# Sous Windows (Git Bash), `ln -s` copie le dossier au lieu de le lier.
if [ -L "$tmp/lien" ]; then
  check "chemin réel (lien symbolique résolu)" "$("$script" "$tmp/lien" || true)" "$a"
else
  echo "-     chemin réel (lien symbolique résolu) : lien symbolique indisponible, ignoré"
fi
check "barre finale sans effet" "$("$script" "$tmp/a/" || true)" "$a"
mkdir -p "$tmp/avec"
check "chemin avec espace pris en entier" \
  "$("$script" "$tmp/avec espace" || true)" \
  "target-$(printf '%s' "$(cd "$tmp/avec espace" && pwd -P)" | sha256sum | cut -c1-12)"
check "chemin avec espace distinct de son début" "$([ "$("$script" "$tmp/avec espace" || true)" != "$("$script" "$tmp/avec" || true)" ] && echo différent || echo identique)" différent
check "dossier absent refusé" "$(verdict "$script" "$tmp/absent")" refusé
check "argument obligatoire" "$(verdict "$script")" refusé

if [ "$failures" -ne 0 ]; then
  echo "$failures échec(s)" >&2
  exit 1
fi
