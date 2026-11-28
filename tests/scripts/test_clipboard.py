"""vm/clipboard.py: unit tests, then both ends of the shared clipboard as
real processes with fake clipboard tools and a pseudo-terminal for the
virtio port."""

import base64
import contextlib
import os
import pty
import select
import signal
import socket
import subprocess
import sys
import threading
import time
import tty
import unittest
from importlib.util import module_from_spec, spec_from_file_location
from unittest import mock

from support import REPO, Sandbox

sys.dont_write_bytecode = True


class Stop(Exception):
    pass


def load(display=None, tools=(), watch_status=124):
    """Import clipboard.py as it would be on a host with `tools`."""
    env = {"DISPLAY": display} if display else {}
    probe = subprocess.CompletedProcess([], watch_status, b"", b"")
    spec = spec_from_file_location("clipboard_under_test", REPO / "vm/clipboard.py")
    module = module_from_spec(spec)
    with (
        mock.patch.dict(os.environ, env, clear=True),
        mock.patch("shutil.which", side_effect=lambda t: f"/x/{t}" if t in tools else None),
        mock.patch("subprocess.run", return_value=probe),
    ):
        spec.loader.exec_module(module)
    return module


def b64(data):
    return base64.b64encode(data) + b"\n"


class Tools(unittest.TestCase):
    def test_xclip_comes_first_when_there_is_a_display(self):
        module = load(":0", ("xclip", "wl-paste"))
        self.assertEqual((module.GET[0], module.SET[0]), ("xclip", "xclip"))

    def test_wl_clipboard_when_it_can_watch(self):
        for display, tools in ((None, ("xclip", "wl-paste")), (":0", ("wl-paste",))):
            with self.subTest(display=display, tools=tools):
                module = load(display, tools)
                self.assertEqual((module.GET[0], module.SET), ("wl-paste", ["wl-copy"]))

    def test_otherwise_it_says_what_to_install(self):
        for tools, status in (((), 124), (("wl-paste",), 1), (("xclip",), 124)):
            with self.subTest(tools=tools, status=status):
                with self.assertRaises(SystemExit) as refused:
                    load(None, tools, status)
                self.assertIn("install xclip", str(refused.exception))


class Bridge(unittest.TestCase):
    def setUp(self):
        self.clip = load(":0", ("xclip",))

    def patch(self, target, name, **kwargs):
        patcher = mock.patch.object(target, name, **kwargs)
        self.addCleanup(patcher.stop)
        return patcher.start()

    def test_reading_the_host_clipboard(self):
        done = lambda code: subprocess.CompletedProcess([], code, b"text", b"")
        run = self.patch(self.clip.subprocess, "run", return_value=done(0))
        self.assertEqual(self.clip.host_get(), b"text")
        run.return_value = done(1)
        self.assertIsNone(self.clip.host_get())
        run.side_effect = subprocess.TimeoutExpired("xclip", 2)
        self.assertIsNone(self.clip.host_get())

    def test_writing_the_host_clipboard(self):
        run = self.patch(self.clip.subprocess, "run")
        self.clip.host_set(b"text")
        self.assertEqual(run.call_args.args[0], self.clip.SET)
        self.assertEqual(run.call_args.kwargs["input"], b"text")

    def start(self, held):
        """serve() on a thread. `held[0]` is the host's clipboard."""
        ours, theirs = socket.socketpair()
        theirs.settimeout(5)
        self.addCleanup(theirs.close)
        copied = []
        self.patch(self.clip, "host_get", side_effect=lambda: held[0])

        def host_set(data):
            copied.append(data)
            held[0] = data

        self.patch(self.clip, "host_set", side_effect=host_set)
        thread = threading.Thread(target=self.clip.serve, args=(ours,), daemon=True)
        thread.start()

        def finish():
            theirs.shutdown(socket.SHUT_WR)
            thread.join(5)
            self.assertFalse(thread.is_alive(), "serve returns when the VM goes")
            ours.close()

        self.addCleanup(finish)
        self.assertEqual(self.line(theirs), b"!ping\n")
        return theirs, copied

    def line(self, sock):
        got = b""
        while not got.endswith(b"\n"):
            got += sock.recv(1)
        return got

    def quiet(self, sock, seconds=0.9):
        return not select.select([sock], [], [], seconds)[0]

    def until(self, check):
        deadline = time.monotonic() + 5
        while not check():
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.02)

    def test_nothing_is_sent_before_the_agent_says_hello(self):
        held = [b"old"]
        vm, _ = self.start(held)
        held[0] = b"new"
        self.assertTrue(self.quiet(vm))
        vm.sendall(b"!hello\n")
        self.assertEqual(self.line(vm), b64(b"new"))

    def test_a_hello_gets_the_current_clipboard_then_only_changes(self):
        held = [b"first"]
        vm, _ = self.start(held)
        vm.sendall(b"!hello\n")
        self.assertEqual(self.line(vm), b64(b"first"))
        self.assertTrue(self.quiet(vm))
        held[0] = b"second"
        self.assertEqual(self.line(vm), b64(b"second"))

    def test_the_guests_clipboard_is_copied_and_not_echoed(self):
        held = [None]
        vm, copied = self.start(held)
        vm.sendall(b"junk!\n" + b"\n" + b64(b"guest") + b64(b"guest"))
        self.until(lambda: copied)
        self.assertTrue(self.quiet(vm))
        self.assertEqual(copied, [b"guest"])

    def test_oversized_clipboards_stay_where_they_are(self):
        self.clip.LIMIT = 8
        held = [None]
        vm, copied = self.start(held)
        vm.sendall(b"!hello\n" + b64(b"x" * 9))
        vm.sendall(b"A" * 40)
        held[0] = b"y" * 9
        self.assertTrue(self.quiet(vm))
        self.assertEqual(copied, [])

    def test_it_reconnects_while_the_socket_exists(self):
        self.patch(self.clip.sys, "argv", new=["clipboard.py", "/some/sock"])
        self.patch(self.clip.os.path, "exists", side_effect=[True, True, False])
        sleep = self.patch(self.clip.time, "sleep")
        connection = mock.MagicMock()
        connection.__enter__.return_value = connection
        connection.connect.side_effect = [ConnectionRefusedError, None]
        self.patch(self.clip.socket, "socket", return_value=connection)
        serve = self.patch(self.clip, "serve")
        self.clip.main()
        serve.assert_called_once_with(connection)
        self.assertEqual(sleep.call_count, 2)
        connection.connect.assert_called_with("/some/sock")


