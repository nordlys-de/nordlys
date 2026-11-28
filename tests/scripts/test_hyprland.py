"""session/hyprland.lua: the Lua tests in hyprland_test.lua, then the shell
commands it would run, executed for real against fake programs."""

import re
import shlex
import shutil
import subprocess
import time
import unittest
from pathlib import Path

from support import REPO, Sandbox, read

LUA = shutil.which("lua5.4") or shutil.which("lua")
HARNESS = Path(__file__).with_name("hyprland_test.lua")
CONFIG = REPO / "session/hyprland.lua"


@unittest.skipIf(LUA is None, "lua isn't installed")
class Config(unittest.TestCase):
    def test_the_lua_tests_pass(self):
        result = subprocess.run(
            [LUA, str(HARNESS), str(CONFIG)], capture_output=True, text=True, timeout=60
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertRegex(result.stdout, r"\n?\d+ tests, 0 failed")


@unittest.skipIf(LUA is None, "lua isn't installed")
class Commands(Sandbox):
    REAL = ("sh", "cat", "sleep")

    def commands(self, data="/data dir", idle="/idle.conf"):
        result = subprocess.run(
            [LUA, str(HARNESS), str(CONFIG), "commands"],
            capture_output=True, text=True, timeout=60, check=True,
            env={"NORDLYS_TEST_DATA": data, "NORDLYS_TEST_IDLE": idle},
        )
        parts = result.stdout.split("\0")[:-1]
        return dict(zip(parts[::2], parts[1::2]))

    def run_command(self, command, **env):
        """Run a command the way Hyprland does, through a shell."""
        self.log.unlink(missing_ok=True)
        return self.run_script(["sh", "-c", command], env=self.env(**env))

    def settle(self, count):
        deadline = time.monotonic() + 5
        while len(self.calls()) < count and time.monotonic() < deadline:
            time.sleep(0.02)
        time.sleep(0.1)
        return self.calls()

    def test_every_command_is_valid_shell(self):
        commands = self.commands()
        self.assertGreater(len(commands), 15)
        for name, command in commands.items():
            with self.subTest(name=name):
                words = shlex.split(command)
                if words[:2] == ["sh", "-c"]:
                    check = ["sh", "-n", "-c", words[2]]
                else:
                    check = ["sh", "-n", "-c", command]
                result = subprocess.run(check, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, f"{command}\n{result.stderr}")

    PIECES = (
        "nordlys-shell notify", "nordlys-shell ", "nordlys-shell shelf",
        "nordlys-shell close-button",
    )

    def fake_autostart(self, systemctl_status, import_status=0):
        self.fake("dbus-update-activation-environment", f"exit {import_status}")
        self.fake("systemctl", f"exit {systemctl_status}")
        for program in ("nordlys-shell", "swaybg", "hypridle", "wl-paste"):
            self.fake(program)

    def test_autostart_hands_the_session_to_systemd(self):
        self.fake_autostart(0)
        self.assert_ran(self.run_command(self.commands()["start1"]))
        calls = self.settle(2)
        self.assertTrue(calls[0].startswith("dbus-update-activation-environment --systemd"))
        for variable in ("WAYLAND_DISPLAY", "HYPRLAND_INSTANCE_SIGNATURE", "NORDLYS_DATA",
                         "XDG_CONFIG_DIRS", "SSH_AUTH_SOCK"):
            self.assertIn(variable, calls[0])
        self.assertEqual(calls[1:], ["systemctl --user --no-block start nordlys-session.target"])

    def test_without_systemd_units_each_piece_is_started_by_hand(self):
        for failing in ({"systemctl_status": 1}, {"systemctl_status": 0, "import_status": 1}):
            with self.subTest(**failing):
                self.log.unlink(missing_ok=True)
                self.fake_autostart(**failing)
                # A folder with a space: the quoting has to survive two shells.
                self.assert_ran(self.run_command(self.commands()["start1"]))
                started = [
                    c for c in self.settle(9 if failing["systemctl_status"] else 8)
                    if not c.startswith(("dbus-", "systemctl "))
                ]
                self.assertEqual(
                    sorted(started),
                    sorted([
                        *self.PIECES,
                        "swaybg -m fill -i /data dir/wallpaper.jpg",
                        "hypridle -c /idle.conf",
                    ]),
                )

    def test_by_hand_includes_clipboard_history_when_installed(self):
        self.fake_autostart(1)
        self.fake("cliphist")
        self.run_command(self.commands()["start1"])
        self.assertIn("wl-paste --watch cliphist store", self.settle(10))

    def test_the_theme_is_applied_at_every_login(self):
        self.fake("nordlys-theme")
        self.run_command(self.commands()["start2"])
        self.assertEqual(self.calls(), ["nordlys-theme --quiet --watch"])

    def test_the_home_folders_are_made_where_the_tool_exists(self):
        command = self.commands()["start3"]
        self.run_command(command)
        self.assertEqual(self.calls(), [])
        self.fake("xdg-user-dirs-update")
        self.run_command(command)
        self.assertEqual(self.calls(), ["xdg-user-dirs-update "])

    def test_the_welcome_card_greets_the_first_login(self):
        self.fake("sleep")
        self.fake("nordlys-shell")
        self.run_command(self.commands()["start6"])
        self.assertEqual(self.calls(), ["sleep 2", "nordlys-shell welcome --first-run"])

    def test_the_vm_agent_starts_only_where_installed(self):
        command = self.commands()["start4"]
        self.run_command(command)
        self.assertEqual(self.calls(), [])
        self.fake("nordlys-vm-agent")
        self.run_command(command)
        self.assertEqual(self.calls(), ["nordlys-vm-agent "])

    def test_the_polkit_agent_starts_through_systemd(self):
        self.fake("systemctl")
        self.run_command(self.commands()["start5"])
        self.assertEqual(self.calls(), ["systemctl --user start hyprpolkitagent"])

    def test_the_terminal_is_yours_else_the_first_installed(self):
        command = self.commands()["SUPER + RETURN"]
        self.fake("kitty")
        self.fake("alacritty")
        self.run_command(command)
        self.assertEqual(self.calls(), ["kitty "])
        self.fake("myterm")
        self.run_command(command, TERMINAL="myterm")
        self.assertEqual(self.calls(), ["myterm "])

    def test_the_editor_is_yours_else_the_first_installed(self):
        command = self.commands()["SUPER + E"]
        self.fake("kate")
        self.run_command(command)
        self.assertEqual(self.calls(), ["kate "])

    def screenshot_fakes(self):
        self.real("mkdir", "date")
        self.fake("slurp", "echo '1,2 3x4'")
        self.fake("grim", 'for last; do :; done; printf picture > "$last"')
        self.fake("wl-copy", 'cat > "$NL_CLIP"')
        self.fake("notify-send")

    def check_saved(self, folder):
        shot, = folder.glob("*.png")
        self.assertRegex(shot.name, r"^\d{4}-\d\d-\d\d_\d\d-\d\d-\d\d\.png$")
        self.assertEqual((self.tmp / "clip").read_text(), "picture", "also copied")
        self.assertIn(f"Screenshot saved {shot}", self.calls()[-1])
        return shot

    def test_print_saves_the_screen_into_pictures(self):
        self.screenshot_fakes()
        self.run_command(self.commands()["Print"], NL_CLIP=self.tmp / "clip")
        shot = self.check_saved(self.home / "Pictures/Screenshots")
        self.assertIn(f"grim {shot}", self.calls())
        self.assertFalse(any(c.startswith("slurp") for c in self.calls()))

    def test_shift_print_saves_a_region_into_your_pictures_folder(self):
        self.screenshot_fakes()
        self.fake("xdg-user-dir", f"echo '{self.tmp}/My Photos'")
        self.run_command(self.commands()["SHIFT + Print"], NL_CLIP=self.tmp / "clip")
        shot = self.check_saved(self.tmp / "My Photos/Screenshots")
        self.assertIn(f"grim -g 1,2 3x4 {shot}", self.calls())

    def test_settings_falls_back_to_the_config_file(self):
        command = self.commands()["SUPER + comma"]
        self.fake("xdg-open")
        self.run_command(command)
        self.assertEqual(self.calls(), [f"xdg-open /home/u/.config/nordlys/nordlys.toml"])
        self.fake("nordlys-settings")
        self.run_command(command)
        self.assertEqual(self.calls(), ["nordlys-settings "])

    def test_a_screenshot_goes_to_the_clipboard(self):
        self.fake("slurp", "echo '1,2 3x4'")
        self.fake("grim", "printf picture")
        self.fake("wl-copy", 'cat > "$NL_CLIP"')
        clip = self.tmp / "clip"
        self.run_command(self.commands()["SUPER + SHIFT + S"], NL_CLIP=clip)
        self.assertEqual(clip.read_text(), "picture")
        self.assertIn("grim -g 1,2 3x4 -", self.calls())


@unittest.skipIf(LUA is None, "lua isn't installed")
class AgreesWithTheRest(unittest.TestCase):
    """The keys call programs by name; those names have to exist."""

    def test_stage_keys_use_real_nordlysctl_commands(self):
        ctl = read("crates/nordlysctl/src/main.rs")
        for word in re.findall(r'bind_stage\("\w+",\s+"([\w-]+)"', read("session/hyprland.lua")):
            self.assertIn(f'"{word}"', ctl, word)

    def test_shell_commands_name_real_pieces(self):
        shell = read("crates/nordlys-shell/src/main.rs")
        lua = read("session/hyprland.lua")
        pieces = set(re.findall(r'nordlys-shell ([a-z][\w-]*)', lua))
        self.assertTrue({"notify", "shelf", "launcher", "lock"} <= pieces, pieces)
        for piece in pieces:
            self.assertIn(f'"{piece}"', shell, piece)

    def test_the_launchers_stage_calls_exist(self):
        search = read("crates/nordlys-shell/src/search.rs")
        lua = read("session/hyprland.lua")
        called = re.findall(r'"nordlys_stage\.(\w+)\(\)"', search)
        self.assertTrue(called)
        for name in called:
            self.assertRegex(lua, rf"\n\s+{name} = ", name)


if __name__ == "__main__":
    unittest.main()
