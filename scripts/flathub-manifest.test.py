#!/usr/bin/env python3
"""Tests de scripts/flathub-manifest.py, lancés par scripts/check-packaging.sh
(`python3 -I scripts/flathub-manifest.test.py`), sur des dépôts git
temporaires. Bibliothèque standard seulement."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "packaging" / "flatpak" / "io.github.gbourquet.Norelly.yml"

# Aucun __pycache__ dans scripts/.
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location(
    "flathub_manifest", Path(__file__).resolve().parent / "flathub-manifest.py"
)
assert _spec is not None and _spec.loader is not None
fm = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fm)

COMMIT = "0123456789abcdef0123456789abcdef01234567"
GIT_SOURCE = (
    "      - type: git\n"
    "        url: https://github.com/gbourquet/norelly.git\n"
    '        tag: "v1.2.3"\n'
    f'        commit: "{COMMIT}"\n'
)


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args],
        check=True,
        capture_output=True,
        text=True,
        env={
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@example.invalid",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@example.invalid",
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
            "PATH": "/usr/bin:/bin:/usr/local/bin",
        },
    ).stdout.strip()


class Transform(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest = MANIFEST.read_text(encoding="utf-8")

    def test_remplace_la_source_locale_par_le_tag_et_le_commit(self) -> None:
        out = fm.to_flathub(self.manifest, "v1.2.3", COMMIT)
        self.assertIn(GIT_SOURCE, out)
        self.assertNotIn("type: dir", out)
        self.assertNotIn("path: ../..", out)
        self.assertNotIn("- .flatpak-builder", out)

    def test_garde_les_autres_sources_et_les_autorisations(self) -> None:
        out = fm.to_flathub(self.manifest, "v1.2.3", COMMIT)
        for kept in (
            "      - type: archive\n",
            "        dest: pnpm\n",
            "      - cargo-sources.json\n",
            "      - node-sources.json\n",
            "finish-args:\n",
            "build-options:\n",
            "      - sh packaging/flatpak/build.sh\n",
        ):
            self.assertIn(kept, out)
        # Entre l'en-tête et la source locale, rien ne change.
        body = self.manifest[self.manifest.index("\nid:") : self.manifest.index("      - type: dir\n")]
        self.assertIn(body, out)
        # Tout ce qui suit la source locale est repris à l'identique.
        tail = self.manifest[self.manifest.index("      - type: archive\n") :]
        self.assertTrue(out.endswith(tail))

    def test_signale_le_manifeste_produit(self) -> None:
        out = fm.to_flathub(self.manifest, "v1.2.3", COMMIT)
        self.assertTrue(out.startswith("# "))
        self.assertIn("scripts/flathub-manifest.py", out.split("\nid:", 1)[0])
        self.assertNotIn("flatpak-builder --force-clean build packaging", out)
        self.assertIn("\nid: io.github.gbourquet.Norelly\n", out)

    def test_refuse_un_manifeste_sans_source_locale(self) -> None:
        without = self.manifest.replace("      - type: dir\n", "      - type: file\n")
        with self.assertRaises(fm.ManifestError):
            fm.to_flathub(without, "v1.2.3", COMMIT)

    def test_refuse_deux_sources_locales(self) -> None:
        block = self.manifest[
            self.manifest.index("      - type: dir\n") : self.manifest.index("      - type: archive\n")
        ]
        doubled = self.manifest.replace(block, block + block)
        with self.assertRaises(fm.ManifestError):
            fm.to_flathub(doubled, "v1.2.3", COMMIT)

    def test_refuse_un_tag_ou_un_commit_mal_formes(self) -> None:
        for tag, commit in (
            ("1.2.3", COMMIT),
            ("v1.2", COMMIT),
            ("v1.2.3\n", COMMIT),
            ("v1.2.3 # x", COMMIT),
            ("v1.2.3", COMMIT[:12]),
            ("v1.2.3", COMMIT.upper()),
            ("v1.2.3", COMMIT + "\n"),
        ):
            with self.subTest(tag=tag, commit=commit), self.assertRaises(fm.ManifestError):
                fm.to_flathub(self.manifest, tag, commit)

    def test_accepte_une_preversion(self) -> None:
        out = fm.to_flathub(self.manifest, "v1.2.3-rc.1", COMMIT)
        self.assertIn('        tag: "v1.2.3-rc.1"\n', out)

    def test_commit_entre_guillemets_meme_tout_en_chiffres(self) -> None:
        # Sans guillemets, YAML lirait ce commit comme un nombre.
        digits = "1234567890" * 4
        out = fm.to_flathub(self.manifest, "v1.2.3", digits)
        self.assertIn(f'        commit: "{digits}"\n', out)


class FromTag(unittest.TestCase):
    """Dépôt temporaire : manifeste, sources vendorisées et version au tag."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="flathub-manifest-"))
        self.addCleanup(shutil.rmtree, self.tmp)
        self.repo = self.tmp / "repo"
        flatpak = self.repo / "packaging" / "flatpak"
        flatpak.mkdir(parents=True)
        shutil.copy(MANIFEST, flatpak / MANIFEST.name)
        (flatpak / "cargo-sources.json").write_text('[{"tag": "cargo"}]\n', encoding="utf-8")
        (flatpak / "node-sources.json").write_text('[{"tag": "node"}]\n', encoding="utf-8")
        conf = self.repo / "app" / "src-tauri"
        conf.mkdir(parents=True)
        (conf / "tauri.conf.json").write_text(json.dumps({"version": "1.2.3"}), encoding="utf-8")
        git(self.repo, "init", "-q")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-q", "-m", "version")
        git(self.repo, "tag", "-a", "v1.2.3", "-m", "v1.2.3")
        self.commit = git(self.repo, "rev-parse", "HEAD")
        # Après le tag : l'arbre de travail et HEAD changent, le tag reste.
        (flatpak / "cargo-sources.json").write_text('[{"tag": "après"}]\n', encoding="utf-8")
        git(self.repo, "commit", "-q", "-am", "après")
        (flatpak / "node-sources.json").write_text("non commité\n", encoding="utf-8")
        self.out = self.tmp / "flathub"

    def run_main(self, *args: str) -> int:
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return fm.main(["--repo", str(self.repo), *args])

    def test_ecrit_le_manifeste_et_les_sources_du_tag(self) -> None:
        self.assertEqual(self.run_main("--tag", "v1.2.3", "--out", str(self.out)), 0)
        manifest = (self.out / MANIFEST.name).read_text(encoding="utf-8")
        self.assertIn(f'        tag: "v1.2.3"\n        commit: "{self.commit}"\n', manifest)
        self.assertEqual(
            (self.out / "cargo-sources.json").read_text(encoding="utf-8"), '[{"tag": "cargo"}]\n'
        )
        self.assertEqual(
            (self.out / "node-sources.json").read_text(encoding="utf-8"), '[{"tag": "node"}]\n'
        )

    def test_tag_leger_accepte(self) -> None:
        git(self.repo, "tag", "-d", "v1.2.3")
        git(self.repo, "tag", "v1.2.3", self.commit)
        self.assertEqual(self.run_main("--tag", "v1.2.3", "--out", str(self.out)), 0)
        manifest = (self.out / MANIFEST.name).read_text(encoding="utf-8")
        self.assertIn(f'        commit: "{self.commit}"\n', manifest)

    def test_version_lue_au_tag_pas_dans_l_arbre_de_travail(self) -> None:
        # Version 1.2.4 dans l'arbre de travail, 1.2.3 au commit du tag.
        (self.repo / "app" / "src-tauri" / "tauri.conf.json").write_text(
            json.dumps({"version": "1.2.4"}), encoding="utf-8"
        )
        git(self.repo, "tag", "v1.2.4", self.commit)
        self.assertNotEqual(self.run_main("--tag", "v1.2.4", "--out", str(self.out)), 0)
        self.assertFalse(self.out.exists())

    def test_refuse_un_tag_absent(self) -> None:
        self.assertNotEqual(self.run_main("--tag", "v9.9.9", "--out", str(self.out)), 0)
        self.assertFalse(self.out.exists())

    def test_refuse_un_tag_qui_ne_correspond_pas_a_la_version(self) -> None:
        git(self.repo, "tag", "-a", "v2.0.0", "-m", "v2.0.0")
        self.assertNotEqual(self.run_main("--tag", "v2.0.0", "--out", str(self.out)), 0)
        self.assertFalse(self.out.exists())

    def test_refuse_un_nom_de_tag_qui_serait_une_option(self) -> None:
        self.assertNotEqual(self.run_main("--tag", "--all", "--out", str(self.out)), 0)
        self.assertFalse(self.out.exists())

    def test_remplace_les_fichiers_d_un_depot_flathub_existant(self) -> None:
        self.out.mkdir()
        (self.out / "flathub.json").write_text("{}\n", encoding="utf-8")
        (self.out / "cargo-sources.json").write_text("ancien\n", encoding="utf-8")
        self.assertEqual(self.run_main("--tag", "v1.2.3", "--out", str(self.out)), 0)
        self.assertEqual((self.out / "flathub.json").read_text(encoding="utf-8"), "{}\n")
        self.assertEqual(
            (self.out / "cargo-sources.json").read_text(encoding="utf-8"), '[{"tag": "cargo"}]\n'
        )


class Check(unittest.TestCase):
    def test_le_manifeste_du_depot_se_transforme(self) -> None:
        self.assertEqual(fm.main(["--check"]), 0)

    def test_arguments_invalides(self) -> None:
        for args in ([], ["--tag", "v1.2.3"], ["--out", "x"], ["--check", "--tag", "v1.2.3"]):
            with self.subTest(args=args), contextlib.redirect_stderr(io.StringIO()):
                self.assertNotEqual(fm.main(args), 0)


if __name__ == "__main__":
    unittest.main()
