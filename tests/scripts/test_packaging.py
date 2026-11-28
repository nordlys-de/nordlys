"""packaging/arch/PKGBUILD and scripts/subset-icons.sh.

The PKGBUILD's functions run in bash with fake cargo and git; package() runs
the real installer into a throwaway $pkgdir.
"""

import json
import re
import shutil
import unittest

from support import REPO, Sandbox, read


# Constants in icons.rs that don't share their Lucide icon's name.
ALIASES = {
    "sliders": "sliders-horizontal",
    "volume-low": "volume-1",
    "volume-high": "volume-2",
    "volume-muted": "volume-x",
    "close": "x",
    "two-up": "columns-2",
    "grid": "layout-grid",
    "window": "app-window",
    "refresh": "refresh-cw",
    "shelf": "panel-left",
    "swap": "arrow-left-right",
    "restart": "rotate-ccw",
    "caps-lock": "arrow-big-up",
}


def workspace_members():
    return {
        re.search(r'^name = "(.+)"', toml.read_text(), re.M).group(1)
        for toml in (REPO / "crates").glob("*/Cargo.toml")
    }


class Pkgbuild(Sandbox):
    REAL = ("sh", "bash", "sed", "head", "install", "chmod", "rm", "dirname", "basename")

    def setUp(self):
        super().setUp()
        self.src = self.tree(
            "packaging/arch/PKGBUILD", "scripts/install.sh", "session", "config", "vm",
            "crates/nordlys-settings/nordlys-settings.desktop", "Cargo.toml",
            name="src/nordlys",
        )
        self.pkgbuild = self.src / "packaging/arch/PKGBUILD"
        self.pkgdir = self.tmp / "pkg"
        self.fake("cargo", 'echo "toolchain=$RUSTUP_TOOLCHAIN dir=${CARGO_TARGET_DIR-unset}" >> "$NORDLYS_TEST_LOG"')
        self.fake("rustc", "echo 'host: x86_64-unknown-linux-gnu'")
        self.fake("git", 'case $* in "rev-list --count HEAD") echo 42;; *) echo abc1234;; esac')

    def bash(self, script):
        return self.run_script(
            ["bash", "-ec", f'source "{self.pkgbuild}"; {script}'],
            env=self.env(srcdir=self.tmp / "src", pkgdir=self.pkgdir, NORDLYS_TEST_LOG=self.log),
            cwd=self.tmp / "src",
        )

    def array(self, name):
        result = self.bash(f'printf "%s\\n" "${{{name}[@]}}"')
        self.assert_ran(result)
        return result.stdout.splitlines()

    def test_it_names_the_package_it_replaces(self):
        self.assertEqual(self.array("provides"), ["nordlys"])
        self.assertEqual(self.array("conflicts"), ["nordlys"])
        self.assertEqual(self.array("backup"), ["etc/pam.d/nordlys"])
        self.assertEqual(len(self.array("source")), len(self.array("sha256sums")))

    def test_it_depends_on_what_the_session_starts(self):
        depends = self.array("depends")
        for needed in ("hyprland", "hypridle", "swaybg", "pam", "wireplumber"):
            self.assertIn(needed, depends)
        self.assertIn("cargo", self.array("makedepends"))

    def test_the_version_comes_from_cargo_and_git(self):
        result = self.bash("pkgver")
        self.assert_ran(result)
        version = re.search(r'^version = "(.+)"', read("Cargo.toml"), re.M).group(1)
        self.assertEqual(result.stdout, f"{version}.r42.gabc1234")

    def test_prepare_fetches_for_this_machine(self):
        self.assert_ran(self.bash("prepare"))
        self.assertIn("cargo fetch --locked --target x86_64-unknown-linux-gnu", self.calls())
        self.assertIn("toolchain=stable dir=unset", self.calls())

    def test_build_and_check_name_real_crates(self):
        for step, first in (("build", "cargo build --frozen --release"),
                            ("check", "cargo test --frozen --release")):
            with self.subTest(step=step):
                self.log.unlink(missing_ok=True)
                self.assert_ran(self.bash(step))
                call = self.calls()[0]
                self.assertTrue(call.startswith(first), call)
                named = re.findall(r"-p (\S+)", call)
                self.assertTrue(named)
                self.assertEqual(set(named) - workspace_members(), set())

    def test_it_builds_and_tests_everything_it_installs(self):
        bins = set(re.search(r'^bins="(.+)"', read("scripts/install.sh"), re.M).group(1).split())
        built = {}
        for step in ("build", "check"):
            self.log.unlink(missing_ok=True)
            self.assert_ran(self.bash(step))
            built[step] = set(re.findall(r"-p (\S+)", self.calls()[0]))
        self.assertEqual(built["build"], bins)
        self.assertLessEqual(built["build"], built["check"])
        self.assertLessEqual(built["check"], workspace_members())
        setup = re.search(r"cargo build --release --locked (.+?)\)", read("scripts/setup.sh"))
        self.assertEqual(set(re.findall(r"-p (\S+)", setup.group(1))), bins)

    def test_package_runs_the_installer_into_pkgdir(self):
        for name in ("nordlys-comp", "nordlys-shell", "nordlysctl", "nordlys-theme",
                     "nordlys-settings"):
            self.write(f"src/nordlys/target/release/{name}", "#!/bin/sh\n", 0o755)
        self.assert_ran(self.bash("package"))
        for rel in (
            "usr/bin/nordlys-shell",
            "usr/bin/nordlys-settings",
            "usr/bin/nordlys-session",
            "usr/share/nordlys/hyprland.lua",
            "usr/lib/systemd/user/nordlys-session.target",
            "usr/share/wayland-sessions/nordlys.desktop",
            "usr/share/applications/nordlys-settings.desktop",
            "etc/pam.d/nordlys",
        ):
            self.assertTrue((self.pkgdir / rel).is_file(), rel)


