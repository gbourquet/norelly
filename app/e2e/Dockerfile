# Environnement des tests end-to-end (ADR-007) : tauri-driver + WebKitWebDriver
# (webkit2gtk-driver), Xvfb, D-Bus et GNOME Keyring (vrai Secret Service).
# Construit et utilisé par e2e/run.sh ; aucune donnée réelle. Locale fr_FR
# générée : les scénarios tournent en français (ADR-011).
FROM ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852

ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git build-essential pkg-config file \
      libwebkit2gtk-4.1-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev \
      libdbus-1-dev libssl-dev \
      webkit2gtk-driver xvfb xauth dbus dbus-x11 gnome-keyring libsecret-tools \
      fonts-dejavu-core locales \
    && rm -rf /var/lib/apt/lists/* \
    && sed -i 's/^# *fr_FR.UTF-8 UTF-8/fr_FR.UTF-8 UTF-8/' /etc/locale.gen \
    && locale-gen

# Rust (version stable figée par rust-toolchain du runner : on suit stable).
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain stable \
    && chmod -R a+rwX /opt/rustup /opt/cargo
RUN cargo install tauri-driver --version 2.1.0 --locked \
    && rm -rf /opt/cargo/registry /opt/cargo/git && chmod -R a+rwX /opt/cargo

# Node.js 24 et pnpm 10.
RUN curl -fsSL https://deb.nodesource.com/setup_24.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/* \
    && npm install -g pnpm@10.34.9
