#!/bin/sh
# Tests for everything that isn't Rust: the shell and Python scripts, the
# Hyprland config and the files that tie the session together. The Rust
# tests are `cargo test --workspace`.
#
#   sh tests/run.sh [unittest options, e.g. -v or -k install]
#
# Needs python3. Tests that need lua, Pillow or UEFI firmware are skipped
# without them. Nothing on this machine is changed: the scripts run from
# throwaway copies with fake sudo, pacman, udevadm, systemctl and QEMU.
set -eu
cd "$(dirname "$0")/scripts"
exec python3 -B -m unittest discover -s . -p 'test_*.py' "$@"
