#!/usr/bin/env bash
# Tests de scripts/aur-push.sh (envoi du PKGBUILD sur l'AUR), contre un
# dépôt git nu local à la place de aur.archlinux.org. Lancé par
# scripts/check.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

script="$PWD/scripts/aur-push.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
failures=0
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1

fail() { echo "ÉCHEC : $*" >&2; failures=$((failures + 1)); }

# Nouveau cas : dépôt AUR vide.
setup() {
  rm -rf "$tmp/aur.git" "$tmp/pkg"
  git init --quiet --bare --initial-branch=master "$tmp/aur.git"
}

# Paquet à envoyer : version $1, pkgrel $2, contenu du PKGBUILD $3.
package() {
  rm -rf "$tmp/pkg"
  mkdir -p "$tmp/pkg"
  printf 'pkgbase = norelly-bin\n\tpkgver = %s\n\tpkgrel = %s\n\npkgname = norelly-bin\n' "$1" "$2" >"$tmp/pkg/.SRCINFO"
  printf '%s\n' "${3:-pkgver=$1}" >"$tmp/pkg/PKGBUILD"
}

# État de l'AUR posé directement : .SRCINFO brut $1.
seed() {
  rm -rf "$tmp/seed"
  git clone --quiet "$tmp/aur.git" "$tmp/seed" 2>/dev/null
  printf '%b' "$1" >"$tmp/seed/.SRCINFO"
  printf 'ancien\n' >"$tmp/seed/PKGBUILD"
  git -C "$tmp/seed" add -A
  git -C "$tmp/seed" -c user.name=t -c user.email=t@t commit --quiet -m seed
  git -C "$tmp/seed" push --quiet origin HEAD:master
}

commits() { git -C "$tmp/aur.git" rev-list --count master 2>/dev/null || echo 0; }

run_ok() {
  "$script" "$tmp/pkg" "$tmp/aur.git" >"$tmp/log" 2>&1 || { fail "$1 : refusé ($(cat "$tmp/log"))"; return 1; }
}

# Refus attendu pour la raison $2, sans nouveau commit sur l'AUR.
run_ko() {
  local before
  before="$(commits)"
  if "$script" "$tmp/pkg" "$tmp/aur.git" >"$tmp/log" 2>&1; then
    fail "$1 : accepté"
  elif ! grep -q -- "$2" "$tmp/log"; then
    fail "$1 : refusé pour une autre raison ($(cat "$tmp/log"))"
  elif [ "$(commits)" != "$before" ]; then
    fail "$1 : envoyé malgré le refus"
  fi
}

# Premier envoi : le dépôt AUR vide reçoit PKGBUILD et .SRCINFO.
setup
package 1.2.3 1
if run_ok "premier envoi"; then
  [ "$(commits)" = 1 ] || fail "premier envoi : $(commits) commit(s)"
  [ "$(git -C "$tmp/aur.git" show master:PKGBUILD)" = "pkgver=1.2.3" ] || fail "premier envoi : PKGBUILD"
  git -C "$tmp/aur.git" show master:.SRCINFO | grep -qx $'\tpkgver = 1.2.3' || fail "premier envoi : .SRCINFO"
  [ "$(git -C "$tmp/aur.git" log -1 --format=%s master)" = "norelly-bin 1.2.3-1" ] || fail "premier envoi : message"
fi

# Relance à l'identique : rien n'est envoyé.
if run_ok "relance à l'identique"; then
  [ "$(commits)" = 1 ] || fail "relance : nouveau commit"
  grep -q "déjà sur l'AUR" "$tmp/log" || fail "relance : message ($(cat "$tmp/log"))"
fi

# Version suivante : envoyée.
package 1.2.4 1
run_ok "version suivante" && { [ "$(commits)" = 2 ] || fail "version suivante : non envoyée"; }

# Version plus ancienne que celle de l'AUR : refusée.
package 1.2.3 1
run_ko "version plus ancienne" "plus récente"

# Même version, contenu différent : refusé (la somme publiée changerait
# sans changement de version).
package 1.2.4 1 "autre contenu"
run_ko "même version, autre contenu" "déjà sur l'AUR avec un autre contenu"

# Même version de l'application, révision d'empaquetage plus récente sur
# l'AUR : jamais remplacée par la révision 1.
setup
seed 'pkgbase = norelly-bin\n\tpkgver = 1.2.3\n\tpkgrel = 2\n'
package 1.2.3 1
run_ko "pkgrel plus récent sur l'AUR" "plus récente"

# Comparaison numérique, pas lexicale.
setup
seed 'pkgbase = norelly-bin\n\tpkgver = 1.2.9\n\tpkgrel = 1\n'
package 1.2.10 1
run_ok "1.2.10 après 1.2.9" && { [ "$(commits)" = 2 ] || fail "1.2.10 : non envoyée"; }

# .SRCINFO de l'AUR mal formé (donnée externe) : refusé avec un message
# fixe, sans recopier son contenu (commandes de workflow `::`).
for bad in 'pkgbase = x\n\tpkgver = 1.2.3\n\tpkgver = ::error::faux\n\tpkgrel = 1\n' \
           'pkgbase = x\n\tpkgver = 1.2.3x\n\tpkgrel = 1\n' \
           'pkgbase = x\n\tpkgrel = 1\n'; do
  setup
  seed "$bad"
  package 1.2.4 1
  run_ko ".SRCINFO de l'AUR mal formé" "version de l'AUR illisible"
  grep -q '::' "$tmp/log" && fail ".SRCINFO mal formé : contenu recopié"
done

# Paquet à envoyer mal formé : refusé.
setup
package 1.2.3-beta 1
run_ko "paquet à envoyer mal formé" "version du paquet à envoyer illisible"

# Clé d'hôte de l'AUR commitée : une seule ligne, ed25519, d'empreinte
# celle publiée sur la page d'accueil de aur.archlinux.org.
[ "$(wc -l <packaging/aur/known_hosts)" = 1 ] || fail "known_hosts : une seule ligne attendue"
[ "$(ssh-keygen -lf packaging/aur/known_hosts | cut -d' ' -f2-)" = "SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4 aur.archlinux.org (ED25519)" ] \
  || fail "known_hosts : empreinte différente de celle publiée par l'AUR"

if [ "$failures" -gt 0 ]; then
  echo "$failures échec(s)" >&2
  exit 1
fi
echo "aur-push.sh : tests réussis"
