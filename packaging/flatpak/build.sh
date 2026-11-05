#!/bin/sh
# Build hors ligne de Norelly dans le bac à sable de flatpak-builder (aucun
# accès réseau). Les dépendances viennent des sources vendorisées :
# - npm : archives de node-sources.json, servies par un registre local sur
#   la boucle locale du bac à sable, vérifiées par pnpm (intégrité du
#   lockfile, --frozen-lockfile) ;
# - cargo : crates de cargo-sources.json (CARGO_HOME/config.toml remplace
#   crates.io par cargo/vendor).
set -eu

id=io.github.gbourquet.Norelly
export CARGO_HOME="$PWD/cargo"
export npm_config_update_notifier=false

python3 -m http.server --bind 127.0.0.1 --directory npm-registry 4873 >/dev/null 2>&1 &
registry=$!
trap 'kill "$registry" 2>/dev/null || true' EXIT
for _ in 1 2 3 4 5 6 7 8 9 10; do
  python3 -c 'import socket; socket.create_connection(("127.0.0.1", 4873), 1)' 2>/dev/null && break
  sleep 0.5
done

node pnpm/bin/pnpm.cjs -C app install --frozen-lockfile --ignore-scripts \
  --registry=http://127.0.0.1:4873/ --store-dir="$PWD/pnpm-store"
node pnpm/bin/pnpm.cjs -C app build

# custom-protocol : interface embarquée (comme `tauri build`), pas devUrl.
cargo build --release --locked --offline -p norelly-app --features tauri/custom-protocol

install -Dm755 target/release/norelly-app /app/bin/norelly
sed -e 's/{{exec}}/norelly/' -e "s/{{icon}}/$id/" packaging/linux/norelly.desktop \
  | install -Dm644 /dev/stdin "/app/share/applications/$id.desktop"
install -Dm644 "packaging/linux/$id.metainfo.xml" "/app/share/metainfo/$id.metainfo.xml"
for size in 32x32 64x64 128x128; do
  install -Dm644 "app/src-tauri/icons/$size.png" "/app/share/icons/hicolor/$size/apps/$id.png"
done
install -Dm644 app/src-tauri/icons/128x128@2x.png "/app/share/icons/hicolor/256x256/apps/$id.png"
install -Dm644 app/src-tauri/icons/icon.png "/app/share/icons/hicolor/512x512/apps/$id.png"
