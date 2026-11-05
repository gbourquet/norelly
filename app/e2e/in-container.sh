#!/usr/bin/env bash
# Exécuté dans le conteneur e2e (voir run.sh) :
#   build      construit l'application (debug, front embarqué) ;
#   scenarios  lance WebdriverIO, sous dbus-run-session, sans réseau extérieur.
set -euo pipefail

case "${1:-}" in
  build)
    pnpm -C app tauri build --debug --no-bundle
    # Contrat du trousseau contre le vrai Secret Service, compilé ici
    # (réseau disponible), joué avec les scénarios.
    cargo test -p norelly-keyring --test integration_secret_service --no-run --locked
    # Mesure de performance (NORELLY_E2E_PERF=1, job nocturne) : sur le
    # produit livré, c'est-à-dire un build release, avec la base de mesure.
    if [ "${NORELLY_E2E_PERF:-}" = "1" ]; then
      pnpm -C app tauri build --no-bundle
      cargo build -p norelly-sqlite --example seed_perf --locked
    fi
    ;;
  scenarios)
    # Trousseau : GNOME Keyring déverrouillé (collection « login » créée au
    # premier lancement), pour exercer le vrai Secret Service.
    mkdir -p "$HOME"
    eval "$(printf 'norelly-e2e' | gnome-keyring-daemon --unlock --components=secrets)"
    NORELLY_SECRET_SERVICE_TEST=1 cargo test -p norelly-keyring \
      --test integration_secret_service --offline --locked
    export NORELLY_APP_BINARY="$CARGO_TARGET_DIR/debug/norelly"
    cd app
    xvfb-run -a --server-args="-screen 0 1280x900x24" \
      node_modules/.bin/wdio run e2e/wdio.conf.mjs
    # Captures d'écran de la documentation utilisateur, une fois par
    # langue, chacune sur un profil neuf (question du premier lancement
    # comprise). Langue imposée par LANGUAGE (lue par WebKitGTK), vérifiée
    # par le scénario. Sortie : /captures/<langue>, recopiée par run.sh.
    data="$HOME/.local/share/io.github.gbourquet.Norelly"
    for lang in fr en; do
      rm -rf "$data" "$HOME/.config/io.github.gbourquet.Norelly" "$HOME/.cache/io.github.gbourquet.Norelly"
      mkdir -p "/captures/$lang"
      case "$lang" in
        fr) locale_env=(LANG=fr_FR.UTF-8 LANGUAGE=fr) ;;
        en) locale_env=(LANG=C.UTF-8 LANGUAGE=en_GB:en) ;;
      esac
      env "${locale_env[@]}" NORELLY_E2E_LANG="$lang" NORELLY_E2E_CAPTURES="/captures/$lang" \
        xvfb-run -a --server-args="-screen 0 1280x900x24" \
        node_modules/.bin/wdio run e2e/wdio.conf.mjs --spec e2e/captures/captures.e2e.mjs
    done
    # Mesure de bout en bout (job nocturne) : profil neuf, base de 50 000
    # messages, scénario de performance seul.
    if [ "${NORELLY_E2E_PERF:-}" = "1" ]; then
      rm -rf "$data"
      "$CARGO_TARGET_DIR/debug/examples/seed_perf" "$data/norelly.sqlite3"
      NORELLY_APP_BINARY="$CARGO_TARGET_DIR/release/norelly" \
        xvfb-run -a --server-args="-screen 0 1280x900x24" \
        node_modules/.bin/wdio run e2e/wdio.conf.mjs --spec e2e/perf/perf.e2e.mjs
    fi
    ;;
  *)
    echo "usage : in-container.sh build|scenarios" >&2
    exit 2
    ;;
esac
