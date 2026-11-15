#!/usr/bin/env bash
# Règle de dépendance de l'architecture hexagonale (ADR-008) : le crate
# domain ne dépend que de thiserror et serde (et de leurs dépendances
# transitives). Toute autre dépendance directe fait échouer la vérification.
set -euo pipefail
cd "$(dirname "$0")/.."

allowed="thiserror serde"
direct=$(cargo tree -p norelly-domain --edges normal --depth 1 --prefix none --format '{p}' \
  | tail -n +2 | awk '{print $1}' | sort -u)

status=0
for dep in $direct; do
  if ! grep -qw -- "$dep" <<<"$allowed"; then
    echo "ERREUR : dépendance interdite dans norelly-domain : $dep (ADR-008)" >&2
    status=1
  fi
done

# Garde-fou supplémentaire : aucune bibliothèque d'E/S, même transitive.
forbidden="tokio rusqlite tauri reqwest lettre imap-next rustls hyper"
all=$(cargo tree -p norelly-domain --edges normal --prefix none --format '{p}' | awk '{print $1}' | sort -u)
for dep in $forbidden; do
  if grep -qx -- "$dep" <<<"$all"; then
    echo "ERREUR : bibliothèque d'E/S dans l'arbre de norelly-domain : $dep (ADR-008)" >&2
    status=1
  fi
done

# Relecture tolérante d'une fiche (`ContactDraft::from_stored`) réservée à
# l'adaptateur SQLite et aux tests du domaine : toute saisie passe par le
# constructeur qui refuse. Liste d'autorisation : partout ailleurs (autres
# adaptateurs, application, Tauri, code non test du domaine), un appel ou
# un alias de `ContactDraft` est refusé. `DisplayName::from_stored` (nom
# d'un compte) n'est pas concerné.
misuse=$(
  find crates app/src-tauri -name '*.rs' -not -path '*/target/*' -print0 \
    | xargs -0 awk '
        FNR == 1 { in_test = (FILENAME ~ /\/tests(\/|\.rs$)/) }
        /#\[cfg\(test\)\]/ { in_test = 1 }
        FILENAME ~ /^crates\/adapters\/sqlite\/src\// { next }
        FILENAME == "crates/domain/src/account.rs" { next }
        /^[[:space:]]*\/\// { next }
        /ContactDraft[[:space:]]+as[[:space:]]/ { print FILENAME ":" FNR ": alias de ContactDraft"; next }
        /from_stored/ {
          if ($0 ~ /DisplayName::from_stored/) next
          if (FILENAME == "crates/domain/src/contacts.rs" && $0 ~ /fn from_stored/) next
          if (FILENAME ~ /^crates\/domain\/src\// && in_test) next
          print FILENAME ":" FNR ": " $0
        }'
)
if [ -n "$misuse" ]; then
  printf '%s\n' "$misuse" >&2
  echo "ERREUR : ContactDraft::from_stored hors de l'adaptateur SQLite et des tests du domaine (ContactDraft::new pour une saisie)" >&2
  status=1
fi

[ "$status" -eq 0 ] && echo "norelly-domain : règle de dépendance respectée"
exit "$status"
