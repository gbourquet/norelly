#!/usr/bin/env bash
# Tests de app/e2e/verify-crates.sh (archives .crate du cache de l'e2e
# revérifiées contre Cargo.lock), lancés par ./scripts/check.sh.
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/verify-crates.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failures=0
check() { # libellé, obtenu, attendu
  if [ "$2" = "$3" ]; then
    echo "ok    $1"
  else
    echo "ÉCHEC $1 : « $2 » au lieu de « $3 »" >&2
    failures=$((failures + 1))
  fi
}
present() { if [ -e "$1" ] || [ -L "$1" ]; then echo présent; else echo absent; fi; }
verdict() { if "$@" </dev/null >/dev/null 2>&1; then echo accepté; else echo refusé; fi; }
sum() { sha256sum "$1" | cut -d' ' -f1; }

registry="$tmp/registry/cache"
index="$registry/index.crates.io-1949cf8c6b5b557f"
mkdir -p "$index" "$tmp/ailleurs"
printf 'archive saine' > "$index/saine-1.0.0.crate"
printf 'archive empoisonnée' > "$index/empoisonnee-2.1.0.crate"
printf 'archive inconnue' > "$index/inconnue-0.1.0.crate"
printf 'archive saine' > "$tmp/ailleurs/cible"
ln -s "$tmp/ailleurs/cible" "$index/lien-1.0.0.crate"
cat > "$tmp/Cargo.lock" <<EOF
version = 4

[[package]]
name = "saine"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "$(sum "$index/saine-1.0.0.crate")"

[[package]]
name = "empoisonnee"
version = "2.1.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "$(printf 'archive attendue' | sha256sum | cut -d' ' -f1)"

[[package]]
name = "lien"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "$(sum "$tmp/ailleurs/cible")"

[[package]]
name = "absente"
version = "3.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "$(printf 'x' | sha256sum | cut -d' ' -f1)"

[[package]]
name = "locale"
version = "0.1.0"
dependencies = [
 "saine",
]
EOF

"$script" "$tmp/Cargo.lock" "$registry" >/dev/null || true

check "archive conforme gardée" "$(present "$index/saine-1.0.0.crate")" présent
check "archive différente de Cargo.lock retirée" "$(present "$index/empoisonnee-2.1.0.crate")" absent
check "archive hors de Cargo.lock gardée" "$(present "$index/inconnue-0.1.0.crate")" présent
# Sous Windows (Git Bash), `ln -s` copie le fichier au lieu de le lier.
if [ -L "$index/lien-1.0.0.crate" ] || [ ! -e "$index/lien-1.0.0.crate" ]; then
  check "lien symbolique retiré, cible intacte" "$(present "$index/lien-1.0.0.crate")/$(present "$tmp/ailleurs/cible")" absent/présent
else
  echo "-     liens symboliques : indisponibles, ignoré"
fi
# Sous Windows (Git Bash), sha256sum écrit « empreinte *chemin » (mode
# binaire) : simulé ici par un sha256sum qui force -b.
bin="$tmp/bin"
mkdir -p "$bin"
real_sha256sum=$(command -v sha256sum)
printf '#!/usr/bin/env bash\nexec %q -b "$@"\n' "$real_sha256sum" > "$bin/sha256sum"
chmod +x "$bin/sha256sum"
printf 'archive saine' > "$index/saine-1.0.0.crate"
printf 'archive empoisonnée' > "$index/empoisonnee-2.1.0.crate"
binary_log=$(PATH="$bin:$PATH" "$script" "$tmp/Cargo.lock" "$registry" 2>&1 >/dev/null || true)
check "mode binaire : archive conforme gardée" "$(present "$index/saine-1.0.0.crate")" présent
check "mode binaire : archive conforme non signalée" "$(grep -c saine-1.0.0 <<<"$binary_log" || true)" 0
check "mode binaire : archive différente retirée" "$(present "$index/empoisonnee-2.1.0.crate")" absent
# Une archive que sha256sum ne lit pas ne donne aucune ligne : refus, sans
# décaler les empreintes des archives suivantes.
rm -f "$index/lien-1.0.0.crate"
printf 'archive saine' > "$index/lien-1.0.0.crate"
printf '#!/usr/bin/env bash\necho "sha256sum: illisible" >&2\nexit 1\n' > "$bin/sha256sum"
check "sha256sum en échec refusé" "$(PATH="$bin:$PATH" verdict "$script" "$tmp/Cargo.lock" "$registry")" refusé
printf '#!/usr/bin/env bash\nshift\n%q -- "${@:2}"\n' "$real_sha256sum" > "$bin/sha256sum"
check "ligne manquante refusée" "$(PATH="$bin:$PATH" verdict "$script" "$tmp/Cargo.lock" "$registry")" refusé
check "registre absent accepté" "$(verdict "$script" "$tmp/Cargo.lock" "$tmp/absent")" accepté
check "Cargo.lock absent refusé" "$(verdict "$script" "$tmp/absent.lock" "$registry")" refusé
check "arguments obligatoires" "$(verdict "$script" "$tmp/Cargo.lock")" refusé

if [ "$failures" -ne 0 ]; then
  echo "$failures échec(s)" >&2
  exit 1
fi
