#!/usr/bin/env bash
# Tests end-to-end (tauri-driver + WebdriverIO), lancés par `pnpm -C app e2e`.
#
# - Construit l'image e2e/Dockerfile (WebKitWebDriver n'est pas disponible
#   sur toutes les distributions ; Ubuntu 24.04 le fournit).
# - Construit l'application dans le conteneur (accès réseau pour crates.io).
# - Démarre, sur un réseau Docker interne sans accès extérieur, Dovecot
#   (IMAP, imap.norelly.test) et Mailpit (SMTP, smtp.norelly.test), avec des
#   certificats autosignés générés pour l'occasion.
# - Lance les scénarios sur ce réseau interne (dépôt monté en lecture
#   seule), avec Xvfb, D-Bus et GNOME Keyring.
set -euo pipefail
cd "$(dirname "$0")/../.."
root=$(pwd)

image="norelly-e2e:$(sha256sum app/e2e/Dockerfile | cut -c1-12)"
run_id="norelly-e2e-$$"
network="$run_id"
cache="${XDG_CACHE_HOME:-$HOME/.cache}/norelly-e2e"
password="mot-de-passe-de-test"
# Images figées par empreinte (chaîne d'approvisionnement).
dovecot_image="dovecot/dovecot:2.4.5@sha256:43c14def08a57f25107593cbcf64a0a6fbae84a7b9de7aa038513c9b21e91914"
mailpit_image="axllent/mailpit:v1.33.0@sha256:9b3a463be15354c0f0eba91697d80e3e0d8a5623ef0e627cacc836bc17fd5231"
workdir=$(mktemp -d)

cleanup() {
  docker rm -f "$run_id-imap" "$run_id-smtp" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
  rm -rf "$workdir"
}
trap cleanup EXIT

if [ ! -d app/node_modules ]; then
  echo "Dépendances du front absentes : lancer d'abord pnpm -C app install" >&2
  exit 1
fi

if ! docker image inspect "$image" >/dev/null 2>&1; then
  echo "==> Construction de l'image $image"
  docker build -t "$image" app/e2e
fi

echo "==> Certificats de test"
for name in imap smtp; do
  openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=$name.norelly.test" \
    -addext "subjectAltName=DNS:$name.norelly.test" \
    -keyout "$workdir/$name.key" -out "$workdir/$name.crt" 2>/dev/null
  chmod 644 "$workdir/$name.key" "$workdir/$name.crt"
done
printf 'norelly:%s\n' "$password" > "$workdir/smtp-auth"
chmod 644 "$workdir/smtp-auth"

echo "==> Serveurs de test"
docker network create --internal "$network" >/dev/null
docker run -d --name "$run_id-imap" --network "$network" --network-alias imap.norelly.test \
  -e USER_PASSWORD="$password" \
  -v "$workdir/imap.crt:/etc/dovecot/ssl/tls.crt:ro" \
  -v "$workdir/imap.key:/etc/dovecot/ssl/tls.key:ro" \
  "$dovecot_image" >/dev/null
docker run -d --name "$run_id-smtp" --network "$network" --network-alias smtp.norelly.test \
  -e MP_SMTP_TLS_CERT=/certs/tls.crt -e MP_SMTP_TLS_KEY=/certs/tls.key \
  -e MP_SMTP_AUTH_FILE=/certs/auth -e MP_SMTP_REQUIRE_TLS=true \
  -v "$workdir/smtp.crt:/certs/tls.crt:ro" \
  -v "$workdir/smtp.key:/certs/tls.key:ro" \
  -v "$workdir/smtp-auth:/certs/auth:ro" \
  "$mailpit_image" >/dev/null

mkdir -p "$cache/target" "$cache/cargo" "$cache/home"
# Le cache n'est jamais placé dans le PATH : un binaire déposé là par un
# build ne peut pas remplacer pnpm ou cargo lors d'une exécution suivante.
common=(
  --rm --user "$(id -u):$(id -g)"
  -e HOME=/cache/home -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target
  -e RUSTUP_HOME=/opt/rustup
  -v "$cache:/cache" -w /work
)

echo "==> Build de l'application (conteneur, réseau extérieur pour crates.io)"
docker run "${common[@]}" ${CI:+-e CI=true} -v "$root:/work" \
  "$image" bash app/e2e/in-container.sh build

# Entrées passwd/group pour l'utilisateur courant : dbus-daemon refuse de
# démarrer si l'UID est inconnu (UID 1001 des runners GitHub, par exemple).
docker run --rm "$image" cat /etc/passwd > "$workdir/passwd"
docker run --rm "$image" cat /etc/group > "$workdir/group"
if ! cut -d: -f3 "$workdir/passwd" | grep -qx "$(id -u)"; then
  echo "e2e:x:$(id -u):$(id -g)::/tmp/home:/bin/bash" >> "$workdir/passwd"
fi
if ! cut -d: -f3 "$workdir/group" | grep -qx "$(id -g)"; then
  echo "e2e:x:$(id -g):" >> "$workdir/group"
fi
chmod 644 "$workdir/passwd" "$workdir/group"

echo "==> Scénarios (conteneur, réseau interne, dépôt en lecture seule)"
# HOME éphémère : base SQLite et trousseau neufs à chaque exécution (les
# certificats de test sont régénérés, une ancienne exception serait vue,
# à juste titre, comme un changement d'empreinte).
docker run "${common[@]}" --network "$network" \
  --tmpfs /tmp/home:exec,mode=1777 -e HOME=/tmp/home \
  -v "$workdir/passwd:/etc/passwd:ro" -v "$workdir/group:/etc/group:ro" \
  -e NORELLY_E2E_PASSWORD="$password" \
  -v "$root:/work:ro" \
  "$image" dbus-run-session -- bash app/e2e/in-container.sh scenarios
