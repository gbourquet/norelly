#!/usr/bin/env bash
# Tests de scripts/aur-deb.sh (.deb d'une version publiée, vérifié), avec
# un faux `gh` : aucun accès réseau. Lancé par scripts/check.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

script="$PWD/scripts/aur-deb.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
failures=0
commit=0123456789abcdef0123456789abcdef01234567

fail() { echo "ÉCHEC : $*" >&2; failures=$((failures + 1)); }

# Faux gh : `release download` copie les fichiers de $FAKE_RELEASE dans le
# dossier courant ; `attestation verify` note ses arguments et réussit
# sauf si FAKE_ATTEST_FAIL vaut le nom du fichier vérifié.
mkdir -p "$tmp/bin"
cat >"$tmp/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
case "$1 $2" in
  "release download")
    cp "$FAKE_RELEASE"/* .
    printf '%s\n' "$*" >>"$FAKE_LOG.download" ;;
  "attestation verify")
    printf '%s\n' "$*" >>"$FAKE_LOG"
    [ "${FAKE_ATTEST_FAIL:-}" != "$3" ] ;;
  *) echo "faux gh : commande inattendue $*" >&2; exit 2 ;;
esac
GH
chmod +x "$tmp/bin/gh"
export PATH="$tmp/bin:$PATH" FAKE_RELEASE="$tmp/release" FAKE_LOG="$tmp/gh.log"

# Nouveau cas : version publiée 1.2.3 cohérente.
setup() {
  rm -rf "$tmp/release" "$tmp/out" "$tmp/gh.log" "$tmp/gh.log.download"
  unset FAKE_ATTEST_FAIL
  mkdir -p "$tmp/release"
  printf 'deb' >"$tmp/release/Norelly_1.2.3_amd64.deb"
  (cd "$tmp/release" && sha256sum Norelly_1.2.3_amd64.deb >SHA256SUMS)
  printf '{}' >"$tmp/release/Norelly_1.2.3_provenance.sigstore.json"
}

run_ok() {
  "$script" "$2" "$3" "$tmp/out" >"$tmp/log" 2>&1 || { fail "$1 : refusé ($(cat "$tmp/log"))"; return 1; }
}

run_ko() {
  if "$script" "$2" "$3" "$tmp/out" >"$tmp/log" 2>&1; then
    fail "$1 : accepté"
  elif ! grep -q -- "$4" "$tmp/log"; then
    fail "$1 : refusé pour une autre raison ($(cat "$tmp/log"))"
  fi
}

# Cas nominal : le .deb est vérifié par sa somme puis par l'attestation,
# avec l'identité exacte du workflow de publication à ce tag et ce commit.
setup
if run_ok "version publiée" v1.2.3 "$commit"; then
  [ -f "$tmp/out/Norelly_1.2.3_amd64.deb" ] || fail ".deb absent de la sortie"
  for f in Norelly_1.2.3_amd64.deb SHA256SUMS; do
    grep -qxF "attestation verify $f --bundle Norelly_1.2.3_provenance.sigstore.json --repo gbourquet/norelly --cert-identity https://github.com/gbourquet/norelly/.github/workflows/release.yml@refs/tags/v1.2.3 --source-ref refs/tags/v1.2.3 --source-digest $commit --deny-self-hosted-runners" "$tmp/gh.log" \
      || fail "attestation de $f non vérifiée comme attendu ($(cat "$tmp/gh.log" 2>/dev/null))"
  done
  grep -q -- "--repo gbourquet/norelly" "$tmp/gh.log.download" || fail "téléchargement hors du dépôt gbourquet/norelly"
fi

# Somme différente de celle de SHA256SUMS.
setup
printf 'autre' >"$tmp/release/Norelly_1.2.3_amd64.deb"
run_ko "somme différente" v1.2.3 "$commit" "absent de SHA256SUMS ou somme différente"

# .deb absent de SHA256SUMS.
setup
: >"$tmp/release/SHA256SUMS"
run_ko ".deb absent de SHA256SUMS" v1.2.3 "$commit" "absent de SHA256SUMS ou somme différente"

# Attestation refusée, pour le .deb ou pour SHA256SUMS.
for f in Norelly_1.2.3_amd64.deb SHA256SUMS; do
  setup
  export FAKE_ATTEST_FAIL="$f"
  run_ko "attestation refusée ($f)" v1.2.3 "$commit" "attestation"
done

# Tag d'une préversion, ou mal formé : refusé avant tout téléchargement.
# shellcheck disable=SC2016 # chaîne d'injection, littérale exprès
for tag in v1.2.3-beta.1 '1.2.3' 'v1.2.3$(id)' ''; do
  setup
  run_ko "tag « $tag »" "$tag" "$commit" "tag de version stable attendu"
  [ -e "$tmp/gh.log.download" ] && fail "tag « $tag » : téléchargement lancé"
done

# Commit qui n'est pas une empreinte complète.
setup
run_ko "commit abrégé" v1.2.3 0123456 "empreinte de commit complète attendue"

if [ "$failures" -gt 0 ]; then
  echo "$failures échec(s)" >&2
  exit 1
fi
echo "aur-deb.sh : tests réussis"
