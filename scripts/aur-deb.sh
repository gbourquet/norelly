#!/usr/bin/env bash
# .deb d'une version stable publiée, téléchargé puis vérifié avant d'être
# référencé par le PKGBUILD de l'AUR : sa somme dans SHA256SUMS, et
# l'attestation de provenance du .deb et de SHA256SUMS, émise par le
# workflow de publication à ce tag et à ce commit (même vérification que
# le job `publish` de release.yml).
#   ./scripts/aur-deb.sh <tag vX.Y.Z> <commit du tag, empreinte complète> <dossier de sortie>
# Utilisé par le job `aur` de release.yml, et à la main pour une version
# publiée avant ce job (docs/technique/methode.md, « Paquet AUR »).
set -euo pipefail
tag="$1" commit="$2" out="$3"
repo=gbourquet/norelly
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "tag de version stable attendu : « $tag »" >&2; exit 1; }
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || { echo "empreinte de commit complète attendue : « $commit »" >&2; exit 1; }
version="${tag#v}"
deb="Norelly_${version}_amd64.deb" bundle="Norelly_${version}_provenance.sigstore.json"

mkdir -p "$out"
cd "$out"
gh release download "$tag" --repo "$repo" --clobber \
  --pattern "$deb" --pattern SHA256SUMS --pattern "$bundle"
grep -qxF -- "$(sha256sum -- "$deb")" SHA256SUMS \
  || { echo "$deb absent de SHA256SUMS ou somme différente" >&2; exit 1; }
for file in "$deb" SHA256SUMS; do
  gh attestation verify "$file" --bundle "$bundle" --repo "$repo" \
    --cert-identity "https://github.com/$repo/.github/workflows/release.yml@refs/tags/$tag" \
    --source-ref "refs/tags/$tag" --source-digest "$commit" \
    --deny-self-hosted-runners \
    || { echo "attestation de provenance refusée pour $file" >&2; exit 1; }
done
echo "$deb vérifié dans $out"
