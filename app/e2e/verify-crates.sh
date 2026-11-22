#!/usr/bin/env bash
# Revérifie contre Cargo.lock les archives .crate du cache de cargo de
# l'e2e, partagé entre les copies de travail (et restauré par la CI).
#
# cargo ne contrôle l'empreinte d'une archive qu'à son téléchargement : une
# archive déjà présente dans registry/cache est extraite telle quelle. Une
# archive d'un paquet de Cargo.lock dont l'empreinte SHA-256 diffère de
# `checksum`, ou qui n'est pas un fichier ordinaire, est donc retirée : cargo
# la télécharge et la vérifie à nouveau. Les archives des paquets absents
# de ce Cargo.lock (ceux d'une autre branche) sont laissées, vérifiées par
# le lancement qui les utilise.
#
# Usage : verify-crates.sh <Cargo.lock> <dossier registry/cache>
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage : verify-crates.sh <Cargo.lock> <dossier registry/cache>" >&2
  exit 2
fi
lock=$1
registry=$2
if [ ! -f "$lock" ]; then
  echo "Cargo.lock introuvable : $lock" >&2
  exit 1
fi
[ -d "$registry" ] || exit 0

# Paquets du registre : « nom version empreinte », un par ligne.
packages=$(awk '
  function flush() {
    if (name != "" && version != "" && checksum != "" && source ~ /^registry\+/)
      print name, version, checksum
    name = version = source = checksum = ""
  }
  /^\[\[package\]\]/ { flush(); next }
  /^name = "/     { name = $3;     gsub(/"/, "", name) }
  /^version = "/  { version = $3;  gsub(/"/, "", version) }
  /^source = "/   { source = $3;   gsub(/"/, "", source) }
  /^checksum = "/ { checksum = $3; gsub(/"/, "", checksum) }
  END { flush() }
' "$lock")

declare -A expected=()
files=()
removed=0
remove() {
  echo "Archive différente de Cargo.lock, retirée : $1" >&2
  rm -rf -- "$1"
  removed=$((removed + 1))
}
while read -r name version checksum; do
  [ -n "$name" ] || continue
  # Valeurs lues dans un Cargo.lock de branche : forme contrôlée avant de
  # composer un chemin.
  [[ $name =~ ^[A-Za-z0-9_-]+$ && $version =~ ^[0-9A-Za-z.+-]+$ && $checksum =~ ^[0-9a-f]{64}$ ]] || continue
  for archive in "$registry"/*/"$name-$version.crate"; do
    if [ -L "$archive" ] || { [ -e "$archive" ] && [ ! -f "$archive" ]; }; then
      remove "$archive"
    elif [ -f "$archive" ]; then
      expected[$archive]=$checksum
      files+=("$archive")
    fi
  done
done <<< "$packages"

# Une seule passe de sha256sum, une ligne par fichier dans l'ordre donné.
# Seule l'empreinte est lue : le chemin qu'il réécrit (« * » du mode
# binaire sous Windows, « \ » en tête d'une ligne échappée) n'est jamais
# relu. Une ligne illisible ne donne aucune empreinte : archive retirée,
# puis téléchargée et vérifiée par cargo. Un échec de sha256sum, ou une
# ligne manquante qui décalerait les empreintes, arrête tout.
if [ "${#files[@]}" -ne 0 ]; then
  sums=$(sha256sum -- "${files[@]}") || {
    echo "sha256sum a échoué sur le cache de cargo" >&2
    exit 1
  }
  mapfile -t lines <<< "$sums"
  if [ "${#lines[@]}" -ne "${#files[@]}" ]; then
    echo "sha256sum : ${#lines[@]} empreinte(s) pour ${#files[@]} archive(s)" >&2
    exit 1
  fi
  for i in "${!files[@]}"; do
    archive=${files[i]}
    line=${lines[i]#\\}
    [ "${line:0:64}" = "${expected[$archive]}" ] || remove "$archive"
  done
fi
if [ "$removed" -ne 0 ]; then
  echo "==> $removed archive(s) .crate retirée(s), à télécharger à nouveau"
fi
