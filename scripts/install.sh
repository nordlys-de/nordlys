#!/bin/sh
# Install Nordlys from a built tree.
#
#   cargo build --release
#   sudo scripts/install.sh                 # into /usr/local
#   scripts/install.sh --prefix /usr --destdir "$pkgdir"   # for packages
#   sudo scripts/install.sh --uninstall
#   sudo scripts/install.sh --vm-guest      # also the test VM's guest agent
#
# Installs the binaries, the session (Hyprland config, idle config,
# wallpaper, default apps, nordlys-session, nordlys-record), the PAM file
# and the SDDM theme, and registers "Nordlys" with the display manager.
# The SDDM theme is only installed, not switched on (scripts/setup.sh
# does that when it sets SDDM up). ~/.config/hypr is never touched.
set -eu

prefix=/usr/local
destdir=
uninstall=
vm_guest=
while [ $# -gt 0 ]; do
    case $1 in
        --prefix) prefix=$2; shift ;;
        --destdir) destdir=$2; shift ;;
        --uninstall) uninstall=1 ;;
        --vm-guest) vm_guest=1 ;;
        -h|--help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "install.sh: unknown option $1" >&2; exit 2 ;;
    esac
    shift
done

root=$(cd "$(dirname "$0")/.." && pwd)
bin="$destdir$prefix/bin"
data="$destdir$prefix/share/nordlys"
# Display managers only look in /usr/share/wayland-sessions.
sessions="$destdir/usr/share/wayland-sessions"
pam="$destdir/etc/pam.d"
units="$destdir$prefix/lib/systemd/user"
bins="nordlys-comp nordlys-shell nordlysctl nordlys-theme nordlys-settings"
apps="$destdir$prefix/share/applications"
portals="$destdir$prefix/share/xdg-desktop-portal"
entries="nordlys.desktop nordlys-comp.desktop"

# SDDM can be told to read Wayland sessions from other folders
# ([Wayland] SessionDir, comma-separated), such as one of hand-picked links.
# When none of them is $sessions, prints the first, where links to the
# entries go. Later files win, as in SDDM: its defaults, then
# /etc/sddm.conf.d, then /etc/sddm.conf.
sddm_session_dir() {
    confs=
    for f in "$destdir"/usr/lib/sddm/sddm.conf.d/* "$destdir"/etc/sddm.conf.d/* "$destdir/etc/sddm.conf"; do
        [ -f "$f" ] && confs="$confs $f"
    done
    [ -n "$confs" ] || return 0
    # shellcheck disable=SC2086
    dirs=$(cat $confs | awk '
        /^[[:space:]]*\[/ { wayland = ($0 ~ /^[[:space:]]*\[Wayland\]/) }
        wayland && /^[[:space:]]*SessionDir[[:space:]]*=/ { sub(/^[^=]*=[[:space:]]*/, ""); dir = $0 }
        END { print dir }')
    [ -n "$dirs" ] || return 0
    first=
    for d in $(echo "$dirs" | tr ',' ' '); do
        d=${d%/}
        [ "$destdir$d" = "$sessions" ] && return 0
        [ -n "$first" ] || first=$d
    done
    echo "$first"
}

