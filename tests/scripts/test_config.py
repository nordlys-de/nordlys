"""The files that tie the session together: systemd units, the desktop
entry, hypridle, udev, PAM, the example nordlys.toml and CI. Each is checked
against the code and scripts that rely on it."""

import re
import shutil
import subprocess
import unittest
from xml.etree import ElementTree

from support import REPO, read

try:
    import tomllib
except ImportError:  # pragma: no cover  (Python < 3.11)
    tomllib = None

UNITS = REPO / "session/systemd"
QMLLINT = shutil.which("qmllint6") or shutil.which("qmllint", path="/usr/lib/qt6/bin")
MAIN_RS = read("crates/nordlys-shell/src/main.rs")


def unit(name):
    """{section: [(key, value)]}; unit files repeat keys such as Wants."""
    sections, current = {}, None
    for line in (UNITS / name).read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            current = sections.setdefault(line[1:-1], [])
            continue
        key, separator, value = line.partition("=")
        assert separator and current is not None, f"{name}: {line!r}"
        current.append((key, value))
    return sections


def values(sections, section, key):
    return [v for k, v in sections.get(section, []) if k == key]


def services():
    return sorted(p.name for p in UNITS.glob("*.service"))


class Units(unittest.TestCase):
    def test_the_target_wants_units_that_exist(self):
        target = unit("nordlys-session.target")
        wanted = " ".join(values(target, "Unit", "Wants")).split()
        ours = [w for w in wanted if w.startswith("nordlys")]
        self.assertTrue(ours)
        for name in ours:
            template = re.sub(r"@[^.]+(?=\.service)", "@", name)
            self.assertTrue((UNITS / template).is_file(), name)
        started = {re.sub(r"@[^.]+", "@", w) for w in ours}
        self.assertEqual(started, set(services()), "no service is forgotten")
        self.assertEqual(values(target, "Unit", "BindsTo"), ["graphical-session.target"])

    def test_every_service_belongs_to_the_session_and_restarts(self):
        for name in services():
            with self.subTest(unit=name):
                parsed = unit(name)
                self.assertEqual(values(parsed, "Unit", "PartOf"), ["nordlys-session.target"])
                self.assertEqual(values(parsed, "Service", "Restart"), ["on-failure"])
                self.assertEqual(len(values(parsed, "Service", "ExecStart")), 1)
                burst = int(values(parsed, "Unit", "StartLimitBurst")[0])
                window = int(values(parsed, "Unit", "StartLimitIntervalSec")[0])
                delay = int(values(parsed, "Service", "RestartSec")[0])
                self.assertLess(burst * delay, window, "a crash loop must be caught")

    def test_the_installer_fills_in_every_placeholder(self):
        script = read("scripts/install.sh")
        used = set()
        for path in UNITS.iterdir():
            used.update(re.findall(r"@[A-Z]+@", path.read_text()))
        self.assertEqual(used, {"@BINDIR@", "@DATADIR@"})
        for placeholder in used:
            self.assertIn(f"s#{placeholder}#", script)

    def test_the_target_starts_only_pieces_the_shell_knows(self):
        wanted = " ".join(values(unit("nordlys-session.target"), "Unit", "Wants"))
        pieces = re.findall(r"nordlys-shell@([^.]+)\.service", wanted)
        self.assertEqual(sorted(pieces), ["bar", "close-button", "shelf"])
        for piece in pieces:
            self.assertIn(f'"{piece}"', MAIN_RS)

    def test_apps_set_to_start_at_login_do(self):
        wanted = " ".join(values(unit("nordlys-session.target"), "Unit", "Wants"))
        self.assertIn("xdg-desktop-autostart.target", wanted.split())

    def test_the_notification_daemon_stops_the_others(self):
        conflicts = values(unit("nordlys-notify.service"), "Unit", "Conflicts")[0].split()
        self.assertEqual(set(conflicts), {"dunst.service", "mako.service", "swaync.service"})


def desktop_entry(rel, group="Desktop Entry"):
    """The keys of a desktop-style file that has one group."""
    lines = [l for l in read(rel).splitlines() if l and not l.startswith("#")]
    assert lines[0] == f"[{group}]", f"{rel}: starts with {lines[0]!r}"
    return dict(line.split("=", 1) for line in lines[1:])


