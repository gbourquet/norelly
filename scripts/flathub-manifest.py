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

Autorisations (`finish-args`) : liste fermée (`ALLOWED_FINISH_ARGS`), une
valeur par ligne, sans guillemets ni échappements, vérifiée sur le manifeste
du dépôt (`--check`) comme sur celui du tag.

Usage :
    flathub-manifest.py --tag vX.Y.Z --out DOSSIER [--repo DÉPÔT]
    flathub-manifest.py --check   (manifeste du dépôt : autorisations et
                                   transformation ; exécuté par
                                   scripts/check-packaging.sh)

Le tag doit être `v` suivi de la version de `app/src-tauri/tauri.conf.json`
au même commit (règle de `release.yml`), et cette version avoir sa note
dans le fichier AppStream. Bibliothèque standard seulement.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FLATPAK = "packaging/flatpak"
MANIFEST = "io.github.gbourquet.Norelly.yml"
VENDORED = ("cargo-sources.json", "node-sources.json")
TAURI_CONF = "app/src-tauri/tauri.conf.json"
METAINFO = "packaging/linux/io.github.gbourquet.Norelly.metainfo.xml"
UPSTREAM = "https://github.com/gbourquet/norelly.git"

TAG = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?")
COMMIT = re.compile(r"[0-9a-f]{40}")
# Source locale : l'élément `- type: dir` de la liste `sources` du module
# et ses lignes plus indentées, jusqu'à l'élément suivant.
SOURCE_ITEM = "      - "
DIR_SOURCE = SOURCE_ITEM + "type: dir\n"
# Toute source `dir`, quelle que soit sa forme, et les clés propres à une
# source locale.
ANY_DIR_SOURCE = re.compile(r"^\s*(?:-\s*)?type\s*:\s*['\"]?dir\b", re.MULTILINE)
LOCAL_KEYS = re.compile(r"^\s+(?:-\s+)?(?:path|skip)\s*:", re.MULTILINE)

# Autorisations admises dans le bac à sable, une par élément de
# `finish-args`. Toute autre (accès aux fichiers, bus de session, sortie du
# bac à sable par org.freedesktop.Flatpak…) se décide ici, en revue.
ALLOWED_FINISH_ARGS = frozenset(
    {
        "--socket=wayland",
        "--socket=fallback-x11",
        "--share=ipc",
        "--device=dri",
        "--share=network",
        "--talk-name=org.freedesktop.secrets",
        "--talk-name=org.freedesktop.Notifications",
    }
)
FINISH_ARGS_KEY = re.compile(r"^\s*(?:\?\s*)?['\"]?finish-args\b", re.MULTILINE)
FINISH_ARG_ITEM = re.compile(r"  - (--[a-z-]+=[A-Za-z0-9._-]+)\n")

HEADER = (
    "# Manifeste Flathub de Norelly, produit par scripts/flathub-manifest.py\n"
    "# depuis packaging/flatpak/io.github.gbourquet.Norelly.yml au tag {tag} :\n"
    "# ne pas le modifier à la main, le régénérer.\n"
)


class ManifestError(Exception):
    pass


def check_finish_args(manifest: str) -> None:
    """Refuse toute autorisation hors de `ALLOWED_FINISH_ARGS`, et toute
    écriture qu'une lecture ligne à ligne ne suit pas avec certitude
    (échappement, liste en flux, clé répétée ou entre guillemets, élément
    sur plusieurs lignes)."""
    if "\\" in manifest:
        raise ManifestError("échappement `\\` dans le manifeste (refusé)")
    if len(FINISH_ARGS_KEY.findall(manifest)) != 1:
        raise ManifestError("une seule clé `finish-args` attendue")
    lines = manifest.splitlines(keepends=True)
    try:
        start = lines.index("finish-args:\n")
    except ValueError:
        raise ManifestError("`finish-args:` attendu seul sur sa ligne, en colonne 0") from None
    seen: set[str] = set()
    for line in lines[start + 1 :]:
        stripped = line.strip()
        # Ligne vide ou commentaire, même en colonne 0 : la liste continue.
        if not stripped or stripped.startswith("#"):
            continue
        if not line.startswith(" "):
            break
        item = FINISH_ARG_ITEM.fullmatch(line)
        if not item:
            raise ManifestError(f"élément de finish-args illisible : {line.rstrip()!r}")
        value = item.group(1)
        if value not in ALLOWED_FINISH_ARGS:
            raise ManifestError(f"autorisation hors de la liste : {value}")
        if value in seen:
            raise ManifestError(f"autorisation répétée : {value}")
        seen.add(value)