FAKE_XCLIP = """
case $1 in
    -o) [ -f "$NL_HOST" ] && cat "$NL_HOST" ;;
    -i) echo set >> "$NL_HOST.writes"; cat > "$NL_HOST.new" && mv "$NL_HOST.new" "$NL_HOST" ;;
esac
"""
FAKE_WL_COPY = 'cat > "$NL_GUEST.new" && mv "$NL_GUEST.new" "$NL_GUEST"'
# wl-paste -t text --watch CMD…: run CMD with the clipboard whenever it changes.
FAKE_WL_PASTE = """
shift 3
previous=
while :; do
    if [ -f "$NL_GUEST" ]; then
        current=$(cat "$NL_GUEST")
        if [ "$current" != "$previous" ]; then
            previous=$current
            printf %s "$current" | "$@"
        fi
    fi
    sleep 0.05
done
"""


class BothEnds(Sandbox):
    """The agent and the host bridge, talking to each other."""

    REAL = ("sh", "cat", "mv", "sleep", "base64")

    def setUp(self):
        super().setUp()
        self.fake("xclip", FAKE_XCLIP)
        self.fake("wl-copy", FAKE_WL_COPY)
        self.fake("wl-paste", FAKE_WL_PASTE)
        self.host = self.tmp / "host-clipboard"
        self.guest = self.tmp / "guest-clipboard"

        # The virtio port: the agent opens one end of a terminal; the other
        # is relayed to the Unix socket QEMU would offer on the host.
        master, slave = pty.openpty()
        tty.setraw(slave)
        self.addCleanup(os.close, slave)
        self.addCleanup(os.close, master)
        path = self.tmp / "clip.sock"
        server = socket.socket(socket.AF_UNIX)
        server.bind(str(path))
        server.listen()
        self.addCleanup(server.close)

        self.agent = self.process(
            [sys.executable, REPO / "vm/nordlys-vm-agent"],
            NORDLYS_VM_PORT=os.ttyname(slave),
        )
        self.bridge = self.process(
            [sys.executable, REPO / "vm/clipboard.py", path], DISPLAY=":nowhere"
        )
        server.settimeout(10)
        connection, _ = server.accept()
        self.addCleanup(connection.close)
        stop = threading.Event()

        def relay():
            while not stop.is_set():
                for ready in select.select([connection, master], [], [], 0.1)[0]:
                    if ready is connection:
                        data = connection.recv(65536)
                        if not data:
                            return
                        os.write(master, data)
                    else:
                        connection.sendall(os.read(master, 65536))

        def safely():
            with contextlib.suppress(OSError):
                relay()

        thread = threading.Thread(target=safely, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 5)
        self.addCleanup(stop.set)

    def process(self, argv, **extra):
        env = self.env(NL_HOST=self.tmp / "host-clipboard",
                       NL_GUEST=self.tmp / "guest-clipboard", **extra)
        process = subprocess.Popen(
            [str(a) for a in argv], env=env, cwd=self.tmp,
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
            start_new_session=True,
        )

        def stop():
            # The group: wl-paste --watch runs for as long as it's allowed.
            with contextlib.suppress(ProcessLookupError):
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(5)
            process.stderr.close()

        self.addCleanup(stop)
        return process

    def arrives(self, path, data):
        deadline = time.monotonic() + 15
        while not (path.exists() and path.read_bytes() == data):
            for process in (self.agent, self.bridge):
                if process.poll() is not None:
                    self.fail(process.stderr.read().decode())
            self.assertLess(time.monotonic(), deadline, f"{data!r} never reached {path.name}")
            time.sleep(0.05)

    def test_text_travels_both_ways_without_echoing(self):
        self.host.write_bytes("from the host: blåbær ✓".encode())
        self.arrives(self.guest, "from the host: blåbær ✓".encode())
        # The guest's watcher sees that copy too; it must not bounce back.
        time.sleep(1)
        self.assertFalse((self.tmp / "host-clipboard.writes").exists())

        self.guest.write_bytes(b"from the guest\nsecond line")
        self.arrives(self.host, b"from the guest\nsecond line")
        time.sleep(1)
        self.assertEqual(
            (self.tmp / "host-clipboard.writes").read_text().splitlines(), ["set"]
        )
        self.assertEqual(self.guest.read_bytes(), b"from the guest\nsecond line")

    def test_both_ends_stop_quietly_on_ctrl_c(self):
        self.host.write_bytes(b"ready")
        self.arrives(self.guest, b"ready")
        for process in (self.agent, self.bridge):
            process.send_signal(signal.SIGINT)
            self.assertEqual(process.wait(10), 0, process.stderr.read().decode())


if __name__ == "__main__":
    unittest.main()
