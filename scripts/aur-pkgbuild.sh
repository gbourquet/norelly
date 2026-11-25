#!/usr/bin/env bash
# Fichiers du dépôt AUR de norelly-bin pour une version stable publiée :
# PKGBUILD et .SRCINFO, modèles de packaging/aur remplis avec la version et
# les sommes SHA-256 du .deb publié et de la licence MIT du dépôt ; licence
# 0BSD des fichiers du paquet (règles de l'AUR), dans la disposition de
# `pkgctl license setup` : LICENSE et REUSE.toml copiés tels quels,
# LICENSES/0BSD.txt lien vers ../LICENSE (créé ici, pas commité : une copie
# de travail Windows le transformerait en fichier).
#   ./scripts/aur-pkgbuild.sh <version> <Norelly_<version>_amd64.deb> <dossier de sortie>
set -euo pipefail
version="$1" deb="$2" out="$3"
# Version stable seulement : le tiret d'une préversion est interdit dans
# pkgver, et l'AUR la proposerait comme une version ordinaire. Forme
# stricte : la version entre dans un PKGBUILD que makepkg exécute.
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version stable attendue : « $version »" >&2; exit 1; }
[ "$(basename -- "$deb")" = "Norelly_${version}_amd64.deb" ] \
  || { echo "Norelly_${version}_amd64.deb attendu : $deb" >&2; exit 1; }
[ -f "$deb" ] || { echo ".deb introuvable : $deb" >&2; exit 1; }
root="$(cd "$(dirname "$0")/.." && pwd)"
sha256_deb="$(sha256sum -- "$deb" | cut -d' ' -f1)"
sha256_license="$(sha256sum -- "$root/LICENSE-MIT" | cut -d' ' -f1)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
for f in PKGBUILD .SRCINFO; do
  sed -e "s/{{version}}/$version/g" -e "s/{{sha256_deb}}/$sha256_deb/g" \
    -e "s/{{sha256_license}}/$sha256_license/g" "$root/packaging/aur/$f" >"$tmp/$f"
done
if grep -q '{{' "$tmp/PKGBUILD" "$tmp/.SRCINFO"; then echo "modèle AUR incomplet" >&2; exit 1; fi
mkdir -p "$out"
cp "$tmp/PKGBUILD" "$tmp/.SRCINFO" "$root/packaging/aur/LICENSE" "$root/packaging/aur/REUSE.toml" "$out/"
mkdir -p "$out/LICENSES"
ln -sfn ../LICENSE "$out/LICENSES/0BSD.txt"
echo "Paquet AUR norelly-bin $version dans $out"
