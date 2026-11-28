"""scripts/setup.sh with fake sudo, pacman, cargo and friends.

The throwaway copy of the script reads and writes /etc inside the test's own
directory. The fake sudo records what it was asked to run; it only carries out
tee, cp and install, and only on paths inside that directory, so nothing on
this machine is installed, enabled or edited.
"""

import unittest

from support import Sandbox


class Setup(Sandbox):
    REAL = ("sh", "sed", "dirname", "basename", "readlink", "grep", "tee", "cp", "install", "ln")

    def setUp(self):
        super().setUp()
        self.repo = self.tree("scripts/setup.sh")
        script = self.repo / "scripts/setup.sh"
        text = script.read_text()
        self.etc = self.tmp / "etc"
        for folder in ("pam.d", "systemd/system"):
            (self.etc / folder).mkdir(parents=True)
        self.assertIn("/etc/pam.d/", text)
        script.write_text(text.replace("/etc/", f"{self.etc}/"))
        self.display_manager = self.etc / "systemd/system/display-manager.service"
        self.fake(
            "sudo",
            f"""case $1 in
                tee|cp|install)
                    for a in "$@"; do
                        case $a in
                            '{self.tmp}'/*) ;;
                            /*) echo "fake sudo: refusing $a" >&2; exit 98 ;;
                        esac
                    done
                    "$@" ;;
                systemctl)
                    if [ "$2 $3" = "enable sddm" ]; then
                        : > '{self.tmp}/sddm.service'
                        ln -s '{self.tmp}/sddm.service' '{self.display_manager}'
                    fi ;;
            esac""",
        )
        self.fake("pacman")
        self.fake("cargo", 'echo "cargo-in $(pwd -P)" >> "$NORDLYS_TEST_LOG"')
        self.fake("id", "echo 1000")
        self.fake("systemd-detect-virt", "echo none; exit 1")

    def setup_nordlys(self, *args):
        return self.run_script(
            ["sh", self.repo / "scripts/setup.sh", *args],
            env=self.env(NORDLYS_TEST_LOG=self.log),
        )

    def sudo_calls(self):
        return [c.removeprefix("sudo ") for c in self.calls() if c.startswith("sudo ")]

    def packages(self):
        first = self.sudo_calls()[0].split()
        self.assertEqual(first[:3], ["pacman", "-Syu", "--needed"])
        return first[3:]

    def test_help(self):
        result = self.setup_nordlys("--help")
        self.assert_ran(result)
        self.assertIn("Set up Nordlys on a fresh Arch", result.stdout)
        self.assertIn("--no-vm-guest", result.stdout)
        self.assertEqual(self.calls(), [])

    @unittest.expectedFailure  # known bug: the sed range runs one line too far
    def test_help_shows_only_the_comment(self):
        self.assertNotIn("set -eu", self.setup_nordlys("--help").stdout)

    def test_an_unknown_option_is_refused(self):
        result = self.setup_nordlys("--fast")
        self.assert_ran(result, 2)
        self.assertIn("unknown option --fast", result.stderr)
        self.assertEqual(self.calls(), [])

    def test_it_refuses_to_run_as_root(self):
        self.fake("id", "echo 0")
        result = self.setup_nordlys()
        self.assert_ran(result, 1)
        self.assertIn("not root", result.stderr)
        self.assertEqual(self.sudo_calls(), [])

    def test_only_arch_for_now(self):
        (self.bin / "pacman").unlink()
        result = self.setup_nordlys()
        self.assert_ran(result, 1)
        self.assertIn("only Arch-based", result.stderr)
        self.assertEqual(self.sudo_calls(), [])

    def test_it_installs_builds_and_runs_the_installer(self):
        result = self.setup_nordlys()
        self.assert_ran(result)
        packages = self.packages()
        for needed in (
            "hyprland", "hypridle", "swaybg", "pam", "wireplumber", "seatd", "libinput",
            "xwayland-satellite", "gnome-keyring", "wf-recorder", "cliphist",
        ):
            self.assertIn(needed, packages)
        self.assertEqual(len(packages), len(set(packages)), "no package listed twice")
        self.assertNotIn("rust", packages, "cargo is already there")
        self.assertNotIn("python", packages)
        self.assertIn(
            "cargo build --release --locked -p nordlys-comp -p nordlys-shell "
            "-p nordlysctl -p nordlys-theme -p nordlys-settings",
            self.calls(),
        )
        self.assertIn(f"cargo-in {self.repo.resolve()}", self.calls())
        self.assertEqual(self.sudo_calls()[1], f"{self.repo.resolve()}/scripts/install.sh")
        self.assertIn("nordlys-shell lock --check", result.stdout)
        self.assertNotIn("shared clipboard", result.stdout)

    def test_the_order_is_dependencies_build_install(self):
        self.assert_ran(self.setup_nordlys())
        steps = [
            next(i for i, c in enumerate(self.calls()) if needle in c)
            for needle in ("pacman -Syu", "cargo build", "install.sh")
        ]
        self.assertEqual(steps, sorted(steps))

    def test_without_cargo_it_adds_rust(self):
        (self.bin / "cargo").unlink()
        # Nothing can build, which stops the script after the install step.
        result = self.setup_nordlys()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("rust", self.packages())
        self.assertFalse(any("install.sh" in c for c in self.calls()))

    def test_a_failed_build_installs_nothing(self):
        self.fake("cargo", "exit 101")
        result = self.setup_nordlys()
        self.assert_ran(result, 101)
        self.assertEqual(len(self.sudo_calls()), 1)

    def test_a_kvm_guest_is_noticed(self):
        for virt in ("kvm", "qemu"):
            with self.subTest(virt=virt):
                self.log.unlink(missing_ok=True)
                self.fake("systemd-detect-virt", f"echo {virt}")
                result = self.setup_nordlys()
                self.assert_ran(result)
                self.assertIn("python", self.packages())
                self.assertTrue(self.sudo_calls()[1].endswith("install.sh --vm-guest"))
                self.assertIn("shared clipboard", result.stdout)

    def test_the_guest_choice_can_be_forced(self):
        self.fake("systemd-detect-virt", "echo kvm")
        self.assert_ran(self.setup_nordlys("--no-vm-guest"))
        self.assertTrue(self.sudo_calls()[1].endswith("install.sh"))
        self.log.unlink()
        self.fake("systemd-detect-virt", "echo none; exit 1")
        self.assert_ran(self.setup_nordlys("--vm-guest"))
        self.assertTrue(self.sudo_calls()[1].endswith("install.sh --vm-guest"))

    # -- the login screen ---------------------------------------------------

    def use_display_manager(self, name):
        unit = self.write(f"units/{name}.service", "")
        self.display_manager.symlink_to(unit)

    def theme_file(self):
        return self.etc / "sddm.conf.d/nordlys.conf"

    def pam(self, name, body="auth include system-login\n"):
        return self.write(f"etc/pam.d/{name}", body)

    def test_sddm_when_there_is_no_display_manager(self):
        result = self.setup_nordlys()
        self.assert_ran(result)
        self.assertIn("installing SDDM", result.stdout)
        self.assertIn("pacman -S --needed sddm", self.sudo_calls())
        self.assertIn("systemctl enable sddm", self.sudo_calls())
        self.assertIn("Using the Nordlys login screen", result.stdout)
        self.assertEqual(self.theme_file().read_text(), "[Theme]\nCurrent=nordlys\n")

    def test_an_sddm_you_already_had_keeps_its_theme(self):
        self.use_display_manager("sddm")
        result = self.setup_nordlys()
        self.assert_ran(result)
        self.assertNotIn("installing SDDM", result.stdout)
        self.assertFalse(self.theme_file().exists())
        self.assertIn("Done.", result.stdout)

    def test_the_theme_can_be_asked_for(self):
        self.use_display_manager("sddm")
        self.assert_ran(self.setup_nordlys("--sddm-theme"))
        self.assertTrue(self.theme_file().is_file())

    def test_another_display_manager_is_left_alone(self):
        self.use_display_manager("gdm")
        gdm = self.pam("gdm")
        result = self.setup_nordlys("--sddm-theme")
        self.assert_ran(result)
        self.assertFalse(self.theme_file().exists())
        self.assertNotIn("keyring at login", result.stdout)
        self.assertNotIn("gnome_keyring", gdm.read_text())

    # -- unlocking the keyring ----------------------------------------------

    AUTH = "-auth       optional    pam_gnome_keyring.so\n"
    SESSION = "-session    optional    pam_gnome_keyring.so auto_start\n"

    def test_the_keyring_lines_are_added_with_a_backup(self):
        for dm in ("sddm", "greetd", "lightdm", "ly"):
            with self.subTest(dm=dm):
                self.display_manager.unlink(missing_ok=True)
                self.use_display_manager(dm)
                original = "auth include system-login\n"
                files = [self.pam(dm, original), self.pam("login", original)]
                for f in files:
                    f.with_name(f.name + ".pre-nordlys").unlink(missing_ok=True)
                self.assert_ran(self.setup_nordlys())
                for f in files:
                    self.assertEqual(f.read_text(), original + self.AUTH + self.SESSION, f.name)
                    backup = f.with_name(f.name + ".pre-nordlys")
                    self.assertEqual(backup.read_text(), original)

    def test_running_setup_again_adds_nothing(self):
        login = self.pam("login")
        self.assert_ran(self.setup_nordlys())
        once = login.read_text()
        result = self.setup_nordlys()
        self.assert_ran(result)
        self.assertEqual(login.read_text(), once)
        self.assertNotIn("keyring at login", result.stdout)
        self.assertNotIn("gnome_keyring", (self.etc / "pam.d/login.pre-nordlys").read_text())

    def test_only_the_missing_line_is_added(self):
        for present, added in (("auth", self.SESSION), ("session", self.AUTH)):
            with self.subTest(present=present):
                mine = f"  {present} optional pam_gnome_keyring.so\n"
                login = self.pam("login", mine)
                self.assert_ran(self.setup_nordlys())
                self.assertEqual(login.read_text(), mine + added)

    def test_a_commented_out_line_does_not_count(self):
        mine = "# auth optional pam_gnome_keyring.so\n"
        login = self.pam("login", mine)
        self.assert_ran(self.setup_nordlys())
        self.assertEqual(login.read_text(), mine + self.AUTH + self.SESSION)

    def test_the_keyring_can_be_skipped(self):
        login = self.pam("login")
        self.assert_ran(self.setup_nordlys("--no-keyring"))
        self.assertEqual(login.read_text(), "auth include system-login\n")
        self.assertFalse((self.etc / "pam.d/login.pre-nordlys").exists())

    def test_without_pam_files_none_are_made(self):
        self.assert_ran(self.setup_nordlys())
        self.assertEqual(list((self.etc / "pam.d").iterdir()), [])


if __name__ == "__main__":
    unittest.main()
