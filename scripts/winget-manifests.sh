#!/usr/bin/env bash
# Manifestes winget d'une version publiée : remplit les modèles de
# packaging/winget avec la version, la date et la somme SHA-256 de
# l'installateur NSIS.
#   ./scripts/winget-manifests.sh <version> <installateur.exe> <dossier de sortie>
set -euo pipefail
version="$1" installer="$2" out="$3"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.+-][0-9A-Za-z.+-]+)?$ ]] || { echo "version invalide : $version" >&2; exit 1; }
sha256="$(sha256sum "$installer" | cut -d' ' -f1 | tr 'a-f' 'A-F')"
date="$(date -u +%Y-%m-%d)"
dir="$(dirname "$0")/../packaging/winget"
mkdir -p "$out"
for f in "$dir"/*.yaml; do
  sed -e "s/{{version}}/$version/g" -e "s/{{sha256}}/$sha256/g" -e "s/{{date}}/$date/g" "$f" >"$out/$(basename "$f")"
done
if grep -rq '{{' "$out"; then echo "modèle winget incomplet" >&2; exit 1; fi
echo "Manifestes winget dans $out"
