#!/usr/bin/env python3
"""Sources vendorisées du Flatpak (build hors ligne, ADR-006).

Lit `Cargo.lock` et `app/pnpm-lock.yaml` et écrit, dans
`packaging/flatpak/` :

- `cargo-sources.json` : une archive `.crate` par paquet de crates.io,
  vérifiée par la somme SHA-256 du lockfile, dépaquetée dans
  `cargo/vendor/<nom>-<version>`, avec son `.cargo-checksum.json`, et la
  configuration cargo qui remplace crates.io par ce répertoire ;
- `node-sources.json` : une archive `.tgz` par paquet npm, vérifiée par
  l'intégrité SHA-512 du lockfile, rangée dans `npm-registry/` selon les
  chemins du registre npm (`<nom>/-/<base>-<version>.tgz`) ; le build sert
  ce répertoire en local (`packaging/flatpak/build.sh`).

Bibliothèque standard seulement. `--check` : échoue si les fichiers commités
ne correspondent plus aux lockfiles (exécuté par scripts/check-packaging.sh).
"""

from __future__ import annotations

import base64
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "packaging" / "flatpak"
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
NPM_REGISTRY = "https://registry.npmjs.org"

CARGO_CONFIG = """[source.vendored-sources]
directory = "cargo/vendor"

[source.crates-io]
replace-with = "vendored-sources"

[net]
offline = true
"""


def cargo_sources() -> list[dict]:
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    sources: list[dict] = []
    for package in lock.get("package", []):
        source = package.get("source")
        if source is None:
            continue  # paquet du workspace
        if source != CRATES_IO:
            raise SystemExit(f"source non prise en charge : {package['name']} ({source})")
        name, version, checksum = package["name"], package["version"], package["checksum"]
        if not re.fullmatch(r"[0-9a-f]{64}", checksum):
            raise SystemExit(f"somme invalide : {name} {version}")
        dest = f"cargo/vendor/{name}-{version}"
        sources.append(
            {
                "type": "archive",
                "archive-type": "tar-gzip",
                "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                "sha256": checksum,
                "dest": dest,
            }
        )
        sources.append(
            {
                "type": "inline",
                "contents": json.dumps({"package": checksum, "files": {}}),
                "dest": dest,
                "dest-filename": ".cargo-checksum.json",
            }
        )
    sources.append(
        {"type": "inline", "contents": CARGO_CONFIG, "dest": "cargo", "dest-filename": "config.toml"}
    )
    return sources


# Clé d'un paquet de la section `packages` (lockfile v9), par exemple
# `  '@scope/nom@1.2.3':` ou `  nom@1.2.3:`.
PACKAGE_KEY = re.compile(r"^  '?((?:@[^@/']+/)?[^@/']+)@([^'():]+)'?:$")
RESOLUTION = re.compile(r"^    resolution: \{integrity: (sha512-[A-Za-z0-9+/=]+)\}$")


def node_sources() -> list[dict]:
    lines = (ROOT / "app" / "pnpm-lock.yaml").read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != "lockfileVersion: '9.0'":
        raise SystemExit("pnpm-lock.yaml : version de lockfile non prise en charge")
    try:
        start = lines.index("packages:")
    except ValueError:
        raise SystemExit("pnpm-lock.yaml : section packages absente") from None
    sources: list[dict] = []
    current: tuple[str, str] | None = None
    for line in lines[start + 1 :]:
        if line and not line.startswith(" "):
            break  # section suivante (snapshots)
        key = PACKAGE_KEY.match(line)
        if key:
            current = (key.group(1), key.group(2))
            continue
        if line.startswith("    resolution:"):
            resolution = RESOLUTION.match(line)
            if current is None or resolution is None:
                raise SystemExit(f"pnpm-lock.yaml : résolution non prise en charge : {line.strip()}")
            name, version = current
            base = name.rsplit("/", 1)[-1]
            digest = base64.b64decode(resolution.group(1).removeprefix("sha512-"), validate=True)
            if len(digest) != 64:
                raise SystemExit(f"intégrité invalide : {name} {version}")
            sources.append(
                {
                    "type": "file",
                    "url": f"{NPM_REGISTRY}/{name}/-/{base}-{version}.tgz",
                    "sha512": digest.hex(),
                    "dest": f"npm-registry/{name}/-",
                    "dest-filename": f"{base}-{version}.tgz",
                }
            )
            current = None
    if not sources:
        raise SystemExit("pnpm-lock.yaml : aucun paquet")
    return sources


def render(sources: list[dict]) -> str:
    return json.dumps(sources, indent=2, ensure_ascii=False) + "\n"


def main() -> int:
    check = sys.argv[1:] == ["--check"]
    if sys.argv[1:] not in ([], ["--check"]):
        print("usage : flatpak-sources.py [--check]", file=sys.stderr)
        return 2
    outputs = {
        OUT / "cargo-sources.json": render(cargo_sources()),
        OUT / "node-sources.json": render(node_sources()),
    }
    stale = []
    for path, content in outputs.items():
        if check:
            if not path.exists() or path.read_text(encoding="utf-8") != content:
                stale.append(path.relative_to(ROOT))
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
    if stale:
        names = ", ".join(str(p) for p in stale)
        print(f"Sources Flatpak périmées ({names}) : lancer ./scripts/flatpak-sources.py", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