SESSIONS = ("session/nordlys.desktop", "session/nordlys-comp.desktop")
SETTINGS = "crates/nordlys-settings/nordlys-settings.desktop"


class Desktop(unittest.TestCase):
    def test_the_hyprland_session_starts_the_installed_script(self):
        entry = desktop_entry(SESSIONS[0])
        self.assertEqual(entry["Type"], "Application")
        self.assertEqual(entry["Name"], "Nordlys")
        self.assertEqual(entry["Exec"], "nordlys-session")
        self.assertEqual(entry["TryExec"], "nordlys-session")
        self.assertIn("session/nordlys-session", read("scripts/install.sh"))

    def test_the_own_compositor_session_asks_for_it(self):
        entry = desktop_entry(SESSIONS[1])
        self.assertEqual(entry["Type"], "Application")
        self.assertEqual(entry["Exec"], "nordlys-session --comp")
        self.assertEqual(entry["TryExec"], "nordlys-comp", "hidden without the compositor")
        self.assertNotEqual(entry["Name"], desktop_entry(SESSIONS[0])["Name"])

    def test_the_desktop_names_are_the_ones_the_session_exports(self):
        script = read("session/nordlys-session")
        names = [desktop_entry(rel)["DesktopNames"] for rel in SESSIONS]
        self.assertEqual(names, ["Hyprland", "Nordlys"])
        for name in names:
            self.assertIn(f"export XDG_CURRENT_DESKTOP={name}\n", script)

    def test_the_own_session_has_portals_of_its_own(self):
        # Hyprland's come with xdg-desktop-portal-hyprland.
        name = desktop_entry(SESSIONS[1])["DesktopNames"]
        rel = f"session/xdg-desktop-portal/{name.lower()}-portals.conf"
        preferred = desktop_entry(rel, group="preferred")
        self.assertEqual(preferred["default"], "gtk")
        self.assertEqual(preferred["org.freedesktop.impl.portal.Secret"], "gnome-keyring")
        self.assertIn("xdg-desktop-portal-gtk", read("packaging/arch/PKGBUILD"))

    def test_settings_is_in_the_launcher(self):
        entry = desktop_entry(SETTINGS)
        self.assertEqual(entry["Exec"], "nordlys-settings %u")
        self.assertEqual(entry["TryExec"], "nordlys-settings")
        self.assertEqual(entry["Terminal"], "false")
        self.assertIn("Settings", entry["Categories"].split(";"))
        self.assertTrue(entry["Categories"].endswith(";"))

    @unittest.skipIf(shutil.which("desktop-file-validate") is None, "needs desktop-file-utils")
    def test_the_desktop_spec_accepts_them(self):
        # The validator doesn't know that display managers read DesktopNames,
        # or the desktop names these two sessions set.
        known = (
            'contains key "DesktopNames"',
            'contains an unregistered value "Nordlys"',
            'contains an unregistered value "Hyprland"',
        )
        for rel in (*SESSIONS, SETTINGS):
            with self.subTest(file=rel):
                result = subprocess.run(
                    ["desktop-file-validate", REPO / rel],
                    capture_output=True, text=True, timeout=20,
                )
                errors = [
                    line for line in (result.stdout + result.stderr).splitlines()
                    if ": error: " in line and not any(k in line for k in known)
                ]
                self.assertEqual(errors, [])


class DefaultApps(unittest.TestCase):
    def setUp(self):
        lines = [
            l for l in read("session/xdg/mimeapps.list").splitlines()
            if l and not l.startswith("#")
        ]
        self.header = lines[0]
        self.pairs = [line.split("=", 1) for line in lines[1:]]

    def test_each_type_is_listed_once(self):
        self.assertEqual(self.header, "[Default Applications]")
        types = [kind for kind, _ in self.pairs]
        self.assertEqual(sorted(types), sorted(set(types)))

    def test_every_line_names_desktop_files(self):
        for kind, apps in self.pairs:
            with self.subTest(type=kind):
                self.assertRegex(kind, r"^[\w.-]+/[\w.+-]+$")
                self.assertTrue(apps.endswith(";"))
                for app in apps.rstrip(";").split(";"):
                    self.assertRegex(app, r"^[\w.-]+\.desktop$")

    def test_the_everyday_files_open_in_something(self):
        types = {kind for kind, _ in self.pairs}
        for kind in ("inode/directory", "image/png", "video/mp4", "audio/mpeg",
                     "application/pdf", "text/plain"):
            self.assertIn(kind, types)


