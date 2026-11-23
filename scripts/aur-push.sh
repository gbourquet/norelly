#!/usr/bin/env bash
# Envoi du PKGBUILD et du .SRCINFO de norelly-bin sur l'AUR, seulement
# s'ils sont strictement plus récents que ceux de l'AUR.
#   ./scripts/aur-push.sh <dossier du paquet> <dépôt AUR>
# Dépôt AUR : ssh://aur@aur.archlinux.org/norelly-bin.git (clé SSH par
# GIT_SSH_COMMAND), ou un dépôt local dans les tests.
#
# Version comparée sur `pkgver-pkgrel` : jamais de retour en arrière, ni
# vers une version plus ancienne, ni vers une révision d'empaquetage plus
# ancienne de la même version. Même version et même contenu (relance) :
# rien n'est envoyé ; même version, autre contenu : refusé.
set -euo pipefail
pkg="$1" remote="$2"

# Version `pkgver-pkgrel` d'un .SRCINFO, ou rien s'il est illisible : une
# seule ligne de chaque, de forme stricte (le .SRCINFO de l'AUR est une
# donnée externe, jamais recopiée dans les messages).
version_of() {
  local ver rel
  ver="$(sed -n 's/^\tpkgver = //p' "$1")"
  rel="$(sed -n 's/^\tpkgrel = //p' "$1")"
  [[ "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] && [[ "$rel" =~ ^[1-9][0-9]*$ ]] || return 0
  printf '%s-%s' "$ver" "$rel"
}

[ -f "$pkg/PKGBUILD" ] && [ -f "$pkg/.SRCINFO" ] || { echo "PKGBUILD ou .SRCINFO absent de $pkg" >&2; exit 1; }
new="$(version_of "$pkg/.SRCINFO")"
[ -n "$new" ] || { echo "version du paquet à envoyer illisible" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
git clone --quiet -- "$remote" "$work/aur" 2>"$work/clone.log" \
  || { cat "$work/clone.log" >&2; echo "dépôt AUR inaccessible" >&2; exit 1; }
cd "$work/aur"

if [ -e .SRCINFO ]; then
  current="$(version_of .SRCINFO)"
  [ -n "$current" ] || { echo "version de l'AUR illisible : envoi refusé" >&2; exit 1; }
  if [ "$current" = "$new" ]; then
    if cmp -s "$pkg/PKGBUILD" PKGBUILD && cmp -s "$pkg/.SRCINFO" .SRCINFO; then
      echo "norelly-bin $new déjà sur l'AUR"
      exit 0
    fi
    echo "norelly-bin $new déjà sur l'AUR avec un autre contenu : envoi refusé" >&2
    exit 1
  fi
  if [ "$(printf '%s\n%s\n' "$current" "$new" | sort -V | tail -1)" != "$new" ]; then
    echo "l'AUR propose $current, plus récente que $new : envoi refusé" >&2
    exit 1
  fi
fi

cp -- "$pkg/PKGBUILD" "$pkg/.SRCINFO" .
git add PKGBUILD .SRCINFO
git -c user.name="Norelly" -c user.email="contact@norelly.fr" \
  commit --quiet -m "norelly-bin $new"
git push --quiet origin HEAD:master
echo "norelly-bin $new envoyé sur l'AUR"
