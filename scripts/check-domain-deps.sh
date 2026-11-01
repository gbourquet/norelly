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

[ "$status" -eq 0 ] && echo "norelly-domain : règle de dépendance respectée"
exit "$status"
