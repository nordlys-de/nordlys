"""Shared helpers for the tests of the shell, Python and config files.

Safety: these scripts call sudo, pacman, udevadm, systemctl and QEMU. A test
never runs one in the real tree or with the real PATH: it copies what the
script needs into a throwaway tree, and PATH holds only fake programs plus
links to the few harmless tools each script is allowed to use.
"""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def read(rel):
    return (REPO / rel).read_text()


class Sandbox(unittest.TestCase):
    """A test with a temporary directory and a PATH of its own."""

    REAL = ("sh",)

    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="nls-")
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)
        self.bin = self.tmp / "bin"
        self.home = self.tmp / "home"
        self.log = self.tmp / "calls.log"
        for d in (self.bin, self.home, self.tmp / "run"):
            d.mkdir()
        self.real(*self.REAL)

    def real(self, *names):
        for name in names:
            found = shutil.which(name, path="/usr/bin:/bin")
            if found is None:
                self.skipTest(f"{name} isn't installed")
            (self.bin / name).symlink_to(found)

    def fake(self, name, body=""):
        """A program that logs "name args…", then runs `body` (sh)."""
        path = self.bin / name
        if path.is_symlink() or path.exists():
            path.unlink()
        path.write_text(f"#!/bin/sh\necho \"{name} $*\" >> '{self.log}'\n{body}\n")
        path.chmod(0o755)
        return path

    def calls(self):
        return self.log.read_text().splitlines() if self.log.exists() else []

    def write(self, rel, body, mode=None):
        path = self.tmp / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)
        if mode is not None:
            path.chmod(mode)
        return path

    def tree(self, *parts, name="repo"):
        """Copy parts of the repository into a throwaway tree."""
        root = self.tmp / name
        for part in parts:
            source, target = REPO / part, root / part
            target.parent.mkdir(parents=True, exist_ok=True)
            if source.is_dir():
                shutil.copytree(source, target)
            else:
                shutil.copy2(source, target)
        return root

    def env(self, **extra):
        env = {
            "PATH": str(self.bin),
            "HOME": str(self.home),
            "XDG_RUNTIME_DIR": str(self.tmp / "run"),
            "LANG": "C.UTF-8",
        }
        env.update({k: str(v) for k, v in extra.items()})
        return env

    def run_script(self, argv, env=None, cwd=None):
        return subprocess.run(
            [str(a) for a in argv],
            env=self.env() if env is None else env,
            cwd=cwd or self.tmp,
            capture_output=True,
            text=True,
            timeout=60,
        )

    def assert_ran(self, result, status=0):
        self.assertEqual(
            result.returncode,
            status,
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )
