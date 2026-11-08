# Environnement des tests end-to-end (ADR-007) : tauri-driver + WebKitWebDriver
# (webkit2gtk-driver), Xvfb, D-Bus et GNOME Keyring (vrai Secret Service) ;
# xdotool pour le sélecteur de fichiers natif, que WebDriver ne voit pas.
# Construit et utilisé par e2e/run.sh ; aucune donnée réelle. Locale fr_FR
# générée : les scénarios tournent en français (ADR-011).
FROM ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852

ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git build-essential pkg-config file \
      libwebkit2gtk-4.1-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev \
      libdbus-1-dev libssl-dev \
      webkit2gtk-driver xvfb xauth xdotool dbus dbus-x11 gnome-keyring libsecret-tools \
      fonts-dejavu-core locales xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && sed -i 's/^# *fr_FR.UTF-8 UTF-8/fr_FR.UTF-8 UTF-8/' /etc/locale.gen \
    && locale-gen

# Outils téléchargés en archives figées, vérifiées par SHA-256 : aucun script exécuté depuis le
# réseau. Mise à jour : changer la version et l'empreinte ensemble (empreintes
# publiées : `rustup-init.sha256`, `SHASUMS256.txt` de Node.js, champ
# `dist.integrity` du registre npm pour pnpm).
ARG RUSTUP_VERSION=1.29.3
ARG RUSTUP_SHA256=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55
ARG NODE_VERSION=24.22.2
ARG NODE_SHA256=48cc0e21e048790e54c0cf5749c2725005313d652651c40b7a21feb9697aaf75
ARG PNPM_VERSION=10.34.9
ARG PNPM_SHA256=7e8a946108e7baab119e51d27c24d6178b46545422643dbc91e34e2f1ee32e0f

# Rust (on suit stable, comme la CI).
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/node/bin:/opt/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -fsSLo /tmp/rustup-init \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
    && echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain stable \
    && rm /tmp/rustup-init \
    && chmod -R a+rwX /opt/rustup /opt/cargo
RUN cargo install tauri-driver --version 2.1.0 --locked \
    && rm -rf /opt/cargo/registry /opt/cargo/git && chmod -R a+rwX /opt/cargo

# Node.js 24 et pnpm 10.
RUN curl --proto '=https' --tlsv1.2 -fsSLo /tmp/node.tar.xz \
      "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
    && echo "${NODE_SHA256}  /tmp/node.tar.xz" | sha256sum -c - \
    && mkdir -p /opt/node && tar -xJf /tmp/node.tar.xz -C /opt/node --strip-components=1 \
    && rm /tmp/node.tar.xz \
    && curl --proto '=https' --tlsv1.2 -fsSLo /tmp/pnpm.tgz \
      "https://registry.npmjs.org/pnpm/-/pnpm-${PNPM_VERSION}.tgz" \
    && echo "${PNPM_SHA256}  /tmp/pnpm.tgz" | sha256sum -c - \
    && npm install -g --ignore-scripts /tmp/pnpm.tgz \
    && rm /tmp/pnpm.tgz
