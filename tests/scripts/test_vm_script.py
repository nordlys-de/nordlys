"""vm/nordlys-vm with a fake QEMU, qemu-img and clipboard bridge.

No virtual machine is ever started: qemu-system-x86_64 on the test's PATH is
a small stand-in that records its arguments and opens the clipboard socket.
"""

import glob
import os
import subprocess
import sys
import unittest

from support import Sandbox

FIRMWARE = [
    pattern
    for name in ("OVMF_VARS", "OVMF_CODE")
    for pattern in (
        f"/usr/share/edk2/x64/{name}.4m.fd",
        f"/usr/share/edk2-ovmf/x64/{name}.4m.fd",
        f"/usr/share/OVMF/{name}_4M.fd",
        f"/usr/share/OVMF/{name}.fd",
    )
    if os.path.isfile(pattern)
]
CAN_BOOT = len(FIRMWARE) >= 2 and os.access("/dev/kvm", os.W_OK)

FAKE_QEMU = """\
import os, socket, sys, time
args = sys.argv[1:]
with open(os.environ["NORDLYS_TEST_LOG"], "a") as log:
    log.write("qemu " + " ".join(args) + "\\n")
chardev = args[args.index("-chardev") + 1]
path = dict(p.split("=", 1) for p in chardev.split(",")[1:])["path"]
if not os.environ.get("NORDLYS_TEST_NO_SOCKET"):
    server = socket.socket(socket.AF_UNIX)
    server.bind(path)
    server.listen()
open(args[args.index("-pidfile") + 1], "w").write(str(os.getpid()))
time.sleep(0.4)
sys.exit(int(os.environ.get("NORDLYS_TEST_STATUS", "0")))
"""


class Vm(Sandbox):
    REAL = (
        "sh", "bash", "dirname", "readlink", "cksum", "cut", "cat", "cp", "mkdir",
        "rm", "sed", "seq", "sleep", "ls", "head", "basename",
    )

    def setUp(self):
        super().setUp()
        self.repo = self.tree("vm")
        self.script = self.repo / "vm/nordlys-vm"
        self.state = self.tmp / "state"
        fake = self.bin / "qemu-system-x86_64"
        fake.write_text(f"#!{sys.executable} -I\n{FAKE_QEMU}")
        fake.chmod(0o755)
        self.fake("qemu-img", 'eval "last=\\${$(($# - 1))}"; : > "$last"')
        self.fake("python3", "sleep 0.2")
        self.iso = self.write("some.iso", "not really\n")

    def vm(self, *args, **extra):
        env = self.env(NORDLYS_VM_STATE=self.state, NORDLYS_TEST_LOG=self.log, **extra)
        return self.run_script(["bash", self.script, *args], env=env)

    def qemu_line(self):
        return next(c for c in self.calls() if c.startswith("qemu -name"))

    def live_process(self):
        """Something for the pidfile to point at."""
        process = subprocess.Popen(["sleep", "30"])
        self.addCleanup(process.wait)
        self.addCleanup(process.kill)
        self.write("state/qemu.pid", str(process.pid))
        return process

    def test_no_command_prints_the_usage_comment(self):
        for args in ((), ("fly",)):
            with self.subTest(args=args):
                result = self.vm(*args)
                self.assert_ran(result, 2)
                self.assertIn("vm/nordlys-vm run [ISO]", result.stdout)
                self.assertNotIn("set -euo", result.stdout)

    def test_a_missing_iso_is_refused(self):
        for command in ("run", "live"):
            with self.subTest(command=command):
                result = self.vm(command, self.tmp / "nope.iso")
                self.assert_ran(result, 1)
                self.assertIn("no such file", result.stderr)

    def test_live_needs_an_iso(self):
        result = self.vm("live")
        self.assert_ran(result, 1)
        self.assertIn("no ISO found", result.stderr)

    def test_it_needs_qemu(self):
        (self.bin / "qemu-system-x86_64").unlink()
        result = self.vm("run", self.iso)
        self.assert_ran(result, 1)
        self.assertIn("qemu-system-x86_64 not found", result.stderr)

    @unittest.skipIf(os.access("/dev/kvm", os.W_OK), "this machine has KVM")
    def test_it_needs_kvm(self):
        result = self.vm("run", self.iso)
        self.assert_ran(result, 1)
        self.assertIn("/dev/kvm isn't usable", result.stderr)

    def test_destroy_removes_the_disk_and_firmware_settings(self):
        disk = self.write("state/disk.qcow2", "")
        uefi = self.write("state/uefi-vars.fd", "")
        keep = self.write("state/notes", "")
        self.assert_ran(self.vm("destroy"))
        self.assertFalse(disk.exists() or uefi.exists())
        self.assertTrue(keep.exists())
        self.assert_ran(self.vm("destroy"))

    def test_destroy_refuses_while_running(self):
        disk = self.write("state/disk.qcow2", "")
        self.live_process()
        result = self.vm("destroy")
        self.assert_ran(result, 1)
        self.assertIn("shut the VM down first", result.stderr)
        self.assertTrue(disk.exists())

    def test_a_stale_pidfile_does_not_count_as_running(self):
        done = subprocess.Popen(["true"])
        done.wait()
        self.write("state/qemu.pid", str(done.pid))
        self.assert_ran(self.vm("destroy"))
        result = self.vm("clip")
        self.assert_ran(result, 1)
        self.assertIn("the VM isn't running", result.stderr)

    def test_clip_restarts_the_bridge(self):
        self.live_process()
        old = subprocess.Popen(["sleep", "30"])
        self.addCleanup(old.kill)
        self.write("state/clip.pid", str(old.pid))
        self.assert_ran(self.vm("clip"))
        self.assertEqual(old.wait(timeout=5), -15, "the old bridge is stopped")
        call = next(c for c in self.calls() if c.startswith("python3 "))
        self.assertIn(f"{(self.repo / 'vm/clipboard.py').resolve()} ", call)
        self.assertTrue(call.endswith(".sock"))
        self.assertNotEqual((self.state / "clip.pid").read_text().strip(), str(old.pid))

    def test_the_socket_lives_in_the_runtime_dir_and_follows_the_state(self):
        self.live_process()
        self.assert_ran(self.vm("clip"))
        first = self.calls()[-1].split()[-1]
        self.assertTrue(first.startswith(f"{self.tmp}/run/nordlys-vm-"), first)
        self.assertLess(len(first), 108)


