#!/usr/bin/env bash
# Tests end-to-end (tauri-driver + WebdriverIO), lancés par `pnpm -C app e2e`.
#
# - Construit l'image e2e/Dockerfile (WebKitWebDriver n'est pas disponible
#   sur toutes les distributions ; Ubuntu 24.04 le fournit).
# - Démarre, sur un réseau Docker isolé, Dovecot (IMAP, imap.norelly.test)
#   et Mailpit (SMTP, smtp.norelly.test), avec des certificats autosignés
#   générés pour l'occasion.
# - Construit l'application et lance les scénarios dans le conteneur, avec
#   Xvfb, D-Bus et GNOME Keyring.
set -euo pipefail
cd "$(dirname "$0")/../.."
root=$(pwd)

image="norelly-e2e:$(sha256sum app/e2e/Dockerfile | cut -c1-12)"
run_id="norelly-e2e-$$"
network="$run_id"
cache="${XDG_CACHE_HOME:-$HOME/.cache}/norelly-e2e"
password="mot-de-passe-de-test"
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
docker network create "$network" >/dev/null
docker run -d --name "$run_id-imap" --network "$network" --network-alias imap.norelly.test \
  -e USER_PASSWORD="$password" \
  -v "$workdir/imap.crt:/etc/dovecot/ssl/tls.crt:ro" \
  -v "$workdir/imap.key:/etc/dovecot/ssl/tls.key:ro" \
  dovecot/dovecot:2.4.5 >/dev/null
docker run -d --name "$run_id-smtp" --network "$network" --network-alias smtp.norelly.test \
  -e MP_SMTP_TLS_CERT=/certs/tls.crt -e MP_SMTP_TLS_KEY=/certs/tls.key \
  -e MP_SMTP_AUTH_FILE=/certs/auth -e MP_SMTP_REQUIRE_TLS=true \
  -v "$workdir/smtp.crt:/certs/tls.crt:ro" \
  -v "$workdir/smtp.key:/certs/tls.key:ro" \
  -v "$workdir/smtp-auth:/certs/auth:ro" \
  axllent/mailpit:v1.33.0 >/dev/null

echo "==> Build et scénarios (conteneur)"
mkdir -p "$cache/target" "$cache/cargo" "$cache/home"
docker run --rm --network "$network" \
  --user "$(id -u):$(id -g)" \
  -e HOME=/cache/home -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target \
  -e PATH=/cache/cargo/bin:/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  -e RUSTUP_HOME=/opt/rustup \
  -e NORELLY_E2E_PASSWORD="$password" \
  ${CI:+-e CI=true} \
  -v "$root:/work" -v "$cache:/cache" -w /work \
  "$image" dbus-run-session -- bash app/e2e/in-container.sh
