#!/bin/sh
# Install Nordlys from a built tree.
#
#   cargo build --release
#   sudo scripts/install.sh                 # into /usr/local
#   scripts/install.sh --prefix /usr --destdir "$pkgdir"   # for packages
#   sudo scripts/install.sh --uninstall
#
# Installs the binaries, the session (Hyprland config, idle config,
# wallpaper, nordlys-session) and the PAM file, and registers "Nordlys"
# with the display manager. ~/.config/hypr is never touched.
set -eu

prefix=/usr/local
destdir=
uninstall=
while [ $# -gt 0 ]; do
    case $1 in
        --prefix) prefix=$2; shift ;;
        --destdir) destdir=$2; shift ;;
        --uninstall) uninstall=1 ;;
        -h|--help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
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
# nordlys-comp is headless only for now, so it stays out.
bins="nordlys-shell nordlysctl nordlys-theme"

if [ -n "$uninstall" ]; then
    for b in $bins nordlys-session; do rm -fv "$bin/$b"; done
    rm -rfv "$data"
    rm -fv "$sessions/nordlys.desktop"
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

install -d "$bin" "$data" "$sessions" "$pam"
for b in $bins; do install -m755 "$release/$b" "$bin/$b"; done
install -m755 "$root/session/nordlys-session" "$bin/nordlys-session"
install -m644 "$root/session/hyprland.lua" "$root/session/hypridle.conf" \
    "$root/session/wallpaper.jpg" "$data/"
install -m644 "$root/config/nordlys.toml" "$data/nordlys.toml.example"
install -m644 "$root/session/nordlys.desktop" "$sessions/nordlys.desktop"
# A prefix other than /usr isn't on every display manager's PATH.
if [ "$prefix" != /usr ]; then
    sed -i "s#^Exec=nordlys-session#Exec=$prefix/bin/nordlys-session#; s#^TryExec=nordlys-session#TryExec=$prefix/bin/nordlys-session#" \
        "$sessions/nordlys.desktop"
fi
[ -e "$pam/nordlys" ] || install -m644 "$root/config/pam.d/nordlys" "$pam/nordlys"

echo "Installed Nordlys into $destdir$prefix."
[ -z "$destdir" ] && echo "Log out and pick \"Nordlys\" in your display manager, or run nordlys-session from a TTY."
exit 0
