#!/usr/bin/env bash
# Tests de scripts/aur-pkgbuild.sh (PKGBUILD et .SRCINFO du paquet AUR
# norelly-bin), sur un .deb factice. Lancé par scripts/check.sh ; le
# contrôle par makepkg et namcap, sous Arch, est dans
# scripts/check-aur.sh (Docker).
set -euo pipefail
cd "$(dirname "$0")/.."

script="$PWD/scripts/aur-pkgbuild.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
failures=0

fail() { echo "ÉCHEC : $*" >&2; failures=$((failures + 1)); }

# Nouveau cas : .deb factice de la version $1, sortie vide.
setup() {
  rm -rf "$tmp/in" "$tmp/out"
  mkdir -p "$tmp/in"
  deb="$tmp/in/Norelly_${1}_amd64.deb"
  printf 'deb %s' "$1" >"$deb"
}

run_ok() {
  "$script" "$2" "$deb" "$tmp/out" >"$tmp/log" 2>&1 || { fail "$1 : refusé ($(cat "$tmp/log"))"; return 1; }
}

# Le script doit échouer pour la raison attendue ($3, motif du message),
# sans rien écrire.
run_ko() {
  if "$script" "$2" "$deb" "$tmp/out" >"$tmp/log" 2>&1; then
    fail "$1 : accepté"
  elif ! grep -q -- "$3" "$tmp/log"; then
    fail "$1 : refusé pour une autre raison ($(cat "$tmp/log"))"
  elif [ -e "$tmp/out/PKGBUILD" ] || [ -e "$tmp/out/.SRCINFO" ]; then
    fail "$1 : fichiers écrits malgré le refus"
  fi
}

# Version stable : PKGBUILD et .SRCINFO seulement, remplis, avec la somme
# du .deb et celle de la licence MIT du dépôt.
setup 1.2.3
if run_ok "version stable" 1.2.3; then
  actual="$(cd "$tmp/out" && LC_ALL=C ls -A)"
  [ "$actual" = "$(printf '.SRCINFO\nPKGBUILD')" ] || fail "fichiers écrits : $actual"
  deb_sum="$(sha256sum "$deb" | cut -d' ' -f1)"
  mit_sum="$(sha256sum LICENSE-MIT | cut -d' ' -f1)"
  url="https://github.com/gbourquet/norelly/releases/download/v1.2.3/Norelly_1.2.3_amd64.deb"
  for f in PKGBUILD .SRCINFO; do
    grep -q '{{' "$tmp/out/$f" && fail "$f : modèle incomplet"
    grep -q "$deb_sum" "$tmp/out/$f" || fail "$f : somme du .deb absente"
    grep -q "$mit_sum" "$tmp/out/$f" || fail "$f : somme de LICENSE-MIT absente"
  done
  grep -qx 'pkgver=1.2.3' "$tmp/out/PKGBUILD" || fail "PKGBUILD : pkgver"
  grep -qx 'pkgrel=1' "$tmp/out/PKGBUILD" || fail "PKGBUILD : pkgrel"
  grep -qx 'pkgbase = norelly-bin' "$tmp/out/.SRCINFO" || fail ".SRCINFO : pkgbase"
  grep -qx $'\tpkgver = 1.2.3' "$tmp/out/.SRCINFO" || fail ".SRCINFO : pkgver"
  grep -qxF $'\tsource = '"$url" "$tmp/out/.SRCINFO" || fail ".SRCINFO : source du .deb"
  grep -qxF $'\tsource = LICENSE-MIT-1.2.3::https://raw.githubusercontent.com/gbourquet/norelly/v1.2.3/LICENSE-MIT' "$tmp/out/.SRCINFO" \
    || fail ".SRCINFO : source de la licence"
fi

# Préversion : le tiret est interdit dans pkgver, et l'AUR la proposerait
# comme une version ordinaire.
setup 1.2.3-beta.1
run_ko "préversion" 1.2.3-beta.1 "version stable attendue"

# Version mal formée (injection dans le PKGBUILD, exécuté par makepkg).
setup 1.2.3
# shellcheck disable=SC2016 # chaîne d'injection, littérale exprès
run_ko "version mal formée" '1.2.3$(id)' "version stable attendue"
run_ko "version vide" '' "version stable attendue"

# .deb d'une autre version : la somme ne correspondrait pas au fichier
# téléchargé.
setup 1.2.3
run_ko ".deb d'une autre version" 1.2.4 "Norelly_1.2.4_amd64.deb attendu"

# .deb absent.
setup 1.2.3
rm "$deb"
run_ko ".deb absent" 1.2.3 "introuvable"

if [ "$failures" -gt 0 ]; then
  echo "$failures échec(s)" >&2
  exit 1
fi
echo "aur-pkgbuild.sh : tests réussis"
