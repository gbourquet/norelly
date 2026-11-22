#!/usr/bin/env python3
"""Manifeste Flathub de Norelly, produit depuis le manifeste du dépôt.

Le manifeste du dépôt (`packaging/flatpak/io.github.gbourquet.Norelly.yml`)
construit l'arbre local (source `type: dir`). Flathub exige une source
`git` figée par un tag et son commit : ce script lit, **au commit du tag**,
le manifeste et les sources vendorisées (`cargo-sources.json`,
`node-sources.json`), remplace la source locale par

    - type: git
      url: https://github.com/gbourquet/norelly.git
      tag: "<tag>"
      commit: "<commit du tag>"

et écrit les trois fichiers dans `--out` (en général un clone du dépôt
Flathub de l'application ; ses autres fichiers sont laissés tels quels).
flatpak-builder vérifie à la récupération que le tag désigne bien ce
commit.

Usage :
    flathub-manifest.py --tag vX.Y.Z --out DOSSIER [--repo DÉPÔT]
    flathub-manifest.py --check   (le manifeste du dépôt se transforme ;
                                   exécuté par scripts/check-packaging.sh)

Le tag doit être `v` suivi de la version de `app/src-tauri/tauri.conf.json`
au même commit (règle de `release.yml`). Bibliothèque standard seulement.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FLATPAK = "packaging/flatpak"
MANIFEST = "io.github.gbourquet.Norelly.yml"
VENDORED = ("cargo-sources.json", "node-sources.json")
TAURI_CONF = "app/src-tauri/tauri.conf.json"
UPSTREAM = "https://github.com/gbourquet/norelly.git"

TAG = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?")
COMMIT = re.compile(r"[0-9a-f]{40}")
# Source locale : l'élément `- type: dir` de la liste `sources` du module
# et ses lignes plus indentées, jusqu'à l'élément suivant.
SOURCE_ITEM = "      - "
DIR_SOURCE = SOURCE_ITEM + "type: dir\n"

HEADER = (
    "# Manifeste Flathub de Norelly, produit par scripts/flathub-manifest.py\n"
    "# depuis packaging/flatpak/io.github.gbourquet.Norelly.yml au tag {tag} :\n"
    "# ne pas le modifier à la main, le régénérer.\n"
)


class ManifestError(Exception):
    pass


def to_flathub(manifest: str, tag: str, commit: str) -> str:
    """Manifeste Flathub : en-tête remplacé, source locale remplacée par le
    tag et son commit, tout le reste à l'identique."""
    if not TAG.fullmatch(tag):
        raise ManifestError(f"tag invalide : {tag!r} (attendu : vX.Y.Z)")
    if not COMMIT.fullmatch(commit):
        raise ManifestError(f"commit invalide : {commit!r} (40 chiffres hexadécimaux)")
    lines = manifest.splitlines(keepends=True)
    starts = [i for i, line in enumerate(lines) if line == DIR_SOURCE]
    if len(starts) != 1:
        raise ManifestError(f"{len(starts)} source(s) `type: dir` dans le manifeste, 1 attendue")
    start = starts[0]
    end = start + 1
    while end < len(lines) and lines[end].startswith(" " * len(SOURCE_ITEM)):
        end += 1
    git_source = [
        f"{SOURCE_ITEM}type: git\n",
        f"        url: {UPSTREAM}\n",
        # Entre guillemets : un commit tout en chiffres serait lu comme un
        # nombre par YAML.
        f'        tag: "{tag}"\n',
        f'        commit: "{commit}"\n',
    ]
    body = lines[:start] + git_source + lines[end:]
    # En-tête : les commentaires de tête, propres au build local.
    first = next((i for i, line in enumerate(body) if not line.startswith("#")), len(body))
    return HEADER.format(tag=tag) + "".join(body[first:])


def _git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise ManifestError(f"git {' '.join(args)} : {result.stderr.strip()}")
    return result.stdout


def from_tag(repo: Path, tag: str) -> dict[str, str]:
    """Fichiers du dépôt Flathub, lus au commit de `tag`."""
    if not TAG.fullmatch(tag):
        raise ManifestError(f"tag invalide : {tag!r} (attendu : vX.Y.Z)")
    commit = _git(repo, "rev-parse", "--verify", "--quiet", f"refs/tags/{tag}^{{commit}}").strip()

    def show(path: str) -> str:
        return _git(repo, "show", f"{commit}:{path}")

    try:
        version = json.loads(show(TAURI_CONF))["version"]
    except (json.JSONDecodeError, KeyError, TypeError) as e:
        raise ManifestError(f"{TAURI_CONF} au tag {tag} : version illisible ({e})") from None
    if f"v{version}" != tag:
        raise ManifestError(f"tag {tag} ≠ version {version} de {TAURI_CONF}")
    files = {MANIFEST: to_flathub(show(f"{FLATPAK}/{MANIFEST}"), tag, commit)}
    for name in VENDORED:
        files[name] = show(f"{FLATPAK}/{name}")
    return files


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="flathub-manifest.py", exit_on_error=False)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--tag")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--repo", type=Path, default=ROOT)
    try:
        args = parser.parse_args(argv)
    except (argparse.ArgumentError, SystemExit):
        return 2
    if args.check == bool(args.tag or args.out) or bool(args.tag) != bool(args.out):
        print("usage : flathub-manifest.py --tag vX.Y.Z --out DOSSIER [--repo DÉPÔT] | --check",
              file=sys.stderr)
        return 2
    try:
        if args.check:
            manifest = (args.repo / FLATPAK / MANIFEST).read_text(encoding="utf-8")
            to_flathub(manifest, "v0.0.0", "0" * 40)
            return 0
        files = from_tag(args.repo, args.tag)
    except (ManifestError, OSError) as e:
        print(f"flathub-manifest : {e}", file=sys.stderr)
        return 1
    args.out.mkdir(parents=True, exist_ok=True)
    for name, content in files.items():
        (args.out / name).write_text(content, encoding="utf-8")
    print(f"Manifeste Flathub de {args.tag} écrit dans {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