class LoginScreen(unittest.TestCase):
    THEME = "session/sddm/nordlys"

    def setUp(self):
        self.qml = read(f"{self.THEME}/Main.qml")
        self.config = desktop_entry(f"{self.THEME}/theme.conf", group="General")

    def test_sddm_is_pointed_at_the_theme(self):
        metadata = desktop_entry(f"{self.THEME}/metadata.desktop", group="SddmGreeterTheme")
        self.assertEqual(metadata["ConfigFile"], "theme.conf")
        self.assertEqual(metadata["QtVersion"], "6")
        self.assertEqual(metadata["MainScript"], "Main.qml")
        self.assertTrue((REPO / self.THEME / metadata["MainScript"]).is_file())
        self.assertIn("Current=nordlys", read("scripts/setup.sh"))
        self.assertIn('usr/share/sddm/themes/nordlys"', read("scripts/install.sh"))

    def test_the_wallpaper_it_names_is_installed_beside_it(self):
        self.assertTrue((REPO / "session" / self.config["background"]).is_file())
        self.assertIn('"$root/session/wallpaper.jpg" "$sddm_theme/"', read("scripts/install.sh"))

    def test_it_reads_the_settings_theme_conf_has(self):
        self.assertEqual(set(re.findall(r"config\.(\w+)", self.qml)), set(self.config))

    def test_logging_in_and_the_power_buttons_reach_sddm(self):
        for call in ("sddm.login(user.text, password.text, session.currentIndex)",
                     "sddm.suspend()", "sddm.reboot()", "sddm.powerOff()"):
            self.assertIn(call, self.qml)

    def test_the_password_is_hidden_and_cleared_after_a_failure(self):
        self.assertIn("echoMode: TextInput.Password", self.qml)
        self.assertRegex(self.qml, r'function onLoginFailed\(\) \{[^}]*password\.text = ""')

    @unittest.skipIf(QMLLINT is None, "needs qmllint")
    def test_the_qml_parses(self):
        result = subprocess.run(
            [QMLLINT, REPO / self.THEME / "Main.qml"],
            capture_output=True, text=True, timeout=60,
        )
        # Unknown SDDM globals are warnings; only a broken file counts here.
        self.assertNotRegex(result.stdout + result.stderr, r"(?i)syntax error|expected token")


class Dotfiles(unittest.TestCase):
    def test_fontconfig_prefers_the_nordlys_typefaces(self):
        root = ElementTree.fromstring(
            (REPO / "dotfiles/fontconfig/conf.d/60-nordlys.conf").read_bytes()
        )
        self.assertEqual(root.tag, "fontconfig")
        first = {
            a.findtext("family"): a.find("prefer").findtext("family")
            for a in root.findall("alias")
        }
        self.assertEqual(
            first,
            {"sans-serif": "Inter", "monospace": "JetBrains Mono", "serif": "Noto Serif"},
        )

    @unittest.skipIf(tomllib is None, "needs Python 3.11")
    def test_helix_uses_the_generated_theme(self):
        self.assertEqual(tomllib.loads(read("dotfiles/helix/config.toml"))["theme"], "nordlys")

    def test_foot_takes_its_colours_from_the_generated_file(self):
        lines = [
            l for l in read("dotfiles/foot/foot.ini").splitlines()
            if l and not l.startswith("#")
        ]
        self.assertEqual(lines[0], "include=~/.config/foot/nordlys.ini")
        for line in lines[1:]:
            self.assertRegex(line, r"^(\[[\w-]+\]|[\w.-]+=.*)$")

    def test_every_configured_app_is_suggested_by_the_package(self):
        pkgbuild = read("packaging/arch/PKGBUILD")
        apps = {p.name for p in (REPO / "dotfiles").iterdir()} - {"fontconfig"}
        self.assertTrue(apps)
        for app in apps:
            self.assertIn(f"'{app}", pkgbuild, app)