def icon_names():
    body = re.search(r'ICONS="(.*?)"', read("scripts/subset-icons.sh"), re.S).group(1)
    return body.split()


def icon_constants():
    """{name: codepoint} from icons.rs."""
    source = read("crates/nordlys-shell/src/icons.rs")
    return {
        name: int(point, 16)
        for name, point in re.findall(
            r"const (\w+): char = '\\u\{([0-9a-fA-F]+)\}';", source
        )
    }


def cmap(path):
    """Every codepoint a TrueType font has a glyph for."""
    data = path.read_bytes()
    number = lambda at, size: int.from_bytes(data[at:at + size], "big")
    tables = {
        data[12 + 16 * i:16 + 16 * i]: number(20 + 16 * i, 4)
        for i in range(number(4, 2))
    }
    start = tables[b"cmap"]
    points = set()
    for i in range(number(start + 2, 2)):
        table = start + number(start + 8 + 8 * i, 4)
        kind = number(table, 2)
        if kind == 12:
            for g in range(number(table + 12, 4)):
                at = table + 16 + 12 * g
                points.update(range(number(at, 4), number(at + 4, 4) + 1))
        elif kind == 4:
            segments = number(table + 6, 2) // 2
            for s in range(segments):
                end = number(table + 14 + 2 * s, 2)
                first = number(table + 16 + 2 * segments + 2 * s, 2)
                if first != 0xFFFF:
                    points.update(range(first, end + 1))
    return points


class IconFont(unittest.TestCase):
    def test_every_icon_the_shell_draws_is_in_the_font(self):
        constants = icon_constants()
        self.assertGreater(len(constants), 30)
        font = cmap(REPO / "crates/nordlys-shell/assets/lucide-subset.ttf")
        missing = {n: hex(p) for n, p in constants.items() if p not in font}
        self.assertEqual(missing, {})

    def test_the_subset_list_matches_the_constants(self):
        names = icon_names()
        self.assertEqual(len(names), len(set(names)), "no duplicates")
        wanted = {
            ALIASES.get(c, c)
            for c in (name.lower().replace("_", "-") for name in icon_constants())
        }
        self.assertEqual(set(names), wanted, "keep ICONS in step with icons.rs")


class SubsetScript(Sandbox):
    REAL = ("sh", "dirname", "wc", "cat")

    def setUp(self):
        super().setUp()
        self.repo = self.tree("scripts/subset-icons.sh")
        (self.repo / "crates/nordlys-shell/assets").mkdir(parents=True)
        self.script = self.repo / "scripts/subset-icons.sh"
        # Stands in for python3 + fontTools: keeps the program it was given.
        self.fake("python3", f'cat > "{self.tmp}/program.py"; printf font > "$4"')

    def test_it_asks_for_the_font_folder(self):
        result = self.run_script(["sh", self.script])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("usage: subset-icons.sh path/to/lucide-font", result.stderr)
        self.assertEqual(self.calls(), [])

    def test_it_hands_python_the_paths_and_every_icon(self):
        result = self.run_script(["sh", self.script, "/some/lucide-font"])
        self.assert_ran(result)
        words = self.calls()[0].split()
        self.assertEqual(words[:4], ["python3", "-I", "-", "/some/lucide-font"])
        self.assertTrue(words[4].endswith("assets/lucide-subset.ttf"))
        self.assertEqual(words[5:], icon_names())
        self.assertRegex(result.stdout, r"wrote .*lucide-subset\.ttf \( *4 bytes\)")
        program = (self.tmp / "program.py").read_text()
        compile(program, "subset-icons heredoc", "exec")


if __name__ == "__main__":
    unittest.main()
