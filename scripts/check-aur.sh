#!/usr/bin/env bash
# Paquet AUR norelly-bin contrôlé sous Arch Linux (Docker), sur un .deb
# factice de même structure que celui de Tauri : .SRCINFO identique à
# `makepkg --printsrcinfo`, PKGBUILD et paquet sans erreur namcap, fichiers
# du dépôt AUR conformes à `pkgctl license check` (licence 0BSD d'Arch,
# REUSE ; règles de l'AUR), paquet
# construit et installé avec les fichiers attendus. Lancé par
# `scripts/check.sh full`.
#
# Image figée par empreinte, dépôts figés à une date de l'archive Arch :
# relever les deux ensemble (`docker pull archlinux:base-devel`, puis
# `docker image inspect --format '{{index .RepoDigests 0}}'` et
# `--format '{{.Created}}'`). Date de l'archive : celle de l'image, une fois
# son instantané complet (le lendemain) ; plus ancienne, pacman rétrograde
# des paquets et en télécharge davantage.
set -euo pipefail
cd "$(dirname "$0")/.."

image="archlinux:base-devel@sha256:949ecb1661609ab7e8f6854ba03efd62792e7a796313738e94b040c095c20d91"
archive="https://archive.archlinux.org/repos/2026/11/23"

docker run --rm -i -v "$PWD:/src:ro" -e ARCHIVE="$archive" "$image" bash -euo pipefail -s <<'SH'
  echo "Server = $ARCHIVE/\$repo/os/\$arch" >/etc/pacman.d/mirrorlist
  # L'archive Arch répond parfois en près de 10 s, délai de connexion de
  # pacman (non réglable) : trois tentatives.
  for attempt in 1 2 3; do
    pacman -Syyuu --noconfirm --needed --disable-download-timeout namcap reuse devtools >/dev/null && break
    [ "$attempt" -lt 3 ] || { echo "archive Arch injoignable après 3 tentatives" >&2; exit 1; }
    sleep 15
  done
  useradd -m builder
  work=/home/builder/aur version=1.2.3
  mkdir -p "$work/deb/usr/bin" "$work/deb/usr/share/applications"
  printf "#!/bin/sh\n" >"$work/deb/usr/bin/norelly"
  chmod 755 "$work/deb/usr/bin/norelly"
  cp /src/packaging/linux/norelly.desktop "$work/deb/usr/share/applications/Norelly.desktop"
  for size in 32x32 128x128 256x256@2 512x512; do
    mkdir -p "$work/deb/usr/share/icons/hicolor/$size/apps"
    cp /src/app/src-tauri/icons/icon.png "$work/deb/usr/share/icons/hicolor/$size/apps/norelly.png"
  done
  (cd "$work/deb" && tar czf ../data.tar.gz usr && printf "2.0\n" >../debian-binary \
    && printf "Package: norelly\n" >../control && tar czf ../control.tar.gz -C .. control)
  deb="$work/Norelly_${version}_amd64.deb"
  (cd "$work" && ar rc "$deb" debian-binary control.tar.gz data.tar.gz)

  /src/scripts/aur-pkgbuild.sh "$version" "$deb" "$work/pkg"
  # Les fichiers envoyés sur l'AUR, seuls : contrôle officiel d'Arch.
  mkdir "$work/lint"
  cp -P "$work/pkg/"{PKGBUILD,.SRCINFO,LICENSE,REUSE.toml} "$work/lint/"
  cp -RP "$work/pkg/LICENSES" "$work/lint/"
  (cd "$work/lint" && pkgctl license check) || { echo "fichiers du dépôt AUR : pkgctl license check refuse" >&2; exit 1; }
  # Sources déjà présentes : makepkg ne télécharge rien.
  cp "$deb" "$work/pkg/"
  cp /src/LICENSE-MIT "$work/pkg/LICENSE-MIT-$version"
  chown -R builder: "$work"
  cd "$work/pkg"

  runuser -u builder -- makepkg --printsrcinfo | diff -u .SRCINFO - \
    || { echo ".SRCINFO différent de makepkg --printsrcinfo" >&2; exit 1; }
  check_namcap() {
    namcap "$1" | tee namcap.log
    if grep -q " E: " namcap.log; then echo "namcap : erreur sur $1" >&2; exit 1; fi
  }
  check_namcap PKGBUILD
  # Dépendances non installées (le binaire factice n'en a pas besoin).
  runuser -u builder -- makepkg --nodeps --noconfirm >/dev/null
  pkg=(norelly-bin-"$version"-1-x86_64.pkg.tar.*)
  check_namcap "${pkg[0]}"
  pacman -Udd --noconfirm "${pkg[0]}" >/dev/null
  expected="/usr/bin/norelly
/usr/share/applications/Norelly.desktop
/usr/share/icons/hicolor/128x128/apps/norelly.png
/usr/share/icons/hicolor/256x256/apps/norelly.png
/usr/share/icons/hicolor/32x32/apps/norelly.png
/usr/share/icons/hicolor/512x512/apps/norelly.png
/usr/share/licenses/norelly-bin/LICENSE-MIT"
  actual="$(pacman -Qlq norelly-bin | grep -v "/$" | LC_ALL=C sort)"
  [ "$actual" = "$expected" ] || { printf "fichiers installés :\n%s\n" "$actual" >&2; exit 1; }
  [ -x /usr/bin/norelly ] || { echo "/usr/bin/norelly non exécutable" >&2; exit 1; }
  pacman -Qi norelly-bin | grep -q "^Provides *: norelly$" || { echo "provides norelly absent" >&2; exit 1; }
SH
echo "Paquet AUR norelly-bin valide"
