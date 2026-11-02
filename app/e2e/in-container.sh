#!/usr/bin/env bash
# Exécuté dans le conteneur e2e, sous dbus-run-session (voir run.sh).
set -euo pipefail

# Trousseau : GNOME Keyring déverrouillé (collection « login » créée au
# premier lancement), pour exercer le vrai Secret Service.
mkdir -p "$HOME"
eval "$(printf 'norelly-e2e' | gnome-keyring-daemon --unlock --components=secrets)"
export GNOME_KEYRING_CONTROL SSH_AUTH_SOCK 2>/dev/null || true


echo "==> Build de l'application (debug, front embarqué)"
pnpm -C app tauri build --debug --no-bundle

export NORELLY_APP_BINARY="$CARGO_TARGET_DIR/debug/norelly-app"
echo "==> Scénarios WebdriverIO"
xvfb-run -a --server-args="-screen 0 1280x900x24" pnpm -C app exec wdio run e2e/wdio.conf.ts
