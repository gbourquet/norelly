#!/usr/bin/env bash
# Exécuté dans le conteneur e2e (voir run.sh) :
#   build      construit l'application (debug, front embarqué) ;
#   scenarios  lance WebdriverIO, sous dbus-run-session, sans réseau extérieur.
set -euo pipefail

case "${1:-}" in
  build)
    pnpm -C app tauri build --debug --no-bundle
    ;;
  scenarios)
    # Trousseau : GNOME Keyring déverrouillé (collection « login » créée au
    # premier lancement), pour exercer le vrai Secret Service.
    mkdir -p "$HOME"
    eval "$(printf 'norelly-e2e' | gnome-keyring-daemon --unlock --components=secrets)"
    export NORELLY_APP_BINARY="$CARGO_TARGET_DIR/debug/norelly-app"
    cd app
    xvfb-run -a --server-args="-screen 0 1280x900x24" \
      node_modules/.bin/wdio run e2e/wdio.conf.mjs
    ;;
  *)
    echo "usage : in-container.sh build|scenarios" >&2
    exit 2
    ;;
esac
