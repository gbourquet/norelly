#!/usr/bin/env bash
# Métadonnées de bureau (phase 10) : fichier .desktop et AppStream valides,
# chaque texte présenté traduit en français (ADR-011), icônes aux bonnes
# tailles. Outils : desktop-file-validate (desktop-file-utils) et
# appstreamcli (appstream).
set -euo pipefail
cd "$(dirname "$0")/.."

dir=packaging/linux
id=io.github.gbourquet.Norelly
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Modèle commun (Tauri le remplit pour .deb et .rpm ; Flatpak par sed).
sed -e 's/{{exec}}/norelly/' -e "s/{{icon}}/$id/" "$dir/norelly.desktop" >"$tmp/$id.desktop"
desktop-file-validate "$tmp/$id.desktop"
appstreamcli validate --no-net --explain "$dir/$id.metainfo.xml"

python3 -I - "$dir/norelly.desktop" "$dir/$id.metainfo.xml" <<'PY'
import sys
import xml.etree.ElementTree as ET

desktop, metainfo = sys.argv[1], sys.argv[2]
errors = []

# .desktop : chaque clé présentée a sa traduction française.
keys = {}
for line in open(desktop, encoding="utf-8"):
    if "=" in line and not line.startswith("#"):
        k = line.split("=", 1)[0]
        keys[k] = True
for k in ("GenericName", "Comment", "Keywords"):
    if k not in keys or f"{k}[fr]" not in keys:
        errors.append(f"{desktop} : {k} ou {k}[fr] absent")

# AppStream : autant de textes français que de textes par défaut, pour le
# résumé, les paragraphes et les éléments de liste.
LANG = "{http://www.w3.org/XML/1998/namespace}lang"
root = ET.parse(metainfo).getroot()
for path in ("summary", "description/p", "description/ul/li", "releases/release/description/p"):
    found = root.findall(path)
    default = [e for e in found if e.get(LANG) is None]
    french = [e for e in found if e.get(LANG) == "fr"]
    if not default or len(default) != len(french):
        errors.append(f"{metainfo} : {path} : {len(default)} texte(s), {len(french)} en français")

if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)
PY

# Icônes : tailles attendues par les thèmes d'icônes (hicolor).
for f in 32x32 64x64 128x128; do
  file "app/src-tauri/icons/$f.png" | grep -q "${f/x/ x }," || { echo "icône $f : taille inattendue" >&2; exit 1; }
done
file app/src-tauri/icons/128x128@2x.png | grep -q "256 x 256" || { echo "icône 256 : taille inattendue" >&2; exit 1; }
file app/src-tauri/icons/icon.png | grep -q "512 x 512" || { echo "icône 512 : taille inattendue" >&2; exit 1; }

echo "Métadonnées de bureau valides"
