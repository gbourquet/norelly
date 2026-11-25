#!/usr/bin/env bash
# Envoi des fichiers de norelly-bin (PKGBUILD, .SRCINFO, LICENSE,
# REUSE.toml, lien LICENSES/0BSD.txt) sur l'AUR, seulement s'ils sont strictement plus récents que
# ceux de l'AUR.
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

files=(PKGBUILD .SRCINFO LICENSE REUSE.toml LICENSES/0BSD.txt)
for f in "${files[@]}"; do
  [ -f "$pkg/$f" ] || { echo "$f absent de $pkg" >&2; exit 1; }
done

# Même fichier : même contenu, et même cible pour un lien symbolique.
same_file() {
  if [ -L "$1" ] || [ -L "$2" ]; then
    [ -L "$1" ] && [ -L "$2" ] && [ "$(readlink "$1")" = "$(readlink "$2")" ]
  else
    cmp -s "$1" "$2"
  fi
}
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
    same=true
    for f in "${files[@]}"; do same_file "$pkg/$f" "$f" || same=false; done
    if "$same"; then
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

for f in "${files[@]}"; do
  mkdir -p "$(dirname "$f")"
  rm -f -- "$f"
  cp -P -- "$pkg/$f" "$f"
done
git add -- "${files[@]}"
git -c user.name="Norelly" -c user.email="contact@norelly.fr" \
  commit --quiet -m "norelly-bin $new"
git push --quiet origin HEAD:master
echo "norelly-bin $new envoyé sur l'AUR"