@unittest.skipUnless(CAN_BOOT, "needs UEFI firmware files and a writable /dev/kvm")
class Boot(Vm):
    def test_a_new_vm_gets_a_disk_and_starts_the_installer(self):
        result = self.vm("run", self.iso)
        self.assert_ran(result)
        self.assertIn("New VM: the installer from some.iso will start", result.stdout)
        self.assertIn(f"qemu-img create -q -f qcow2 {self.state}/disk.qcow2 40G", self.calls())
        self.assertTrue((self.state / "uefi-vars.fd").is_file())
        line = self.qemu_line()
        for wanted in (
            "-enable-kvm", "-smp 4", "-m 6G", "virtio-vga-gl",
            "gtk,gl=on,zoom-to-fit=on", "hostfwd=tcp:127.0.0.1:2222-:22",
            "pa,id=snd", "name=org.nordlys.clipboard",
            "virtio-blk-pci,drive=hd,bootindex=1", "ide-cd,drive=cd,bootindex=2",
            f"file={self.iso}",
        ):
            self.assertIn(wanted, line)
        self.assertTrue(any("clipboard.py" in c for c in self.calls()))
        for leftover in ("qemu.pid", "clip.pid"):
            self.assertFalse((self.state / leftover).exists(), leftover)
        self.assertEqual(list((self.tmp / "run").iterdir()), [])

    def test_an_installed_vm_boots_without_an_iso(self):
        self.write("state/disk.qcow2", "")
        result = self.vm("run")
        self.assert_ran(result)
        self.assertNotIn("New VM", result.stdout)
        self.assertNotIn("ide-cd", self.qemu_line())
        self.assertFalse(any(c.startswith("qemu-img") for c in self.calls()))

    def test_a_new_vm_without_an_iso_says_how_to_pass_one(self):
        result = self.vm("run")
        self.assert_ran(result, 1)
        self.assertIn("no ISO: pass one", result.stderr)

    def test_the_newest_download_is_the_default_iso(self):
        old = self.write("home/Downloads/cachyos-old.iso", "")
        self.write("home/Downloads/cachyos-new.iso", "")
        os.utime(old, (1, 1))
        self.assert_ran(self.vm("run"))
        self.assertIn("cachyos-new.iso", self.qemu_line())

    def test_live_boots_the_iso_first(self):
        self.write("state/disk.qcow2", "")
        self.assert_ran(self.vm("live", self.iso))
        line = self.qemu_line()
        self.assertIn("virtio-blk-pci,drive=hd,bootindex=2", line)
        self.assertIn("ide-cd,drive=cd,bootindex=1", line)

    def test_the_environment_tunes_it(self):
        self.assert_ran(
            self.vm(
                "run", self.iso,
                NORDLYS_VM_MEM="2G", NORDLYS_VM_CPUS=2, NORDLYS_VM_DISK="9G",
                NORDLYS_VM_DISPLAY="none", NORDLYS_VM_SSH_PORT=2201,
                NORDLYS_VM_AUDIO="none", NORDLYS_VM_EXTRA="-snapshot -boot menu=on",
            )
        )
        line = self.qemu_line()
        for wanted in (
            "-m 2G", "-smp 2", "-display none", "127.0.0.1:2201-:22",
            "none,id=snd", "-snapshot -boot menu=on",
        ):
            self.assertIn(wanted, line)
        self.assertTrue(any(c.endswith(" 9G") for c in self.calls()))

    def test_a_qemu_failure_is_reported(self):
        result = self.vm("run", self.iso, NORDLYS_TEST_STATUS=7)
        self.assert_ran(result, 1)
        self.assertIn("QEMU exited with status 7", result.stderr)
        self.assertFalse((self.state / "qemu.pid").exists())

    def test_without_a_socket_there_is_no_bridge(self):
        self.assert_ran(self.vm("run", self.iso, NORDLYS_TEST_NO_SOCKET=1))
        self.assertFalse(any("clipboard.py" in c for c in self.calls()))

    def test_missing_firmware_says_what_to_install(self):
        # The throwaway copy looks for firmware where there is none.
        text = self.script.read_text()
        self.script.write_text(text.replace("/usr/share/", f"{self.tmp}/none/"))
        result = self.vm("run", self.iso)
        self.assert_ran(result, 1)
        self.assertIn("UEFI firmware not found; install edk2-ovmf", result.stderr)
        self.assertFalse(any(c.startswith("qemu -name") for c in self.calls()))

    def test_a_second_boot_is_refused(self):
        self.live_process()
        result = self.vm("run", self.iso)
        self.assert_ran(result, 1)
        self.assertIn("already running", result.stderr)


# Boot reuses Vm's helpers; don't run Vm's tests twice.
for _name in [n for n in vars(Vm) if n.startswith("test_")]:
    setattr(Boot, _name, None)
del _name


if __name__ == "__main__":
    unittest.main()
