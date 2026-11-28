"""session/nordlys-session with a fake Hyprland and systemctl."""

import os
import shutil
import unittest

from support import Sandbox

SYSTEM_DATA = ("/usr/local/share/nordlys", "/usr/share/nordlys")
ENV_DUMP = """
for v in NORDLYS_DATA NORDLYS_IDLE XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP XDG_SESSION_TYPE \\
    XDG_CONFIG_DIRS XCURSOR_SIZE QT_QPA_PLATFORMTHEME QT_QPA_PLATFORM GDK_BACKEND \\
    MOZ_ENABLE_WAYLAND ELECTRON_OZONE_PLATFORM_HINT SSH_AUTH_SOCK GNOME_KEYRING_CONTROL \\
    AQ_DRM_DEVICES NORDLYS_TEST_PLAIN; do
    eval "echo \\"$v=\\${$v-unset}\\"" >> "$NORDLYS_TEST_ENV"
done
exit ${NORDLYS_TEST_STATUS:-0}
"""


class Session(Sandbox):
    REAL = ("sh", "dirname", "readlink", "mkdir", "cp")

    def setUp(self):
        super().setUp()
        # An installed layout: prefix/bin beside prefix/share/nordlys.
        self.script = self.tmp / "prefix/bin/nordlys-session"
        self.script.parent.mkdir(parents=True)
        shutil.copy2(self.tree("session/nordlys-session") / "session/nordlys-session", self.script)
        self.data = self.tmp / "prefix/share/nordlys"
        self.write("prefix/share/nordlys/hyprland.lua", "-- config\n")
        self.write("prefix/share/nordlys/nordlys.toml.example", "# example\n")
        self.fake("start-hyprland", ENV_DUMP)
        self.fake("Hyprland", ENV_DUMP)
        self.fake("systemctl")
        self.seen = self.tmp / "env.txt"

    def start(self, *args, **extra):
        self.seen.unlink(missing_ok=True)
        self.log.unlink(missing_ok=True)
        return self.run_script(
            ["sh", self.script, *args], env=self.env(NORDLYS_TEST_ENV=self.seen, **extra)
        )

    def keyring(self, has_socket_unit=True, has_ssh_agent=True):
        """gnome-keyring installed, with or without its systemd units."""
        self.fake("gnome-keyring-daemon", "echo GNOME_KEYRING_CONTROL=/run/keyring")
        self.fake(
            "systemctl",
            f"""case "$*" in
                "--user cat gnome-keyring-daemon.socket") exit {int(not has_socket_unit)} ;;
                "--user start gcr-ssh-agent.socket") exit {int(not has_ssh_agent)} ;;
            esac""",
        )

    def session_env(self):
        return dict(line.split("=", 1) for line in self.seen.read_text().splitlines())

    def test_it_starts_hyprland_with_the_installed_config(self):
        self.assert_ran(self.start())
        env = self.session_env()
        self.assertEqual(env["NORDLYS_DATA"], str(self.data.resolve()))
        self.assertEqual(env["XDG_CURRENT_DESKTOP"], "Hyprland")
        self.assertEqual(env["XDG_SESSION_DESKTOP"], "nordlys")
        self.assertEqual(env["XDG_SESSION_TYPE"], "wayland")
        self.assertEqual(env["NORDLYS_IDLE"], "unset")
        self.assertEqual(
            self.calls()[0],
            f"start-hyprland -- -c {self.data.resolve()}/hyprland.lua",
        )

    def test_without_start_hyprland_it_runs_hyprland(self):
        (self.bin / "start-hyprland").unlink()
        self.assert_ran(self.start())
        self.assertEqual(self.calls()[0], f"Hyprland -c {self.data.resolve()}/hyprland.lua")

    def test_nordlys_data_wins(self):
        self.write("mine/hyprland.lua", "-- mine\n")
        self.assert_ran(self.start(NORDLYS_DATA=self.tmp / "mine"))
        self.assertEqual(self.session_env()["NORDLYS_DATA"], str((self.tmp / "mine").resolve()))

    def test_a_nordlys_data_without_a_config_is_passed_over(self):
        (self.tmp / "empty").mkdir()
        self.assert_ran(self.start(NORDLYS_DATA=self.tmp / "empty"))
        self.assertEqual(self.session_env()["NORDLYS_DATA"], str(self.data.resolve()))

    def test_it_finds_its_data_through_a_symlink(self):
        link = self.tmp / "elsewhere/nordlys-session"
        link.parent.mkdir()
        link.symlink_to(self.script)
        self.script = link
        self.assert_ran(self.start())
        self.assertEqual(self.session_env()["NORDLYS_DATA"], str(self.data.resolve()))

    @unittest.skipIf(
        any(os.path.exists(f"{d}/hyprland.lua") for d in SYSTEM_DATA),
        "Nordlys is installed on this machine",
    )
    def test_with_no_config_anywhere_it_says_what_to_set(self):
        (self.data / "hyprland.lua").unlink()
        result = self.start()
        self.assert_ran(result, 1)
        self.assertIn("can't find hyprland.lua (set NORDLYS_DATA)", result.stderr)
        self.assertEqual(self.calls(), [])

    def test_the_first_login_gets_a_config_to_edit(self):
        self.assert_ran(self.start())
        config = self.home / ".config/nordlys/nordlys.toml"
        self.assertEqual(config.read_text(), "# example\n")
        config.write_text("# edited\n")
        self.assert_ran(self.start())
        self.assertEqual(config.read_text(), "# edited\n", "never overwritten")

    def test_xdg_config_home_is_respected(self):
        self.write("cfg/nordlys/hypridle.conf", "# mine\n")
        self.assert_ran(self.start(XDG_CONFIG_HOME=self.tmp / "cfg"))
        self.assertTrue((self.tmp / "cfg/nordlys/nordlys.toml").is_file())
        self.assertEqual(
            self.session_env()["NORDLYS_IDLE"], str(self.tmp / "cfg/nordlys/hypridle.conf")
        )

    def test_without_an_example_no_config_is_made(self):
        (self.data / "nordlys.toml.example").unlink()
        self.assert_ran(self.start())
        self.assertFalse((self.home / ".config/nordlys").exists())

    def test_afterwards_the_services_stop_and_the_status_is_hyprlands(self):
        result = self.start(NORDLYS_TEST_STATUS=3)
        self.assert_ran(result, 3)
        self.assertEqual(
            self.calls()[1:],
            [
                "systemctl --user stop nordlys-session.target",
                "systemctl --user unset-environment WAYLAND_DISPLAY DISPLAY NORDLYS_SOCKET "
                "HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP "
                "NORDLYS_DATA NORDLYS_IDLE XDG_CONFIG_DIRS QT_QPA_PLATFORMTHEME SSH_AUTH_SOCK",
            ],
        )

    def test_without_systemd_it_just_exits(self):
        (self.bin / "systemctl").unlink()
        self.assert_ran(self.start(NORDLYS_TEST_STATUS=5), 5)
        self.assertEqual(len(self.calls()), 1)

    def test_the_default_apps_sit_behind_your_own(self):
        data = self.data.resolve()
        self.assert_ran(self.start())
        self.assertEqual(self.session_env()["XDG_CONFIG_DIRS"], f"{data}/xdg:/etc/xdg")
        self.assert_ran(self.start(XDG_CONFIG_DIRS="/a:/b"))
        self.assertEqual(self.session_env()["XDG_CONFIG_DIRS"], f"{data}/xdg:/a:/b")

    def test_your_env_file_reaches_the_compositor(self):
        self.write(
            "home/.config/nordlys/env",
            'export AQ_DRM_DEVICES="$HOME/card1:$HOME/card0"\nNORDLYS_TEST_PLAIN=yes\n',
        )
        self.assert_ran(self.start())
        env = self.session_env()
        self.assertEqual(env["AQ_DRM_DEVICES"], f"{self.home}/card1:{self.home}/card0")
        # Plain assignments are exported too.
        self.assertEqual(env["NORDLYS_TEST_PLAIN"], "yes")

    def test_without_an_env_file_nothing_is_added(self):
        self.assert_ran(self.start())
        self.assertEqual(self.session_env()["AQ_DRM_DEVICES"], "unset")

    def test_an_env_file_with_a_syntax_error_is_skipped(self):
        self.write("home/.config/nordlys/env", "AQ_DRM_DEVICES=x\nif then fi (\n")
        result = self.start()
        self.assert_ran(result)
        self.assertEqual(self.session_env()["AQ_DRM_DEVICES"], "unset")
        self.assertIn("syntax error", result.stderr)

    def test_comp_runs_nordlys_comp_with_the_toolkit_variables(self):
        self.fake("nordlys-comp", ENV_DUMP)
        result = self.start("--comp", NORDLYS_TEST_STATUS=4)
        self.assert_ran(result, 4)
        self.assertEqual(
            self.calls()[0],
            f"nordlys-comp --tty --run sh '{self.data.resolve()}/nordlys-autostart'",
        )
        env = self.session_env()
        for name, value in {
            "XDG_CURRENT_DESKTOP": "Nordlys",
            "XDG_SESSION_DESKTOP": "nordlys",
            "XCURSOR_SIZE": "24",
            "QT_QPA_PLATFORMTHEME": "qt6ct",
            "QT_QPA_PLATFORM": "wayland;xcb",
            "GDK_BACKEND": "wayland,x11",
            "MOZ_ENABLE_WAYLAND": "1",
            "ELECTRON_OZONE_PLATFORM_HINT": "auto",
        }.items():
            self.assertEqual(env[name], value, name)
        self.assertFalse(any(c.startswith(("Hyprland", "start-hyprland")) for c in self.calls()))
        self.assertIn("systemctl --user stop nordlys-session.target", self.calls())

    def test_comp_keeps_your_cursor_size(self):
        self.fake("nordlys-comp", ENV_DUMP)
        self.assert_ran(self.start("--comp", XCURSOR_SIZE=48))
        self.assertEqual(self.session_env()["XCURSOR_SIZE"], "48")

    def test_hyprland_gets_no_toolkit_variables_from_here(self):
        self.assert_ran(self.start())
        self.assertEqual(self.session_env()["QT_QPA_PLATFORM"], "unset")

    def test_without_gnome_keyring_nothing_is_started(self):
        self.assert_ran(self.start())
        self.assertFalse(any("keyring" in c or "gcr" in c for c in self.calls()[:1]))
        env = self.session_env()
        self.assertEqual(env["SSH_AUTH_SOCK"], "unset")
        self.assertEqual(env["GNOME_KEYRING_CONTROL"], "unset")

    def test_the_keyring_starts_through_its_socket_and_lends_its_ssh_agent(self):
        self.keyring()
        self.assert_ran(self.start())
        self.assertIn("systemctl --user start gnome-keyring-daemon.socket", self.calls())
        self.assertFalse(any(c.startswith("gnome-keyring-daemon") for c in self.calls()))
        self.assertEqual(self.session_env()["SSH_AUTH_SOCK"], f"{self.tmp}/run/gcr/ssh")

    def test_without_the_socket_unit_the_daemon_is_started(self):
        self.keyring(has_socket_unit=False, has_ssh_agent=False)
        self.assert_ran(self.start())
        self.assertIn("gnome-keyring-daemon --start --components=secrets", self.calls())
        env = self.session_env()
        self.assertEqual(env["GNOME_KEYRING_CONTROL"], "/run/keyring")
        self.assertEqual(env["SSH_AUTH_SOCK"], "unset")

    def test_your_own_ssh_agent_is_kept(self):
        self.keyring()
        self.assert_ran(self.start(SSH_AUTH_SOCK="/mine"))
        self.assertEqual(self.session_env()["SSH_AUTH_SOCK"], "/mine")
        self.assertNotIn("systemctl --user start gcr-ssh-agent.socket", self.calls())


if __name__ == "__main__":
    unittest.main()
