"""vm/nordlys-vm-agent, loaded as a module. Its programs (wl-copy, wl-paste,
hyprctl) are replaced, and its endless loops are ended from the outside."""

import base64
import contextlib
import fcntl
import io
import os
import socket
import subprocess
import sys
import tempfile
import unittest
import warnings
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_loader
from pathlib import Path
from unittest import mock

from support import REPO

sys.dont_write_bytecode = True


class Stop(Exception):
    """Raised from a patched sleep to leave a loop."""


def load():
    loader = SourceFileLoader("nordlys_vm_agent", str(REPO / "vm/nordlys-vm-agent"))
    module = module_from_spec(spec_from_loader(loader.name, loader))
    loader.exec_module(module)
    return module


def b64(data):
    return base64.b64encode(data)


class Agent(unittest.TestCase):
    def setUp(self):
        # main() keeps its lock file open for as long as the agent lives.
        warnings.filterwarnings("ignore", "unclosed file .*nordlys-vm-agent.lock", ResourceWarning)
        self.agent = load()
        tmp = tempfile.TemporaryDirectory(prefix="nla-")
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)

    def patch(self, target, name, **kwargs):
        patcher = mock.patch.object(target, name, **kwargs)
        self.addCleanup(patcher.stop)
        return patcher.start()

    def pair(self):
        ours, theirs = socket.socketpair()
        self.addCleanup(ours.close)
        self.addCleanup(theirs.close)
        ours.setblocking(False)
        theirs.settimeout(5)
        return ours, theirs

    # ---- sending ----

    def test_a_message_is_one_line(self):
        ours, theirs = self.pair()
        self.agent.send(ours.fileno(), b"abc")
        self.assertEqual(theirs.recv(100), b"abc\n")

    def test_partial_and_blocked_writes_are_finished(self):
        written = []
        outcomes = iter([2, BlockingIOError, 1, 2])

        def write(_fd, data):
            outcome = next(outcomes)
            if outcome is BlockingIOError:
                raise BlockingIOError
            written.append(data[:outcome])
            return outcome

        self.patch(self.agent.os, "write", side_effect=write)
        waited = self.patch(self.agent.select, "select", return_value=([], [7], []))
        self.agent.send(7, b"abcd")
        self.assertEqual(b"".join(written), b"abcd\n")
        waited.assert_called_once()

    def test_a_host_that_is_not_listening_is_ignored(self):
        self.patch(self.agent.os, "write", side_effect=OSError(5, "no reader"))
        self.agent.send(7, b"abc")

    # ---- host to guest ----

    def serve(self, *chunks):
        """Feed the port, then close it; returns (clipboards set, replies)."""
        ours, theirs = self.pair()
        copied = []
        self.patch(self.agent, "clip_set", side_effect=copied.append)
        self.patch(self.agent.time, "sleep", side_effect=Stop)
        for chunk in chunks:
            theirs.sendall(chunk)
        theirs.shutdown(socket.SHUT_WR)
        with self.assertRaises(Stop):
            self.agent.serve_port(ours.fileno())
        theirs.setblocking(False)
        try:
            replies = theirs.recv(1000)
        except BlockingIOError:
            replies = b""
        return copied, replies

    def test_lines_from_the_host_become_the_clipboard(self):
        copied, replies = self.serve(b64(b"one") + b"\n" + b64("två\n2".encode()) + b"\n")
        self.assertEqual(copied, [b"one", "två\n2".encode()])
        self.assertEqual(replies, b"")
        self.assertEqual(self.agent.last, "två\n2".encode())

    def test_a_ping_is_answered(self):
        copied, replies = self.serve(b"!ping\n")
        self.assertEqual((copied, replies), ([], b"!hello\n"))

    def test_junk_and_empty_lines_are_skipped(self):
        copied, _ = self.serve(b"not base64!\n\nabc\n" + b64(b"ok") + b"\n")
        self.assertEqual(copied, [b"ok"])

    def test_an_endless_line_is_thrown_away(self):
        self.agent.LIMIT = 4
        copied, _ = self.serve(b"A" * 20)
        self.assertEqual(copied, [])

    def test_a_blocked_or_broken_read_carries_on(self):
        reads = iter([BlockingIOError, OSError(5, "gone"), b64(b"hi") + b"\n", b""])

        def read(_fd, _size):
            outcome = next(reads)
            if isinstance(outcome, bytes):
                return outcome
            raise outcome

        self.patch(self.agent.os, "read", side_effect=read)
        self.patch(self.agent.select, "select", return_value=([], [], []))
        self.patch(self.agent.time, "sleep", side_effect=[None, Stop])
        copy = self.patch(self.agent, "clip_set")
        with self.assertRaises(Stop):
            self.agent.serve_port(7)
        copy.assert_called_once_with(b"hi")

    def test_wl_copy_gets_no_pipes_to_hold_open(self):
        run = self.patch(self.agent.subprocess, "run")
        self.agent.clip_set(b"text")
        run.assert_called_once_with(
            ["wl-copy"], input=b"text",
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )

    # ---- guest to host ----

    def test_only_new_clipboards_are_sent(self):
        self.agent.LIMIT = 10
        self.agent.last = b"from-host"
        copies = [b"from-host", b"mine", b"mine", b"", b"x" * 11, b"other"]
        process = mock.Mock(stdout=[b64(c) + b"\n" for c in copies])
        popen = self.patch(self.agent.subprocess, "Popen", return_value=process)
        self.patch(self.agent.time, "sleep", side_effect=Stop)
        sent = self.patch(self.agent, "send")
        with self.assertRaises(Stop):
            self.agent.watch_clipboard(7)
        self.assertEqual(
            [c.args for c in sent.call_args_list],
            [(7, b64(b"mine")), (7, b64(b"other"))],
        )
        self.assertEqual(popen.call_args.args[0][:4], ["wl-paste", "-t", "text", "--watch"])
        process.wait.assert_called_once()

    # ---- following the window ----

    def output(self, name, modes=None):
        directory = self.tmp / name
        directory.mkdir(exist_ok=True)
        if modes is not None:
            (directory / "modes").write_text(modes)

    def test_only_virtual_outputs_with_a_mode_count(self):
        self.agent.DRM = str(self.tmp)
        self.output("card0-Virtual-1", "1280x800\n1024x768\n")
        self.output("card1-Virtual-2", "")
        self.output("card2-Virtual-3")
        self.output("card0-HDMI-A-1", "1920x1080\n")
        self.assertEqual(self.agent.preferred_modes(), {"Virtual-1": "1280x800"})

    def follow(self, modes, results):
        """Run the resize loop over a series of window sizes."""
        sizes = iter(modes)
        self.patch(self.agent, "preferred_modes", side_effect=lambda: next(sizes))
        self.patch(
            self.agent.time, "sleep",
            side_effect=[None] * (len(modes) - 1) + [Stop],
        )
        answers = [subprocess.CompletedProcess([], code, out, err) for code, out, err in results]
        run = self.patch(self.agent.subprocess, "run", side_effect=answers)
        errors = io.StringIO()
        with contextlib.redirect_stderr(errors), self.assertRaises(Stop):
            self.agent.follow_window()
        return [c.args[0] for c in run.call_args_list], errors.getvalue()

    def test_each_new_size_is_applied_once(self):
        small, large = {"Virtual-1": "800x600"}, {"Virtual-1": "1920x1080"}
        commands, errors = self.follow(
            [small, small, large, large], [(0, "ok\n", ""), (0, "", "")]
        )
        self.assertEqual(len(commands), 2)
        self.assertEqual(commands[0][:2], ["hyprctl", "eval"])
        self.assertEqual(
            commands[1][2],
            'hl.monitor({ output = "Virtual-1", mode = "1920x1080@60", '
            'position = "auto", scale = 1 })',
        )
        self.assertEqual(errors, "")

    def test_a_failed_resize_is_reported_once(self):
        mode = {"Virtual-1": "800x600"}
        for result in [(1, "", "no hyprland"), (0, "error: bad mode", "")]:
            with self.subTest(result=result):
                commands, errors = self.follow([mode, mode, mode], [result])
                self.assertEqual(len(commands), 1)
                self.assertEqual(errors.count("resize to 800x600 failed"), 1)

    # ---- start-up ----

    def start(self, port, hyprland=False):
        env = {"XDG_RUNTIME_DIR": str(self.tmp)}
        if hyprland:
            env["HYPRLAND_INSTANCE_SIGNATURE"] = "sig"
        patcher = mock.patch.dict(os.environ, env, clear=True)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.agent.PORT = str(port)
        return self.patch(self.agent.threading, "Thread")

    def test_only_one_agent_runs_per_session(self):
        self.start(self.tmp / "port")
        with open(self.tmp / "nordlys-vm-agent.lock", "w") as held:
            fcntl.flock(held, fcntl.LOCK_EX)
            with self.assertRaises(SystemExit) as refused:
                self.agent.main()
        self.assertEqual(str(refused.exception), "nordlys-vm-agent: already running")

    def test_it_greets_the_host_and_serves_the_port(self):
        port = self.tmp / "port"
        port.write_bytes(b"")
        threads = self.start(port, hyprland=True)
        serve = self.patch(self.agent, "serve_port")
        sent = self.patch(self.agent, "send")
        self.agent.main()
        fd = serve.call_args.args[0]
        self.addCleanup(os.close, fd)
        sent.assert_called_once_with(fd, b"!hello")
        targets = [c.kwargs["target"] for c in threads.call_args_list]
        self.assertEqual(targets, [self.agent.follow_window, self.agent.watch_clipboard])
        self.assertTrue(all(c.kwargs["daemon"] for c in threads.call_args_list))

    def test_without_a_port_it_still_resizes_and_waits(self):
        threads = self.start(self.tmp / "no-such-port", hyprland=True)
        self.patch(self.agent.time, "sleep", side_effect=Stop)
        errors = io.StringIO()
        with contextlib.redirect_stderr(errors), self.assertRaises(Stop):
            self.agent.main()
        self.assertIn("no clipboard port", errors.getvalue())
        self.assertEqual(threads.call_count, 1)

    def test_outside_hyprland_nothing_resizes(self):
        threads = self.start(self.tmp / "no-such-port")
        self.patch(self.agent.time, "sleep", side_effect=Stop)
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(Stop):
            self.agent.main()
        threads.assert_not_called()


if __name__ == "__main__":
    unittest.main()