class Idle(unittest.TestCase):
    def setUp(self):
        self.text = read("session/hypridle.conf")
        self.listeners = re.findall(r"^listener \{\n(.*?)^\}", self.text, re.M | re.S)

    def pairs(self, block):
        return dict(
            (k.strip(), v.strip())
            for k, v in (l.split("=", 1) for l in block.splitlines() if "=" in l)
        )

    def test_braces_balance(self):
        code = "\n".join(
            l for l in self.text.splitlines() if not l.lstrip().startswith("#")
        )
        self.assertEqual(code.count("{"), code.count("}"))

    def test_it_dims_then_locks_then_blanks_then_sleeps(self):
        steps = [self.pairs(b) for b in self.listeners]
        times = [int(s["timeout"]) for s in steps]
        self.assertEqual(times, sorted(times))
        self.assertEqual(len(set(times)), len(times))
        actions = [s["on-timeout"] for s in steps]
        for action, wanted in zip(
            actions, ("brightnessctl", "lock-session", 'action = "off"', "suspend")
        ):
            self.assertIn(wanted, action)
        for step in steps:
            if "brightnessctl" in step["on-timeout"] or "dpms" in step["on-timeout"]:
                self.assertIn("on-resume", step, "what is turned off comes back")

    def test_locking_never_starts_a_second_lock(self):
        self.assertIn('lock_cmd = pgrep -f "nordlys-shell lock" || nordlys-shell lock', self.text)
        self.assertIn("before_sleep_cmd = loginctl lock-session", self.text)


class VmRule(unittest.TestCase):
    def test_the_rule_names_the_port_the_agent_opens(self):
        rule = read("vm/70-nordlys-vm.rules")
        name = re.search(r'ATTR\{name\}=="([^"]+)"', rule).group(1)
        self.assertIn('TAG+="uaccess"', rule)
        self.assertIn(f'"/dev/virtio-ports/{name}"', read("vm/nordlys-vm-agent"))
        self.assertIn(f"name={name}", read("vm/nordlys-vm"))


class Pam(unittest.TestCase):
    def test_it_includes_login_and_may_unlock_the_keyring(self):
        rules = [
            line.split()
            for line in read("config/pam.d/nordlys").splitlines()
            if line.strip() and not line.startswith("#")
        ]
        self.assertEqual(rules, [
            ["auth", "include", "login"],
            ["-auth", "optional", "pam_gnome_keyring.so"],
        ])

    def test_the_lock_screen_uses_this_service(self):
        pam = read("crates/nordlys-shell/src/lock/pam.rs")
        self.assertIn('Path::new("/etc/pam.d/nordlys")', pam)


@unittest.skipIf(tomllib is None, "needs Python 3.11")
class ExampleConfig(unittest.TestCase):
    def setUp(self):
        self.config = tomllib.loads(read("config/nordlys.toml"))

    def test_it_parses_with_the_documented_values(self):
        self.assertIn(self.config["theme"]["mode"], ("dark", "light"))
        self.assertIn(self.config["stage"]["layout"], ("auto", "ultrawide", "rails", "peek"))
        self.assertTrue(0 < self.config["stage"]["split"] < 1)

    def test_groups_have_unique_keys_and_apps(self):
        groups = self.config["group"]
        keys = [g["key"] for g in groups]
        self.assertEqual(len(keys), len(set(keys)))
        for group in groups:
            self.assertTrue(group["apps"], group["name"])
            self.assertIn(group.get("open_in", "new-workspace"), ("new-workspace", "current"))


class Workflow(unittest.TestCase):
    def test_ci_checks_every_script(self):
        ci = read(".github/workflows/ci.yml")
        shebanged = [
            f"session/{p.name}" for p in (REPO / "session").iterdir()
            if p.is_file() and p.read_bytes().startswith(b"#!")
        ]
        scripts = ["scripts/*.sh", "vm/nordlys-vm", "vm/nordlys-vm-agent",
                   "vm/clipboard.py", "session/hyprland.lua", *shebanged]
        self.assertGreater(len(scripts), 7)
        for script in scripts:
            self.assertIn(script, ci)
        self.assertIn("sh tests/run.sh", ci)


if __name__ == "__main__":
    unittest.main()
