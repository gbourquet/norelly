#!/usr/bin/env python3
"""Sources vendorisées du Flatpak (build hors ligne, ADR-006).

Lit `Cargo.lock` et `app/pnpm-lock.yaml` et écrit, dans
`packaging/flatpak/` :

- `cargo-sources.json` : une archive `.crate` par paquet de crates.io,
  vérifiée par la somme SHA-256 du lockfile, dépaquetée dans
  `cargo/vendor/<nom>-<version>`, avec son `.cargo-checksum.json` ; pour
  un dépôt git admis par `allow-git` de deny.toml et figé par commit, le
  dépôt cloné à ce commit et ses crates recopiées dans `cargo/vendor/<nom>` ;
  puis la configuration cargo qui remplace crates.io et ces dépôts par ce
  répertoire ;
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
"""

CARGO_CONFIG_END = """
[net]
offline = true
"""

# Source git du Cargo.lock : `git+<url>?rev=<commit>#<commit résolu>`.
GIT_SOURCE = re.compile(r"git\+(https://[^?#\s]+)\?rev=([0-9a-f]{40})#([0-9a-f]{40})")
CRATE_NAME = re.compile(r"[A-Za-z0-9_-]+")


def allowed_git(deny_toml: str) -> list[str]:
    """Dépôts git admis : la liste `[sources] allow-git` de deny.toml."""
    return list(tomllib.loads(deny_toml).get("sources", {}).get("allow-git", []))


def git_sources(name: str, source: str, allowed: list[str], cloned: set[str]) -> list[dict]:
    """Crate d'un dépôt git admis : le dépôt cloné à son commit (une fois),
    puis le dossier de la crate recopié dans le répertoire vendorisé.

    Une crate git est rangée sous `cargo/vendor/<nom>` : les dépendances
    `path = "../<nom>"` entre crates du même dépôt s'y retrouvent. Le
    manifeste hérite de `rust-version` du workspace du dépôt, absent du
    répertoire vendorisé : la ligne est retirée, et le build échoue si un
    autre héritage subsiste."""
    url = source.removeprefix("git+").split("?", 1)[0]
    if url not in allowed:
        raise SystemExit(f"source git non admise (absente de allow-git) : {name} ({source})")
    match = GIT_SOURCE.fullmatch(source)
    if match is None or match.group(1) != url or match.group(2) != match.group(3):
        raise SystemExit(f"source git non figée par commit : {name} ({source})")
    if not CRATE_NAME.fullmatch(name):
        raise SystemExit(f"nom de crate invalide : {name}")
    rev = match.group(2)
    clone = f"cargo/git/{url.rstrip('/').rsplit('/', 1)[-1]}-{rev[:12]}"
    vendor = f"cargo/vendor/{name}"
    sources: list[dict] = []
    if clone not in cloned:
        cloned.add(clone)
        sources.append({"type": "git", "url": url, "commit": rev, "dest": clone})
    sources.append(
        {
            "type": "shell",
            "commands": [
                f'cp -r "{clone}/{name}" "{vendor}"',
                f'sed -i "/^rust-version\\.workspace = true$/d" "{vendor}/Cargo.toml"',
                f'! grep -q "workspace" "{vendor}/Cargo.toml"',
            ],
        }
    )
    sources.append(
        {
            "type": "inline",
            "contents": json.dumps({"package": None, "files": {}}),
            "dest": vendor,
            "dest-filename": ".cargo-checksum.json",
        }
    )
    return sources


def cargo_sources(lock: dict, allowed: list[str]) -> list[dict]:
    sources: list[dict] = []
    git_config: dict[str, tuple[str, str]] = {}
    cloned: set[str] = set()
    for package in lock.get("package", []):
        source = package.get("source")
        if source is None:
            continue  # paquet du workspace
        if source.startswith("git+"):
            sources.extend(git_sources(package["name"], source, allowed, cloned))
            url, rev = source.removeprefix("git+").split("#", 1)[0].split("?rev=", 1)
            git_config[f"git+{url}?rev={rev}"] = (url, rev)
            continue
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
    config = CARGO_CONFIG
    for key, (url, rev) in git_config.items():
        config += f'\n[source."{key}"]\ngit = "{url}"\nrev = "{rev}"\nreplace-with = "vendored-sources"\n'
    config += CARGO_CONFIG_END
    sources.append(
        {"type": "inline", "contents": config, "dest": "cargo", "dest-filename": "config.toml"}
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
        OUT / "cargo-sources.json": render(
            cargo_sources(
                tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8")),
                allowed_git((ROOT / "deny.toml").read_text(encoding="utf-8")),
            )
        ),
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
