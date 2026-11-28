"""The shell commands inside the systemd units, run the way systemd runs them:
`$$` becomes `$` and @DATADIR@ is the install's data directory."""

import re
import unittest

from support import Sandbox, read


class UnitCommands(Sandbox):
    REAL = ("sh", "cat")

    def command(self, unit):
        line = re.search(
            r"^ExecStart=/bin/sh -c '(.*)'$", read(f"session/systemd/{unit}"), re.M
        )
        self.assertIsNotNone(line, unit)
        return line.group(1).replace("@DATADIR@", "/opt/data").replace("$$", "$")

    def run_unit(self, unit, **env):
        self.log.unlink(missing_ok=True)
        return self.run_script(["sh", "-c", self.command(unit)], env=self.env(**env))

    def wallpaper(self, **env):
        self.assert_ran(self.run_unit("nordlys-wallpaper.service", **env))
        return self.calls()

    def test_the_nordlys_wallpaper_fills_the_screen(self):
        self.fake("swaybg")
        self.assertEqual(self.wallpaper(), ["swaybg -m fill -i /opt/data/wallpaper.jpg"])

    def test_the_wallpaper_and_fit_picked_in_settings_win(self):
        self.fake("swaybg")
        for config, env in (
            ("home/.config/nordlys", {}),
            ("xdg/nordlys", {"XDG_CONFIG_HOME": self.tmp / "xdg"}),
        ):
            for mode in ("fill", "fit", "center", "tile"):
                with self.subTest(config=config, mode=mode):
                    mine = self.write(f"{config}/wallpaper", "image\n")
                    self.write(f"{config}/wallpaper-mode", f"{mode}\n")
                    self.assertEqual(self.wallpaper(**env), [f"swaybg -m {mode} -i {mine}"])

    def test_an_unknown_fit_falls_back_to_fill(self):
        self.fake("swaybg")
        self.write("home/.config/nordlys/wallpaper-mode", "stretch; reboot\n")
        self.assertEqual(self.wallpaper(), ["swaybg -m fill -i /opt/data/wallpaper.jpg"])

    def test_idle_uses_the_nordlys_rules_unless_you_have_your_own(self):
        self.fake("hypridle")
        self.assert_ran(self.run_unit("nordlys-idle.service"))
        self.assertEqual(self.calls(), ["hypridle -c /opt/data/hypridle.conf"])
        self.assert_ran(self.run_unit("nordlys-idle.service", NORDLYS_IDLE="/my idle.conf"))
        self.assertEqual(self.calls(), ["hypridle -c /my idle.conf"])

    def test_clipboard_history_needs_cliphist(self):
        self.fake("wl-paste")
        self.assert_ran(self.run_unit("nordlys-clipboard.service"))
        self.assertEqual(self.calls(), [])
        self.fake("cliphist")
        self.assert_ran(self.run_unit("nordlys-clipboard.service"))
        self.assertEqual(self.calls(), ["wl-paste --watch cliphist store"])


if __name__ == "__main__":
    unittest.main()