if [ -n "$uninstall" ]; then
    for b in $bins nordlys-session nordlys-record nordlys-vm-agent; do rm -fv "$bin/$b"; done
    rm -rfv "$destdir/usr/share/sddm/themes/nordlys"
    rm -fv "$destdir/etc/udev/rules.d/70-nordlys-vm.rules"
    rm -rfv "$data"
    linked=$(sddm_session_dir)
    for e in $entries; do
        l="$destdir$linked/$e"
        [ -n "$linked" ] && [ -L "$l" ] && [ "$(readlink "$l")" = "/usr/share/wayland-sessions/$e" ] && rm -fv "$l"
        rm -fv "$sessions/$e"
    done
    rm -fv "$apps/nordlys-settings.desktop"
    rm -fv "$portals/nordlys-portals.conf"
    for u in "$root"/session/systemd/*; do rm -fv "$units/$(basename "$u")"; done
    # The PAM file only includes `login`; it's left in place on purpose
    # so a running lock screen can still check passwords.
    echo "Left $pam/nordlys in place; remove it by hand once nothing is locked."
    exit 0
fi

release="$root/target/release"
for b in $bins; do
    if [ ! -x "$release/$b" ]; then
        echo "install.sh: $release/$b is missing; run cargo build --release first" >&2
        exit 1
    fi
done

install -d "$bin" "$data" "$data/xdg" "$sessions" "$pam"
for b in $bins; do install -m755 "$release/$b" "$bin/$b"; done
install -m755 "$root/session/nordlys-session" "$root/session/nordlys-record" "$bin/"
install -m644 "$root/session/hyprland.lua" "$root/session/hypridle.conf" \
    "$root/session/wallpaper.jpg" "$data/"
install -m755 "$root/session/nordlys-autostart" "$data/"
install -m644 "$root/config/nordlys.toml" "$data/nordlys.toml.example"
# Default apps, behind the user's own (nordlys-session adds this to XDG_CONFIG_DIRS).
install -m644 "$root/session/xdg/mimeapps.list" "$data/xdg/mimeapps.list"
install -m644 "$root/session/nordlys.desktop" "$sessions/nordlys.desktop"
install -m644 "$root/session/nordlys-comp.desktop" "$sessions/nordlys-comp.desktop"
# Portals for the nordlys-comp session (XDG_CURRENT_DESKTOP=Nordlys).
install -d "$portals"
install -m644 "$root/session/xdg-desktop-portal/nordlys-portals.conf" "$portals/"
# Settings in app launchers.
install -d "$apps"
install -m644 "$root/crates/nordlys-settings/nordlys-settings.desktop" "$apps/"
# User services that keep the shell running (restarted if they crash).
install -d "$units"
for u in "$root"/session/systemd/*; do
    sed -e "s#@BINDIR@#$prefix/bin#g" -e "s#@DATADIR@#$prefix/share/nordlys#g" "$u" \
        >"$units/$(basename "$u")"
    chmod 644 "$units/$(basename "$u")"
done
# A prefix other than /usr isn't on every display manager's PATH.
if [ "$prefix" != /usr ]; then
    sed -i "s#^Exec=nordlys-session#Exec=$prefix/bin/nordlys-session#; s#^TryExec=nordlys-#TryExec=$prefix/bin/nordlys-#" \
        "$sessions/nordlys.desktop" "$sessions/nordlys-comp.desktop"
fi
# SDDM set to read sessions elsewhere wouldn't list Nordlys; link it in.
linked=$(sddm_session_dir)
if [ -n "$linked" ] && [ -d "$destdir$linked" ]; then
    for e in $entries; do ln -sfn "/usr/share/wayland-sessions/$e" "$destdir$linked/$e"; done
    echo "Linked the Nordlys sessions into $linked, where SDDM looks for them."
elif [ -n "$linked" ]; then
    echo "install.sh: SDDM reads sessions from $linked, which doesn't exist; Nordlys won't be listed there" >&2
fi
[ -e "$pam/nordlys" ] || install -m644 "$root/config/pam.d/nordlys" "$pam/nordlys"
# The login screen theme for SDDM, with the wallpaper beside it.
sddm_theme="$destdir/usr/share/sddm/themes/nordlys"
install -d "$sddm_theme"
install -m644 "$root"/session/sddm/nordlys/* "$root/session/wallpaper.jpg" "$sddm_theme/"

if [ -n "$vm_guest" ]; then
    install -m755 "$root/vm/nordlys-vm-agent" "$bin/nordlys-vm-agent"
    install -d "$destdir/etc/udev/rules.d"
    install -m644 "$root/vm/70-nordlys-vm.rules" "$destdir/etc/udev/rules.d/"
    : >"$data/vm-guest"
    if [ -z "$destdir" ]; then
        udevadm control --reload
        udevadm trigger --subsystem-match=virtio-ports
    fi
fi

echo "Installed Nordlys into $destdir$prefix."
[ -z "$destdir" ] && echo "Log out and pick \"Nordlys\" in your display manager, or run nordlys-session from a TTY."
exit 0
