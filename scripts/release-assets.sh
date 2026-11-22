#!/usr/bin/env bash
# Fichiers d'une version publiée : rassemble à plat, dans <sortie>, tous les
# fichiers des artefacts téléchargés dans <entrée> (sous-dossiers compris,
# upload-artifact gardant la racine commune des chemins envoyés), puis écrit
# <sortie>/SHA256SUMS, une ligne par fichier, triée, au format de sha256sum.
# Tout cas douteux arrête la publication : nom inattendu, doublon, lien ou
# fichier spécial, SHA256SUMS déjà présent, sortie non vide.
#   ./scripts/release-assets.sh <entrée> <sortie>
set -euo pipefail
export LC_ALL=C

die() { echo "release-assets : $*" >&2; exit 1; }

[ $# -eq 2 ] || die "usage : release-assets.sh <entrée> <sortie>"
in="$1" out="$2"
[ -d "$in" ] || die "dossier d'entrée absent : $in"
case "$(realpath -m -- "$out")/" in
  "$(realpath -- "$in")/"*) die "la sortie ne peut pas être dans l'entrée" ;;
esac
if [ -e "$out" ] && [ -n "$(ls -A -- "$out")" ]; then die "dossier de sortie non vide : $out"; fi

odd="$(find "$in" -mindepth 1 ! -type d ! -type f -print -quit)"
[ -z "$odd" ] || die "lien ou fichier spécial refusé : $(printf '%q' "$odd")"

mkdir -p -- "$out"
# Noms déjà vus, en minuscules : deux noms égaux à la casse près ne
# cohabiteraient pas dans un dossier Windows.
declare -A seen=()
while IFS= read -r -d '' file; do
  name="${file##*/}"
  # Noms des fichiers de Tauri, de winget et du manifeste : ni espace, ni
  # saut de ligne (format de SHA256SUMS), ni tiret initial (options).
  [[ "$name" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]*$ ]] || die "nom refusé : $(printf '%q' "$name")"
  [ "$name" != SHA256SUMS ] || die "SHA256SUMS déjà présent dans les artefacts"
  [ -z "${seen[${name,,}]:-}" ] || die "deux fichiers nommés $name (casse ignorée)"
  seen[${name,,}]=1
  cp -- "$file" "$out/$name"
done < <(find "$in" -type f -print0)

cd -- "$out"
shopt -s nullglob
files=(*)
[ ${#files[@]} -gt 0 ] || die "aucun fichier à publier"
sha256sum -- "${files[@]}" >.SHA256SUMS.tmp
mv .SHA256SUMS.tmp SHA256SUMS
cat SHA256SUMS
