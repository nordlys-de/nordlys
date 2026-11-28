"""session/nordlys-autostart, what a nordlys-comp session starts, run with
fake programs. The throwaway copy looks for polkit agents inside the test's
own directory, never in /usr/lib."""

import unittest

from support import Sandbox, read

VARS = (
    "WAYLAND_DISPLAY DISPLAY NORDLYS_SOCKET XDG_CURRENT_DESKTOP XDG_SESSION_TYPE "
    "XDG_SESSION_DESKTOP NORDLYS_DATA NORDLYS_IDLE PATH XDG_CONFIG_DIRS "
    "QT_QPA_PLATFORMTHEME SSH_AUTH_SOCK"
)
PIECES = ("notify", "bar", "shelf", "close-button")
AGENTS = (
    "hyprpolkitagent/hyprpolkitagent",
    "polkit-gnome/polkit-gnome-authentication-agent-1",
)


class Autostart(Sandbox):
    REAL = ("sh",)

    def setUp(self):
        super().setUp()
        text = read("session/nordlys-autostart")
        self.assertIn("/usr/lib/", text)
        self.script = self.write(
            "autostart", text.replace("/usr/lib/", f"{self.tmp}/lib/")
        )
        self.fake("sleep")
        self.fake("nordlys-shell")
        self.fake("nordlys-theme")
        self.fake("dbus-update-activation-environment")
        self.systemctl()

    def systemctl(self, units=True, import_ok=True, polkit=True):
        self.fake(
            "systemctl",
            f"""case "$*" in
                "--user cat nordlys-session.target") exit {int(not units)} ;;
                "--user import-environment"*) exit {int(not import_ok)} ;;
                "--user start hyprpolkitagent") exit {int(not polkit)} ;;
            esac""",
        )

    def start(self, **extra):
        self.assert_ran(self.run_script(["sh", self.script], env=self.env(**extra)))
        return self.calls()

    def shell_calls(self, calls):
        return sorted(c.split(" ", 1)[1] for c in calls if c.startswith("nordlys-shell "))

    def test_the_services_get_the_environment_then_start(self):
        calls = self.start()
        imported = f"dbus-update-activation-environment --systemd {VARS}"
        self.assertIn(imported, calls)
        start = "systemctl --user --no-block start nordlys-session.target"
        self.assertIn(start, calls)
        self.assertLess(calls.index(imported), calls.index(start))
        self.assertEqual(self.shell_calls(calls), ["welcome --first-run"])
        self.assertFalse(any("import-environment" in c for c in calls))

    def test_without_dbus_systemd_imports_the_environment(self):
        self.fake("dbus-update-activation-environment", "exit 1")
        self.assertIn(f"systemctl --user import-environment {VARS}", self.start())

    def test_without_the_units_each_piece_is_started_by_hand(self):
        self.systemctl(units=False)
        self.fake("swaybg")
        self.fake("hypridle")
        calls = self.start(NORDLYS_DATA="/data dir", NORDLYS_IDLE="/my idle.conf")
        self.assertEqual(
            self.shell_calls(calls), sorted([*PIECES, "welcome --first-run"])
        )
        self.assertIn("swaybg -m fill -i /data dir/wallpaper.jpg", calls)
        self.assertIn("hypridle -c /my idle.conf", calls)
        self.assertFalse(any("start nordlys-session.target" in c for c in calls))

    def test_by_hand_falls_back_to_the_installed_data(self):
        self.systemctl(units=False)
        self.fake("swaybg")
        self.fake("hypridle")
        calls = self.start()
        self.assertIn("swaybg -m fill -i /usr/share/nordlys/wallpaper.jpg", calls)
        self.assertIn("hypridle -c /usr/share/nordlys/hypridle.conf", calls)

    def test_missing_extras_are_skipped(self):
        self.systemctl(units=False)
        calls = self.start()
        for program in ("swaybg", "hypridle", "xdg-user-dirs-update", "nordlys-vm-agent"):
            self.assertFalse(any(c.startswith(program) for c in calls), program)

    def test_the_theme_folders_and_vm_agent_run(self):
        self.fake("xdg-user-dirs-update")
        self.fake("nordlys-vm-agent")
        calls = self.start()
        for wanted in ("nordlys-theme --quiet --watch", "xdg-user-dirs-update ", "nordlys-vm-agent "):
            self.assertIn(wanted, calls)

    def agent(self, name):
        path = self.write(
            f"lib/{name}", f"#!/bin/sh\necho 'agent {name}' >> '{self.log}'\n", 0o755
        )
        return path

    def agents_run(self, calls):
        return [c.removeprefix("agent ") for c in calls if c.startswith("agent ")]

    def test_the_polkit_service_is_enough(self):
        for name in AGENTS:
            self.agent(name)
        calls = self.start()
        self.assertIn("systemctl --user start hyprpolkitagent", calls)
        self.assertEqual(self.agents_run(calls), [])

    def test_without_the_service_one_agent_is_run(self):
        self.systemctl(polkit=False)
        for name in AGENTS:
            self.agent(name)
        self.assertEqual(self.agents_run(self.start()), [AGENTS[0]])

    def test_the_gnome_agent_is_the_second_choice(self):
        self.systemctl(polkit=False)
        self.agent(AGENTS[1])
        self.assertEqual(self.agents_run(self.start()), [AGENTS[1]])


if __name__ == "__main__":
    unittest.main()
