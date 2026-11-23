#!/usr/bin/env bash
# PKGBUILD et .SRCINFO du paquet AUR norelly-bin d'une version stable
# publiée : remplit les modèles de packaging/aur avec la version et les
# sommes SHA-256 du .deb publié et de la licence MIT du dépôt.
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
cp "$tmp/PKGBUILD" "$tmp/.SRCINFO" "$out/"
echo "PKGBUILD et .SRCINFO de norelly-bin $version dans $out"
