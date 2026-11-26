#!/bin/sh
# Set up Nordlys on a fresh Arch or CachyOS install: dependencies, build,
# install, and the login entry. Run it as your user from a clone:
#
#   git clone https://github.com/nordlys-de/nordlys && nordlys/scripts/setup.sh
#
# In a QEMU/KVM virtual machine it also installs the guest agent (shared
# clipboard and window-size resizing). Force with --vm-guest or --no-vm-guest.
# Safe to run again after a `git pull`.
set -eu

vm_guest=auto
for arg in "$@"; do
    case $arg in
        --vm-guest) vm_guest=yes ;;
        --no-vm-guest) vm_guest=no ;;
        -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "setup.sh: unknown option $arg" >&2; exit 2 ;;
    esac
done

say() { printf '\033[1;34m::\033[0m %s\n' "$*"; }
die() { echo "setup.sh: $*" >&2; exit 1; }

[ "$(id -u)" -ne 0 ] || die "run it as your user, not root (it asks for sudo itself)"
command -v pacman >/dev/null || die "only Arch-based systems for now; see README.md for other distros"
root=$(cd "$(dirname "$0")/.." && pwd)

if [ "$vm_guest" = auto ]; then
    case $(systemd-detect-virt 2>/dev/null || true) in
        kvm|qemu) vm_guest=yes ;;
        *) vm_guest=no ;;
    esac
fi

packages="base-devel git pkgconf pam libxkbcommon wayland mesa
    hyprland hypridle swaybg hyprpolkitagent xdg-desktop-portal-hyprland
    inter-font ttf-jetbrains-mono noto-fonts pipewire pipewire-pulse wireplumber
    playerctl networkmanager bluez-utils brightnessctl grim slurp wl-clipboard
    foot libnotify qt6ct xdg-utils
    xdg-desktop-portal-gtk xdg-user-dirs gsettings-desktop-schemas dconf
    adw-gtk-theme adwaita-icon-theme noto-fonts-emoji
    nautilus gvfs ffmpegthumbnailer imv mpv mpv-mpris
    zathura zathura-pdf-mupdf gnome-text-editor helix btop
    pavucontrol file-roller"
# A rustup or distro toolchain that's already here is fine.
command -v cargo >/dev/null || packages="$packages rust"
[ "$vm_guest" = yes ] && packages="$packages python"

say "Installing dependencies"
# -Syu: installing against a stale package list would half-upgrade the system.
# shellcheck disable=SC2086
sudo pacman -Syu --needed $packages

say "Building Nordlys (a few minutes the first time)"
(cd "$root" && cargo build --release --locked -p nordlys-shell -p nordlysctl -p nordlys-theme)

say "Installing"
if [ "$vm_guest" = yes ]; then
    sudo "$root/scripts/install.sh" --vm-guest
else
    sudo "$root/scripts/install.sh"
fi

if [ ! -e /etc/systemd/system/display-manager.service ]; then
    say "No display manager is enabled; installing SDDM"
    sudo pacman -S --needed sddm
    sudo systemctl enable sddm
fi

say "Done."
echo "1. Check the lock screen accepts your password:  nordlys-shell lock --check"
echo "2. Reboot (or log out), pick the \"Nordlys\" session on the login screen, and log in."
if [ "$vm_guest" = yes ]; then
    echo "   The shared clipboard and window resizing work inside the Nordlys session."
fi
