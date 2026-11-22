#!/usr/bin/env python3
"""Tests de scripts/flatpak-sources.py, lancés par scripts/check-packaging.sh
(`python3 -I scripts/flatpak-sources.test.py`). Bibliothèque standard
seulement."""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path

# Aucun __pycache__ dans scripts/.
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location(
    "flatpak_sources", Path(__file__).resolve().parent / "flatpak-sources.py"
)
assert _spec is not None and _spec.loader is not None
fs = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fs)

FORK = "https://github.com/gbourquet/imap-codec"
REV = "9c6a1eccf673f0df749d6ba56eb60353411e9028"
CHECKSUM = "a" * 64


def crate(name: str, version: str = "1.0.0") -> dict:
    return {
        "name": name,
        "version": version,
        "source": fs.CRATES_IO,
        "checksum": CHECKSUM,
    }


def git_crate(name: str, url: str = FORK, rev: str = REV, fragment: str | None = None) -> dict:
    return {
        "name": name,
        "version": "2.0.0",
        "source": f"git+{url}?rev={rev}#{fragment if fragment is not None else rev}",
    }


def lock(*packages: dict) -> dict:
    return {"package": [{"name": "norelly-app", "version": "0.1.0"}, *packages]}


class CratesIo(unittest.TestCase):
    def test_a_registry_crate_is_an_archive_checked_by_its_sum(self) -> None:
        sources = fs.cargo_sources(lock(crate("serde")), [FORK])
        archive = sources[0]
        self.assertEqual(archive["type"], "archive")
        self.assertEqual(archive["sha256"], CHECKSUM)
        self.assertEqual(archive["dest"], "cargo/vendor/serde-1.0.0")

    def test_without_git_source_the_config_only_replaces_crates_io(self) -> None:
        config = fs.cargo_sources(lock(crate("serde")), [FORK])[-1]["contents"]
        self.assertNotIn("git+", config)
        self.assertIn('[source.crates-io]\nreplace-with = "vendored-sources"', config)


class GitSources(unittest.TestCase):
    def sources(self) -> list[dict]:
        return fs.cargo_sources(
            lock(crate("serde"), git_crate("imap-codec"), git_crate("imap-types")), [FORK]
        )

    def test_the_fork_is_cloned_once_at_its_commit(self) -> None:
        clones = [s for s in self.sources() if s["type"] == "git"]
        self.assertEqual(len(clones), 1)
        self.assertEqual(clones[0]["url"], FORK)
        self.assertEqual(clones[0]["commit"], REV)
        self.assertNotIn("branch", clones[0])
        self.assertNotIn("tag", clones[0])

    def test_each_crate_of_the_fork_is_copied_into_the_vendor_directory(self) -> None:
        shells = [s for s in self.sources() if s["type"] == "shell"]
        commands = "\n".join(c for s in shells for c in s["commands"])
        clone = next(s for s in self.sources() if s["type"] == "git")["dest"]
        for name in ("imap-codec", "imap-types"):
            self.assertIn(f'cp -r "{clone}/{name}" "cargo/vendor/{name}"', commands)

    def test_a_manifest_still_inheriting_from_the_workspace_stops_the_build(self) -> None:
        shells = [s for s in self.sources() if s["type"] == "shell"]
        commands = "\n".join(c for s in shells for c in s["commands"])
        self.assertIn("rust-version\\.workspace = true", commands)
        self.assertIn('! grep -q "workspace" "cargo/vendor/imap-codec/Cargo.toml"', commands)

    def test_a_crate_of_the_fork_has_an_empty_checksum(self) -> None:
        inline = [
            s
            for s in self.sources()
            if s["type"] == "inline" and s["dest"] == "cargo/vendor/imap-codec"
        ]
        self.assertEqual(len(inline), 1)
        self.assertEqual(json.loads(inline[0]["contents"]), {"package": None, "files": {}})

    def test_the_config_replaces_the_git_source_by_the_vendor_directory(self) -> None:
        config = self.sources()[-1]["contents"]
        self.assertIn(
            f'[source."git+{FORK}?rev={REV}"]\n'
            f'git = "{FORK}"\n'
            f'rev = "{REV}"\n'
            'replace-with = "vendored-sources"\n',
            config,
        )


class GitRefusals(unittest.TestCase):
    def refused(self, package: dict, allowed: list[str] | None = None) -> str:
        with self.assertRaises(SystemExit) as ctx:
            fs.cargo_sources(lock(package), [FORK] if allowed is None else allowed)
        return str(ctx.exception)

    def test_a_repository_outside_allow_git_is_refused(self) -> None:
        message = self.refused(git_crate("evil", url="https://github.com/evil/imap-codec"))
        self.assertIn("source git non admise", message)

    def test_no_repository_is_admitted_when_allow_git_is_empty(self) -> None:
        self.assertIn("source git non admise", self.refused(git_crate("imap-codec"), []))

    def test_a_branch_instead_of_a_commit_is_refused(self) -> None:
        package = git_crate("imap-codec")
        package["source"] = f"git+{FORK}?branch=main#{REV}"
        self.assertIn("source git non figée par commit", self.refused(package))

    def test_a_short_commit_is_refused(self) -> None:
        package = git_crate("imap-codec", rev="9c6a1ec", fragment=REV)
        self.assertIn("source git non figée par commit", self.refused(package))

    def test_a_resolved_commit_other_than_the_pinned_one_is_refused(self) -> None:
        package = git_crate("imap-codec", fragment="b" * 40)
        self.assertIn("source git non figée par commit", self.refused(package))

    def test_a_crate_name_that_is_not_a_plain_directory_is_refused(self) -> None:
        self.assertIn("nom de crate invalide", self.refused(git_crate("../imap-codec")))

    def test_another_kind_of_source_is_still_refused(self) -> None:
        package = crate("serde")
        package["source"] = "registry+https://example.org/index"
        self.assertIn("source non prise en charge", self.refused(package))


class AllowedGit(unittest.TestCase):
    def test_the_admitted_repositories_come_from_deny_toml(self) -> None:
        self.assertEqual(fs.allowed_git('[sources]\nallow-git = ["%s"]\n' % FORK), [FORK])

    def test_without_allow_git_no_repository_is_admitted(self) -> None:
        self.assertEqual(fs.allowed_git('[sources]\nunknown-git = "deny"\n'), [])


if __name__ == "__main__":
    unittest.main(verbosity=1)