def to_flathub(manifest: str, tag: str, commit: str) -> str:
    """Manifeste Flathub : en-tête remplacé, source locale remplacée par le
    tag et son commit, tout le reste à l'identique."""
    if not TAG.fullmatch(tag):
        raise ManifestError(f"tag invalide : {tag!r} (attendu : vX.Y.Z)")
    if not COMMIT.fullmatch(commit):
        raise ManifestError(f"commit invalide : {commit!r} (40 chiffres hexadécimaux)")
    check_finish_args(manifest)
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
    out = HEADER.format(tag=tag) + "".join(body[first:])
    # Source locale coupée (ligne vide, commentaire) ou écrite autrement :
    # il en resterait une trace.
    if ANY_DIR_SOURCE.search(out) or LOCAL_KEYS.search(out):
        raise ManifestError("source locale incomplètement remplacée (bloc `type: dir` à revoir)")
    return out


def _git(repo: Path, *args: str) -> str:
    # Le dépôt est celui de `repo`, jamais un autre désigné par
    # l'environnement (GIT_DIR, GIT_WORK_TREE…).
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    result = subprocess.run(
        ["git", "-C", str(repo), "-c", "core.fsmonitor=", *args],
        capture_output=True,
        encoding="utf-8",
        check=False,
        env=env,
    )
    if result.returncode != 0:
        raise ManifestError(f"git {' '.join(args)} : {result.stderr.strip()}")
    return result.stdout


def from_tag(repo: Path, tag: str) -> dict[str, str]:
    """Fichiers du dépôt Flathub, lus au commit de `tag`."""
    if not TAG.fullmatch(tag):
        raise ManifestError(f"tag invalide : {tag!r} (attendu : vX.Y.Z)")
    commit = _git(repo, "rev-parse", "--verify", "--quiet", f"refs/tags/{tag}^{{commit}}").strip()
    if not COMMIT.fullmatch(commit):
        raise ManifestError(f"commit du tag {tag} inattendu : {commit!r}")

    def show(path: str) -> str:
        return _git(repo, "show", "--no-textconv", f"{commit}:{path}")

    try:
        version = json.loads(show(TAURI_CONF))["version"]
    except (json.JSONDecodeError, KeyError, TypeError) as e:
        raise ManifestError(f"{TAURI_CONF} au tag {tag} : version illisible ({e})") from None
    if f"v{version}" != tag:
        raise ManifestError(f"tag {tag} ≠ version {version} de {TAURI_CONF}")
    try:
        releases = ET.fromstring(show(METAINFO)).findall("releases/release")
    except ET.ParseError as e:
        raise ManifestError(f"{METAINFO} au tag {tag} : illisible ({e})") from None
    if not any(r.get("version") == version for r in releases):
        raise ManifestError(f"{METAINFO} au tag {tag} : aucune note pour la version {version}")
    files = {MANIFEST: to_flathub(show(f"{FLATPAK}/{MANIFEST}"), tag, commit)}
    for name in VENDORED:
        files[name] = show(f"{FLATPAK}/{name}")
    return files


def write_files(out: Path, files: dict[str, str]) -> None:
    """Écrit `files` dans `out`, chacun par un temporaire renommé ; refuse
    d'abord une cible qui n'est pas un fichier ordinaire (lien compris)."""
    for name in files:
        target = out / name
        if target.is_symlink() or (target.exists() and not target.is_file()):
            raise ManifestError(f"{target} : pas un fichier ordinaire (refusé)")
    out.mkdir(parents=True, exist_ok=True)
    for name, content in files.items():
        fd, temp = tempfile.mkstemp(dir=out, prefix=f".{name}.", suffix=".tmp")
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                f.write(content)
            os.chmod(temp, 0o644)
            os.replace(temp, out / name)
        except BaseException:
            Path(temp).unlink(missing_ok=True)
            raise


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="flathub-manifest.py", exit_on_error=False)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--tag")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--repo", type=Path, default=ROOT)
    try:
        args = parser.parse_args(argv)
    except argparse.ArgumentError as e:
        print(f"flathub-manifest : {e}", file=sys.stderr)
        return 2
    except SystemExit as e:
        # --help (0) ou argument inconnu (2), déjà signalés par argparse.
        return e.code if isinstance(e.code, int) else 2
    if args.check == bool(args.tag or args.out) or bool(args.tag) != bool(args.out):
        parser.print_usage(sys.stderr)
        return 2
    try:
        if args.check:
            manifest = (args.repo / FLATPAK / MANIFEST).read_text(encoding="utf-8")
            to_flathub(manifest, "v0.0.0", "0" * 40)
            return 0
        write_files(args.out, from_tag(args.repo, args.tag))
    except (ManifestError, OSError, UnicodeDecodeError) as e:
        print(f"flathub-manifest : {e}", file=sys.stderr)
        return 1
    print(f"Manifeste Flathub de {args.tag} écrit dans {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
