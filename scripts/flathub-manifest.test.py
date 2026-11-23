#!/usr/bin/env python3
"""Tests de scripts/flathub-manifest.py, lancés par scripts/check-packaging.sh
(`python3 -I scripts/flathub-manifest.test.py`), sur des dépôts git
temporaires. Bibliothèque standard seulement."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
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


def metainfo(*versions: str) -> str:
    releases = "".join(f'<release version="{v}" date="2026-11-01"/>' for v in versions)
    return f'<?xml version="1.0"?><component><releases>{releases}</releases></component>\n'


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

    def test_refuse_une_source_locale_coupee_par_une_ligne_vide(self) -> None:
        for inserted in ("\n", "    # commentaire\n", "      # commentaire\n"):
            broken = self.manifest.replace(
                "      - type: dir\n", "      - type: dir\n" + inserted, 1
            )
            with self.subTest(inserted=inserted), self.assertRaises(fm.ManifestError):
                fm.to_flathub(broken, "v1.2.3", COMMIT)

    def test_refuse_une_autre_source_locale_ecrite_autrement(self) -> None:
        other = self.manifest.replace(
            "      - cargo-sources.json\n", "      -  type: dir\n        path: .\n      - cargo-sources.json\n"
        )
        with self.assertRaises(fm.ManifestError):
            fm.to_flathub(other, "v1.2.3", COMMIT)

    def test_applique_la_liste_des_autorisations(self) -> None:
        wider = self.manifest.replace(
            "  - --share=network\n", "  - --share=network\n  - --filesystem=home\n"
        )
        with self.assertRaises(fm.ManifestError):
            fm.to_flathub(wider, "v1.2.3", COMMIT)


class FinishArgs(unittest.TestCase):
    """Autorisations du Flatpak : liste fermée, écrite sous une seule forme."""

    def setUp(self) -> None:
        self.manifest = MANIFEST.read_text(encoding="utf-8")

    def with_args(self, extra: str) -> str:
        return self.manifest.replace("  - --share=network\n", "  - --share=network\n" + extra, 1)

    def test_le_manifeste_du_depot_est_accepte(self) -> None:
        fm.check_finish_args(self.manifest)

    def test_un_commentaire_est_accepte(self) -> None:
        fm.check_finish_args(self.with_args("  # - --filesystem=home\n\n"))

    def test_refuse_les_acces_hors_de_la_liste(self) -> None:
        for extra in (
            "  - --filesystem=xdg-download\n",
            "  - --filesystem=home\n",
            # Deux arguments : flatpak build-finish lit `--filesystem home`.
            "  - --filesystem\n  - home\n",
            "  - --talk-name=org.freedesktop.Flatpak\n",
            "  - --socket=session-bus\n",
            "  - --device=all\n",
            '  - "--file\\x73ystem=host"\n',
            "  - '--filesystem=host'\n",
            "  - --share=network # x\n",
            "  - --share=network\n",
            "    - --filesystem=host\n",
            "  -\n    --filesystem=host\n",
            # Un commentaire en colonne 0 ne termine pas la liste YAML.
            "# commentaire\n  - --filesystem=host\n",
        ):
            with self.subTest(extra=extra), self.assertRaises(fm.ManifestError):
                fm.check_finish_args(self.with_args(extra))

    def test_refuse_une_liste_en_flux_ou_une_cle_repetee(self) -> None:
        flow = self.manifest.replace("finish-args:\n", "finish-args: [--filesystem=host]\nx:\n", 1)
        repeated = self.manifest + "finish-args:\n  - --filesystem=host\n"
        quoted = self.manifest + '"finish-args":\n  - --filesystem=host\n'
        missing = self.manifest.replace("finish-args:\n", "autre:\n", 1)
        for manifest in (flow, repeated, quoted, missing):
            with self.subTest(manifest=manifest[-60:]), self.assertRaises(fm.ManifestError):
                fm.check_finish_args(manifest)

    def test_refuse_une_liste_non_indentee(self) -> None:
        # YAML lit `- …` en colonne 0 comme les éléments de finish-args.
        flat = self.manifest.replace("finish-args:\n", "finish-args:\n- --filesystem=home\n", 1)
        with self.assertRaises(fm.ManifestError):
            fm.check_finish_args(flat)

    def test_refuse_une_liste_vide(self) -> None:
        lines = self.manifest.splitlines(keepends=True)
        start = lines.index("finish-args:\n")
        end = lines.index("build-options:\n")
        empty = "".join(lines[: start + 1] + ["\n"] + lines[end:])
        with self.assertRaises(fm.ManifestError):
            fm.check_finish_args(empty)

    def test_refuse_une_cle_de_premier_niveau_deguisee(self) -> None:
        # flatpak-builder garde la dernière valeur d'une clé répétée.
        for extra in (
            "!!str finish-args:\n  - --filesystem=home\n",
            "!x finish-args:\n  - --filesystem=home\n",
            "&k finish-args:\n  - --filesystem=home\n",
            "x-k: &k finish-args\n*k :\n  - --filesystem=home\n",
            "? !!str finish-args\n:\n  - --filesystem=home\n",
            "---\nfinish-args:\n  - --filesystem=home\n",
            "\ufefffinish-args:\n  - --filesystem=home\n",
            "command: autre\n",
        ):
            with self.subTest(extra=extra), self.assertRaises(fm.ManifestError):
                fm.check_finish_args(self.manifest + extra)

    def test_refuse_une_chaine_sur_plusieurs_lignes_ou_entre_apostrophes(self) -> None:
        for replacement in ('command: "norelly\n  x"\n', "command: 'norelly'\n"):
            manifest = self.manifest.replace("command: norelly\n", replacement, 1)
            with self.subTest(replacement=replacement), self.assertRaises(fm.ManifestError):
                fm.check_finish_args(manifest)

    @unittest.skipUnless(shutil.which("flatpak-builder"), "flatpak-builder absent")
    def test_flatpak_builder_lit_les_memes_autorisations(self) -> None:
        shown = subprocess.run(
            ["flatpak-builder", "--show-manifest", str(MANIFEST)],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        self.assertLessEqual(set(json.loads(shown)["finish-args"]), fm.ALLOWED_FINISH_ARGS)

    def test_refuse_un_echappement_n_importe_ou(self) -> None:
        escaped = self.manifest.replace("command: norelly\n", 'command: "norel\\x6cy"\n', 1)
        with self.assertRaises(fm.ManifestError):
            fm.check_finish_args(escaped)


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
        self.metainfo = self.repo / "packaging" / "linux" / "io.github.gbourquet.Norelly.metainfo.xml"
        self.metainfo.parent.mkdir(parents=True)
        self.metainfo.write_text(metainfo("1.2.3"), encoding="utf-8")
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


    def test_refuse_un_tag_sans_note_de_version_appstream(self) -> None:
        self.metainfo.write_text(metainfo("1.2.2", "1.2.30"), encoding="utf-8")
        git(self.repo, "commit", "-q", "-am", "notes")
        (self.repo / "app" / "src-tauri" / "tauri.conf.json").write_text(
            json.dumps({"version": "1.2.4"}), encoding="utf-8"
        )
        git(self.repo, "commit", "-q", "-am", "1.2.4")
        git(self.repo, "tag", "v1.2.4")
        self.assertNotEqual(self.run_main("--tag", "v1.2.4", "--out", str(self.out)), 0)
        self.assertFalse(self.out.exists())

    def tag_preversion(self, note: str) -> int:
        self.metainfo.write_text(metainfo(note, "1.2.3"), encoding="utf-8")
        (self.repo / "app" / "src-tauri" / "tauri.conf.json").write_text(
            json.dumps({"version": "1.2.4-beta.1"}), encoding="utf-8"
        )
        git(self.repo, "commit", "-q", "-am", "1.2.4-beta.1")
        git(self.repo, "tag", "v1.2.4-beta.1")
        return self.run_main("--tag", "v1.2.4-beta.1", "--out", str(self.out))

    def test_preversion_notee_avec_un_tilde_dans_appstream(self) -> None:
        # AppStream classe 1.2.4-beta.1 après 1.2.4, mais 1.2.4~beta.1 avant.
        self.assertEqual(self.tag_preversion("1.2.4~beta.1"), 0)

    def test_refuse_une_preversion_notee_avec_un_tiret_dans_appstream(self) -> None:
        self.assertNotEqual(self.tag_preversion("1.2.4-beta.1"), 0)
        self.assertFalse(self.out.exists())

    def test_ne_suit_pas_un_lien_dans_le_dossier_de_sortie(self) -> None:
        self.out.mkdir()
        elsewhere = self.tmp / "ailleurs.json"
        elsewhere.write_text("intact\n", encoding="utf-8")
        (self.out / "cargo-sources.json").symlink_to(elsewhere)
        self.assertNotEqual(self.run_main("--tag", "v1.2.3", "--out", str(self.out)), 0)
        self.assertEqual(elsewhere.read_text(encoding="utf-8"), "intact\n")
        self.assertFalse((self.out / MANIFEST.name).exists())

    def test_ignore_git_dir_de_l_environnement(self) -> None:
        other = self.tmp / "autre"
        other.mkdir()
        git(other, "init", "-q")
        old = os.environ.get("GIT_DIR")
        os.environ["GIT_DIR"] = str(other / ".git")
        try:
            self.assertEqual(self.run_main("--tag", "v1.2.3", "--out", str(self.out)), 0)
        finally:
            if old is None:
                del os.environ["GIT_DIR"]
            else:
                os.environ["GIT_DIR"] = old


class Check(unittest.TestCase):
    def test_le_manifeste_du_depot_se_transforme(self) -> None:
        self.assertEqual(fm.main(["--check"]), 0)

    def test_arguments_invalides(self) -> None:
        for args in ([], ["--tag", "v1.2.3"], ["--out", "x"], ["--check", "--tag", "v1.2.3"]):
            with self.subTest(args=args), contextlib.redirect_stderr(io.StringIO()):
                self.assertNotEqual(fm.main(args), 0)

    def test_aide(self) -> None:
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(fm.main(["--help"]), 0)


if __name__ == "__main__":
    unittest.main()
