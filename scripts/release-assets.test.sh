#!/usr/bin/env bash
# Tests de scripts/release-assets.sh (fichiers d'une version et SHA256SUMS),
# sur des fichiers factices. Lancé par scripts/check.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

script="$PWD/scripts/release-assets.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
failures=0

fail() { echo "ÉCHEC : $*" >&2; failures=$((failures + 1)); }

# Nouveau cas : dossiers d'entrée et de sortie vides.
setup() { rm -rf "$tmp/in" "$tmp/out"; mkdir -p "$tmp/in"; }

# Le script doit réussir ; sa sortie d'erreur est gardée pour le diagnostic.
run_ok() {
  "$script" "$tmp/in" "$tmp/out" >"$tmp/log" 2>&1 || { fail "$1 : refusé ($(cat "$tmp/log"))"; return 1; }
}

# Le script doit échouer pour la raison attendue ($2, motif du message),
# sans laisser de SHA256SUMS. Entrée et sortie : $3 et $4 s'ils sont donnés.
run_ko() {
  local in="${3:-$tmp/in}" out="${4:-$tmp/out}"
  if "$script" "$in" "$out" >"$tmp/log" 2>&1; then
    fail "$1 : accepté"
  elif ! grep -q -- "$2" "$tmp/log"; then
    fail "$1 : refusé pour une autre raison ($(cat "$tmp/log"))"
  elif [ -e "$out/SHA256SUMS" ]; then
    fail "$1 : SHA256SUMS écrit malgré le refus"
  fi
}

# Fichiers des artefacts de la CI, dont les paquets Linux rangés dans des
# sous-dossiers (racine commune de upload-artifact) : tous à plat, sommés.
setup
mkdir -p "$tmp/in/deb" "$tmp/in/rpm"
printf 'deb' >"$tmp/in/deb/Norelly_1.2.3_amd64.deb"
printf 'rpm' >"$tmp/in/rpm/Norelly-1.2.3-1.x86_64.rpm"
printf 'exe' >"$tmp/in/Norelly_1.2.3_x64-setup.exe"
printf 'sig' >"$tmp/in/Norelly_1.2.3_x64-setup.exe.sig"
printf '{}' >"$tmp/in/latest.json"
if run_ok "artefacts de la CI" && { [ -f "$tmp/out/SHA256SUMS" ] || { fail "SHA256SUMS absent"; false; }; }; then
  expected="Norelly-1.2.3-1.x86_64.rpm
Norelly_1.2.3_amd64.deb
Norelly_1.2.3_x64-setup.exe
Norelly_1.2.3_x64-setup.exe.sig
SHA256SUMS
latest.json"
  actual="$(cd "$tmp/out" && LC_ALL=C ls -A)"
  [ "$actual" = "$expected" ] || fail "fichiers publiés : $actual"
  # Une ligne par fichier publié (sauf SHA256SUMS), triée, au format de
  # sha256sum, vérifiable par sha256sum -c.
  names="$(cut -d' ' -f3- "$tmp/out/SHA256SUMS")"
  [ "$names" = "$(grep -vx SHA256SUMS <<<"$expected")" ] || fail "noms de SHA256SUMS : $names"
  grep -qx "$(printf 'exe' | sha256sum | cut -d' ' -f1)  Norelly_1.2.3_x64-setup.exe" "$tmp/out/SHA256SUMS" \
    || fail "somme de l'installateur absente ou fausse"
  (cd "$tmp/out" && sha256sum --check --strict --quiet SHA256SUMS) || fail "sha256sum -c refuse SHA256SUMS"
  # Contenu copié à l'identique.
  [ "$(cat "$tmp/out/Norelly_1.2.3_amd64.deb")" = deb ] || fail "contenu du .deb modifié"
fi

# Rien à publier.
setup
run_ko "dossier d'entrée vide" "aucun fichier"

# Deux artefacts du même nom : l'un écraserait l'autre.
setup
mkdir -p "$tmp/in/a" "$tmp/in/b"
printf '1' >"$tmp/in/a/latest.json"
printf '2' >"$tmp/in/b/latest.json"
run_ko "deux fichiers du même nom" "deux fichiers nommés latest.json"

# Un SHA256SUMS venu d'un artefact serait remplacé sans bruit.
setup
printf 'faux' >"$tmp/in/SHA256SUMS"
run_ko "SHA256SUMS dans les artefacts" "SHA256SUMS déjà présent"

# Nom qui casserait le format de SHA256SUMS ou la commande de publication.
setup
printf 'x' >"$tmp/in/nom avec espace.deb"
run_ko "nom avec une espace" "nom refusé"
setup
printf 'x' >"$tmp/in/$(printf 'a\nb.deb')"
run_ko "nom avec un saut de ligne" "nom refusé"
setup
printf 'x' >"$tmp/in/-n.deb"
run_ko "nom commençant par un tiret" "nom refusé"

# Lien symbolique : il ferait publier un fichier hors des artefacts.
setup
printf 'secret' >"$tmp/hors.txt"
ln -s "$tmp/hors.txt" "$tmp/in/lien.deb"
run_ko "lien symbolique" "lien ou fichier spécial"

# Fichier spécial (tube nommé) : ni copié, ni publié.
setup
mkfifo "$tmp/in/tube.deb"
run_ko "tube nommé" "lien ou fichier spécial"

# Dossier de sortie déjà rempli : des restes seraient publiés.
setup
printf 'x' >"$tmp/in/a.deb"
mkdir -p "$tmp/out"
printf 'reste' >"$tmp/out/ancien.deb"
run_ko "dossier de sortie non vide" "sortie non vide"

# Entrée absente.
setup
run_ko "entrée absente" "dossier d'entrée absent" "$tmp/absent"

# Sortie égale à l'entrée, ou dedans : les copies seraient relues.
setup
printf 'x' >"$tmp/in/a.deb"
run_ko "sortie égale à l'entrée" "dans l'entrée" "$tmp/in" "$tmp/in"
setup
printf 'x' >"$tmp/in/a.deb"
run_ko "sortie dans l'entrée" "dans l'entrée" "$tmp/in" "$tmp/in/dist"

# Arguments manquants.
if "$script" >"$tmp/log" 2>&1; then
  fail "sans argument : accepté"
elif ! grep -q usage "$tmp/log"; then
  fail "sans argument : refusé pour une autre raison ($(cat "$tmp/log"))"
fi

if [ "$failures" -ne 0 ]; then
  echo "$failures échec(s)" >&2
  exit 1
fi
echo "release-assets : tous les cas passent"
