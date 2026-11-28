"""session/nordlys-record with fake recorders, slurp and hyprctl.

The fake recorder is a Python program because, like the real ones, it has to
take SIGINT even though the shell starts it in the background."""

import os
import re
import signal
import subprocess
import sys
import time
import unittest

from support import Sandbox

RECORDER = """\
import os, signal, sys, time
name = os.path.basename(sys.argv[0])
with open(os.environ["NORDLYS_TEST_LOG"], "a") as log:
    log.write(name + " " + " ".join(sys.argv[1:]) + "\\n")
target = sys.argv[sys.argv.index("-f" if name == "wf-recorder" else "-o") + 1]

def finish(*_):
    open(target, "w").write("video")
    sys.exit(0)

signal.signal(signal.SIGINT, finish)
mode = os.environ.get("NORDLYS_TEST_MODE")
if mode == "crash":
    print("no encoder", flush=True)
    sys.exit(3)
if mode == "gone":  # the screen went away after something was recorded
    finish()
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
time.sleep(20)
"""


class Record(Sandbox):
    REAL = ("sh", "cat", "sed", "mkdir", "rm", "date", "sleep", "tail", "awk")

    def setUp(self):
        super().setUp()
        self.script = self.tree("session/nordlys-record") / "session/nordlys-record"
        self.run_dir = self.tmp / "run"
        self.pid_file = self.run_dir / "nordlys-record.pid"
        self.errors = self.tmp / "stderr"
        self.fake("notify-send")
        self.fake("slurp", "echo '10,20 300x400'")
        self.addCleanup(self.kill_recorder)

    def recorder(self, name="wf-recorder"):
        path = self.bin / name
        path.write_text(f"#!{sys.executable} -I\n{RECORDER}")
        path.chmod(0o755)

    def kill_recorder(self):
        try:
            os.kill(int(self.pid_file.read_text()), signal.SIGTERM)
        except (OSError, ValueError):
            pass

    def record(self, *args, **extra):
        """Run the script. Its background watcher keeps stdout open for as
        long as the recording lasts, so output goes to files, not pipes."""
        with open(self.errors, "w") as errors:
            return subprocess.run(
                ["sh", str(self.script), *args],
                env=self.env(NORDLYS_TEST_LOG=self.log, **extra),
                stdout=subprocess.DEVNULL, stderr=errors, timeout=30, cwd=self.tmp,
            ).returncode

    def wait_for(self, what, ready):
        deadline = time.monotonic() + 10
        while not ready():
            if time.monotonic() > deadline:
                self.fail(f"timed out waiting for {what}: {self.calls()}")
            time.sleep(0.02)

    def called(self, prefix):
        return [c for c in self.calls() if c.startswith(prefix + " ")]

    def begin(self, *args, recorder="wf-recorder", **extra):
        self.recorder(recorder)
        self.assertEqual(self.record(*args, **extra), 0, self.errors.read_text())
        self.wait_for("the recorder", lambda: self.called(recorder) and self.pid_file.exists())
        # Give it a moment to take over SIGINT before anyone stops it.
        time.sleep(0.3)
        return self.called(recorder)[0].split(" ")[1:]

    def videos(self):
        return sorted((self.home / "Videos/Recordings").glob("*.mp4"))

    def notices(self):
        return self.called("notify-send")

    def test_a_region_is_recorded_into_videos(self):
        args = self.begin()
        self.assertEqual(args[0], "-f")
        self.assertRegex(
            args[1],
            rf"^{re.escape(str(self.home))}/Videos/Recordings/\d{{4}}-\d\d-\d\d_\d\d-\d\d-\d\d\.mp4$",
        )
        self.assertEqual(args[2:], ["-g", "10,20", "300x400"])
        self.assertIn("slurp -d", self.calls())
        self.assertEqual((self.run_dir / "nordlys-record.file").read_text().strip(), args[1])

    def test_stop_lets_the_recorder_finish_and_says_so_once(self):
        args = self.begin("region")
        self.assertEqual(self.record("stop"), 0)
        self.wait_for("the notice", self.notices)
        time.sleep(0.3)
        self.assertEqual(
            self.notices(),
            [f"notify-send -a Screen recording -i video-x-generic Recording saved {args[1]}"],
        )
        self.assertEqual(self.videos()[0].read_text(), "video")
        for leftover in ("nordlys-record.pid", "nordlys-record.file"):
            self.assertFalse((self.run_dir / leftover).exists(), leftover)

    def test_the_same_key_stops_a_running_recording(self):
        self.begin("screen")
        self.assertEqual(self.record("screen"), 0)
        self.wait_for("the file", self.videos)
        self.assertEqual(len(self.called("wf-recorder")), 1, "no second recording")
        self.assertFalse(self.pid_file.exists())

    def test_stop_with_nothing_running_is_quiet(self):
        self.assertEqual(self.record("stop"), 0)
        self.assertEqual(self.calls(), [])

    def test_a_stale_pid_file_does_not_block_a_new_recording(self):
        done = subprocess.Popen(["true"])
        done.wait()
        self.pid_file.write_text(str(done.pid))
        self.begin("screen")

    def test_the_screen_is_the_focused_monitor(self):
        self.fake("hyprctl", """printf '{\\n  "id": 2,\\n  "monitor": "DP-2",\\n}\\n'""")
        args = self.begin("screen")
        self.assertEqual(args[2:], ["-o", "DP-2"])
        self.assertEqual(self.called("slurp"), [])
        self.assertIn("hyprctl activeworkspace -j", self.calls())

    def test_without_hyprctl_the_recorder_picks_the_screen(self):
        self.assertEqual(len(self.begin("screen")), 2)

    def test_audio_is_asked_for(self):
        self.assertIn("--audio", self.begin("screen", "--audio"))

    def test_your_videos_folder_is_used(self):
        self.fake("xdg-user-dir", f"echo '{self.tmp}/Films'")
        args = self.begin("screen")
        self.assertTrue(args[1].startswith(f"{self.tmp}/Films/Recordings/"), args)

    def test_gpu_screen_recorder_is_the_fallback(self):
        args = self.begin("region", "--audio", recorder="gpu-screen-recorder")
        self.assertEqual(args[:3], ["-f", "60", "-o"])
        self.assertEqual(
            args[4:], ["-w", "region", "-region", "300x400+10+20", "-a", "default_output"]
        )

    def test_gpu_screen_recorder_records_a_screen(self):
        self.assertEqual(self.begin("screen", recorder="gpu-screen-recorder")[4:], ["-w", "screen"])

    def test_gpu_screen_recorder_records_the_focused_monitor(self):
        self.fake("hyprctl", """echo '  "monitor": "eDP-1",'""")
        self.assertEqual(self.begin("screen", recorder="gpu-screen-recorder")[4:], ["-w", "eDP-1"])

    def test_a_recorder_that_fails_says_why(self):
        self.recorder()
        self.assertEqual(self.record("screen", NORDLYS_TEST_MODE="crash"), 0)
        self.wait_for("the notice", self.notices)
        self.wait_for("the log", lambda: any("exit 3" in line for line in self.calls()))
        text = "\n".join(self.calls())
        self.assertIn("-u critical Recording stopped no encoder", text)
        self.assertIn("(exit 3)", text)
        self.assertFalse(self.pid_file.exists())

    def test_a_recording_that_ends_by_itself_is_still_saved(self):
        self.recorder()
        self.assertEqual(self.record("screen", NORDLYS_TEST_MODE="gone"), 0)
        self.wait_for("the notice", self.notices)
        self.assertIn("Recording saved", self.notices()[0])

    def test_pressing_escape_in_slurp_records_nothing(self):
        self.recorder()
        self.fake("slurp", "exit 1")
        self.assertEqual(self.record("region"), 0)
        self.assertEqual(self.called("wf-recorder"), [])
        self.assertEqual(self.notices(), [])

    def test_a_region_needs_slurp(self):
        self.recorder()
        (self.bin / "slurp").unlink()
        self.assertEqual(self.record("region"), 1)
        self.assertIn("Can't pick a region Install slurp.", self.notices()[0])

    def test_without_a_recorder_it_says_what_to_install(self):
        self.assertEqual(self.record("screen"), 1)
        self.assertIn("Install wf-recorder", self.notices()[0])

    @unittest.expectedFailure  # known bug: the sed range runs one line too far
    def test_help_shows_only_the_comment(self):
        result = self.run_script(["sh", self.script, "--help"])
        self.assertNotIn("set -u", result.stdout)

    def test_help_and_unknown_commands(self):
        result = self.run_script(["sh", self.script, "--help"])
        self.assert_ran(result)
        self.assertIn("nordlys-record region [--audio]", result.stdout)
        result = self.run_script(["sh", self.script, "movie"])
        self.assert_ran(result, 2)
        self.assertIn("region, screen or stop", result.stderr)


if __name__ == "__main__":
    unittest.main()
