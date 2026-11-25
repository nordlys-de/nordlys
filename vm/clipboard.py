#!/usr/bin/env python3
"""The host end of the VM's shared clipboard (nordlys-vm starts it).

Talks to vm/nordlys-vm-agent inside the VM over a virtio-serial port that
QEMU exposes here as a Unix socket. Each message is one line: base64 of
the clipboard text, "!ping" from here, or "!hello" from the agent (when it
starts and in answer to a ping). Text only.
Uses xclip (X11, or Wayland through Xwayland), else wl-clipboard on
compositors with data-control.

    clipboard.py SOCKET
"""
import base64, os, select, shutil, socket, subprocess, sys, time

LIMIT = 1 << 20  # ignore clipboards over 1 MiB

# xclip first, also on Wayland (through Xwayland): without the data-control
# protocol, polling wl-paste opens a window each time and steals focus.
if os.environ.get("DISPLAY") and shutil.which("xclip"):
    GET = ["xclip", "-o", "-selection", "clipboard", "-t", "UTF8_STRING"]
    SET = ["xclip", "-i", "-selection", "clipboard"]
elif shutil.which("wl-paste") and subprocess.run(
        ["timeout", "1", "wl-paste", "--watch", "true"], capture_output=True).returncode == 124:
    GET, SET = ["wl-paste", "-n", "-t", "text"], ["wl-copy"]
else:
    sys.exit("clipboard.py: install xclip (needs X or Xwayland); no shared clipboard")


def host_get():
    try:
        r = subprocess.run(GET, capture_output=True, timeout=2)
    except subprocess.TimeoutExpired:
        return None
    return r.stdout if r.returncode == 0 else None


def host_set(data):
    # Both tools fork to keep serving the clipboard: give them no pipes.
    subprocess.run(SET, input=data, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def serve(sock):
    last = host_get()  # what's here already isn't "new" until the agent asks
    ready = False  # nothing is sent until the agent has said hello
    buf = b""
    sock.sendall(b"!ping\n")  # an agent that's already up answers !hello
    while True:
        if select.select([sock], [], [], 0.4)[0]:
            chunk = sock.recv(65536)
            if not chunk:
                return
            buf += chunk
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                if line == b"!hello":
                    ready, last = True, None
                    continue
                try:
                    data = base64.b64decode(line, validate=True)
                except ValueError:
                    continue
                ready = True
                if data and len(data) <= LIMIT and data != last:
                    last = data
                    host_set(data)
            if len(buf) > 2 * LIMIT:
                buf = b""
        data = host_get()
        if ready and data and len(data) <= LIMIT and data != last:
            last = data
            sock.sendall(base64.b64encode(data) + b"\n")


def main():
    path = sys.argv[1]
    while os.path.exists(path):
        try:
            with socket.socket(socket.AF_UNIX) as s:
                s.connect(path)
                s.settimeout(5)
                serve(s)
        except OSError:
            pass
        time.sleep(1)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
