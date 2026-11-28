"""scripts/install.sh, always into a throwaway --destdir.

Without --destdir the script writes to /etc and /usr and runs udevadm, so
`install` below refuses to run it that way.
"""

import stat
import unittest

from support import Sandbox

PARTS = (
    "scripts/install.sh", "session", "config", "vm",
    "crates/nordlys-settings/nordlys-settings.desktop",
)
BINS = ("nordlys-comp", "nordlys-shell", "nordlysctl", "nordlys-theme", "nordlys-settings")
SCRIPTS = ("nordlys-session", "nordlys-record")


class Install(Sandbox):
    REAL = ("sh", "sed", "install", "chmod", "rm", "dirname", "basename", "cat", "awk", "tr", "ln", "readlink")

    def setUp(self):
        super().setUp()
        self.fake("udevadm", "exit 97")
        self.repo = self.tree(*PARTS)
        for name in BINS:
            self.write(f"repo/target/release/{name}", f"#!/bin/sh\n# {name}\n", 0o755)
        self.dest = self.tmp / "dest"

    def install(self, *args):
        argv = ["sh", self.repo / "scripts/install.sh", "--destdir", self.dest, *args]
        return self.run_script(argv)

    def installed(self):
        return {
            str(p.relative_to(self.dest)) for p in self.dest.rglob("*") if p.is_file()
        }

    def mode(self, rel):
        return stat.S_IMODE((self.dest / rel).stat().st_mode)

    def test_help_prints_the_usage_comment(self):
        result = self.run_script(["sh", self.repo / "scripts/install.sh", "--help"])
        self.assert_ran(result)
        self.assertIn("Install Nordlys from a built tree.", result.stdout)
        self.assertIn("--uninstall", result.stdout)
        self.assertNotIn("set -eu", result.stdout)
        self.assertFalse(any(line.startswith("#") for line in result.stdout.splitlines()))

    def test_an_unknown_option_is_refused(self):
        result = self.install("--frobnicate")
        self.assert_ran(result, 2)
        self.assertIn("unknown option --frobnicate", result.stderr)
        self.assertFalse(self.dest.exists())

    def test_it_needs_a_release_build_first(self):
        (self.repo / "target/release/nordlysctl").unlink()
        result = self.install()
        self.assert_ran(result, 1)
        self.assertIn("nordlysctl is missing", result.stderr)
        self.assertFalse(self.dest.exists())

    def test_a_default_install_goes_under_usr_local(self):
        result = self.install()
        self.assert_ran(result)
        self.assertIn(f"Installed Nordlys into {self.dest}/usr/local.", result.stdout)
        self.assertNotIn("Log out", result.stdout)
        units = {
            f"usr/local/lib/systemd/user/{u.name}"
            for u in (self.repo / "session/systemd").iterdir()
        }
        expected = units | {
            *(f"usr/local/bin/{b}" for b in (*BINS, *SCRIPTS)),
            "usr/local/share/nordlys/nordlys-autostart",
            "usr/local/share/nordlys/xdg/mimeapps.list",
            "usr/share/wayland-sessions/nordlys-comp.desktop",
            "usr/local/share/xdg-desktop-portal/nordlys-portals.conf",
            "usr/local/share/applications/nordlys-settings.desktop",
            *(
                f"usr/share/sddm/themes/nordlys/{name}"
                for name in ("Main.qml", "metadata.desktop", "theme.conf", "wallpaper.jpg")
            ),
            "usr/local/share/nordlys/hyprland.lua",
            "usr/local/share/nordlys/hypridle.conf",
            "usr/local/share/nordlys/wallpaper.jpg",
            "usr/local/share/nordlys/nordlys.toml.example",
            "usr/share/wayland-sessions/nordlys.desktop",
            "etc/pam.d/nordlys",
        }
        self.assertEqual(self.installed(), expected)
        for name in (*BINS, *SCRIPTS):
            self.assertEqual(self.mode(f"usr/local/bin/{name}"), 0o755, name)
        self.assertEqual(self.mode("usr/local/share/nordlys/nordlys-autostart"), 0o755)
        for unit in units:
            self.assertEqual(self.mode(unit), 0o644, unit)
            body = (self.dest / unit).read_text()
            self.assertNotIn("@BINDIR@", body, unit)
            self.assertNotIn("@DATADIR@", body, unit)
        notify = self.dest / "usr/local/lib/systemd/user/nordlys-notify.service"
        self.assertIn("ExecStart=/usr/local/bin/nordlys-shell notify", notify.read_text())
        wallpaper = self.dest / "usr/local/lib/systemd/user/nordlys-wallpaper.service"
        self.assertIn("/usr/local/share/nordlys/wallpaper.jpg", wallpaper.read_text())
        desktop = (self.dest / "usr/share/wayland-sessions/nordlys.desktop").read_text()
        self.assertIn("Exec=/usr/local/bin/nordlys-session\n", desktop)
        self.assertIn("TryExec=/usr/local/bin/nordlys-session\n", desktop)
        own = (self.dest / "usr/share/wayland-sessions/nordlys-comp.desktop").read_text()
        self.assertIn("Exec=/usr/local/bin/nordlys-session --comp\n", own)
        self.assertIn("TryExec=/usr/local/bin/nordlys-comp\n", own)
        self.assertEqual(self.calls(), [], "udevadm must not run for a package")

    def test_a_usr_prefix_keeps_the_plain_session_command(self):
        self.assert_ran(self.install("--prefix", "/usr"))
        desktop = (self.dest / "usr/share/wayland-sessions/nordlys.desktop").read_text()
        self.assertIn("\nExec=nordlys-session\n", desktop)
        self.assertTrue((self.dest / "usr/bin/nordlys-shell").is_file())

    def test_every_session_entry_stays_plain_under_usr(self):
        self.assert_ran(self.install("--prefix", "/usr"))
        own = (self.dest / "usr/share/wayland-sessions/nordlys-comp.desktop").read_text()
        self.assertIn("\nExec=nordlys-session --comp\n", own)
        self.assertIn("\nTryExec=nordlys-comp\n", own)

    def test_an_edited_pam_file_survives_a_reinstall(self):
        self.write("dest/etc/pam.d/nordlys", "auth include system-auth\n")
        self.assert_ran(self.install())
        self.assertEqual(
            (self.dest / "etc/pam.d/nordlys").read_text(), "auth include system-auth\n"
        )

    def test_a_vm_guest_gets_the_agent_and_its_rule(self):
        self.assert_ran(self.install("--vm-guest"))
        self.assertEqual(self.mode("usr/local/bin/nordlys-vm-agent"), 0o755)
        self.assertTrue((self.dest / "etc/udev/rules.d/70-nordlys-vm.rules").is_file())
        self.assertTrue((self.dest / "usr/local/share/nordlys/vm-guest").is_file())
        self.assertEqual(self.calls(), [], "no udevadm when packaging")

    def sddm_links(self):
        return {
            p.name: str(p.readlink())
            for p in (self.dest / "etc/sddm/wayland-sessions").iterdir() if p.is_symlink()
        }

    def custom_session_dir(self):
        """SDDM told to read only a folder of hand-picked links."""
        self.write(
            "dest/usr/lib/sddm/sddm.conf.d/default.conf",
            "[Wayland]\nSessionDir=/usr/local/share/wayland-sessions,/usr/share/wayland-sessions\n"
            "[X11]\nSessionDir=/usr/share/xsessions\n",
        )
        self.write(
            "dest/etc/sddm.conf.d/10-display.conf",
            "[Wayland]\nSessionDir = /etc/sddm/wayland-sessions/\n",
        )
        (self.dest / "etc/sddm/wayland-sessions").mkdir(parents=True)
        (self.dest / "etc/sddm/wayland-sessions/gnome.desktop").symlink_to(
            "/usr/share/wayland-sessions/gnome.desktop"
        )

    def test_sddm_reading_another_folder_gets_links_there(self):
        self.custom_session_dir()
        result = self.install()
        self.assert_ran(result)
        self.assertIn("Linked the Nordlys sessions into /etc/sddm/wayland-sessions", result.stdout)
        self.assertEqual(self.sddm_links(), {
            "gnome.desktop": "/usr/share/wayland-sessions/gnome.desktop",
            "nordlys.desktop": "/usr/share/wayland-sessions/nordlys.desktop",
            "nordlys-comp.desktop": "/usr/share/wayland-sessions/nordlys-comp.desktop",
        })
        # Installing again replaces the links instead of failing.
        self.assert_ran(self.install())

    def test_sddm_s_usual_folders_get_no_links(self):
        self.write(
            "dest/usr/lib/sddm/sddm.conf.d/default.conf",
            "[Wayland]\nSessionDir=/usr/local/share/wayland-sessions,/usr/share/wayland-sessions\n",
        )
        (self.dest / "etc/sddm/wayland-sessions").mkdir(parents=True)
        result = self.install()
        self.assert_ran(result)
        self.assertNotIn("Linked", result.stdout)
        self.assertEqual(self.sddm_links(), {})

    def test_a_missing_sddm_folder_is_reported_not_made(self):
        self.write("dest/etc/sddm.conf", "[Wayland]\nSessionDir=/etc/sddm/wayland-sessions\n")
        result = self.install()
        self.assert_ran(result)
        self.assertIn("doesn't exist", result.stderr)
        self.assertFalse((self.dest / "etc/sddm/wayland-sessions").exists())

    def test_uninstall_removes_only_its_own_sddm_links(self):
        self.custom_session_dir()
        self.assert_ran(self.install())
        self.assert_ran(self.install("--uninstall"))
        self.assertEqual(self.sddm_links(), {
            "gnome.desktop": "/usr/share/wayland-sessions/gnome.desktop",
        })

    def test_uninstall_removes_everything_but_pam(self):
        self.assert_ran(self.install("--vm-guest"))
        self.write("dest/usr/local/bin/unrelated", "keep me\n")
        result = self.install("--uninstall")
        self.assert_ran(result)
        self.assertIn("in place; remove it by hand", result.stdout)
        self.assertEqual(self.installed(), {"etc/pam.d/nordlys", "usr/local/bin/unrelated"})

    def test_uninstalling_twice_is_fine(self):
        self.assert_ran(self.install("--uninstall"))
        self.assert_ran(self.install("--uninstall"))


if __name__ == "__main__":
    unittest.main()
