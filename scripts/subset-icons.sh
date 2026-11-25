#!/bin/sh
# Rebuild crates/nordlys-shell/assets/lucide-subset.ttf from a Lucide font
# release (https://github.com/lucide-icons/lucide/releases, lucide-font-*.zip).
# Needs python3 with fontTools. Icons are picked by codepoint, so the
# ligature table (which fontTools can't read in Lucide's build) is dropped.
# Keep ICONS in step with crates/nordlys-shell/src/icons.rs.
#
#   scripts/subset-icons.sh path/to/lucide-font
set -eu
src=${1:?usage: subset-icons.sh path/to/lucide-font}
out=$(dirname "$0")/../crates/nordlys-shell/assets/lucide-subset.ttf
ICONS="wifi wifi-off bluetooth bluetooth-off bell bell-off moon music play pause
skip-forward sliders-horizontal volume volume-1 volume-2 volume-x x battery
battery-low battery-medium battery-full battery-charging columns-2 layout-grid
search terminal folder file app-window power lock settings history monitor
refresh-cw panel-left arrow-left-right eye eye-off arrow-right rotate-ccw
arrow-big-up"
python3 -I - "$src" "$out" $ICONS <<'PY'
import json, sys
from fontTools.ttLib import TTFont
from fontTools import subset
src, out, names = sys.argv[1], sys.argv[2], sys.argv[3:]
points = json.load(open(f"{src}/codepoints.json"))
font = TTFont(f"{src}/lucide.ttf")
for table in ("GSUB", "GPOS", "GDEF"):
    if table in font:
        del font[table]
options = subset.Options()
options.hinting = False
options.desubroutinize = True
options.layout_features = []
sub = subset.Subsetter(options)
sub.populate(unicodes=[points[n] for n in names])
sub.subset(font)
font.save(out)
PY
echo "wrote $out ($(wc -c < "$out") bytes)"
