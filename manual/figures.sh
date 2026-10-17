#!/bin/sh
# Regenerates the manual's figures. Run from the repository root:  sh manual/figures.sh
set -e
R="cargo run --release --quiet --example render_png --"
OUT=manual/figures
SMALL='$ beta integral sigma $'
# the same small formula through every backend at 2 px/pt, enlarged in the manual without smoothing
$R "$SMALL" $OUT/px-raster.png --backend raster --scale 2
$R "$SMALL" $OUT/px-svg.png    --backend svg    --scale 2
$R "$SMALL" $OUT/px-vector.png --backend vector --scale 2
$R "$SMALL" $OUT/px-hard.png   --backend vector --scale 2 --hard
# one glyph, large: the filled interior and the triangulation behind it
$R '$ alpha $' $OUT/alpha-fill.png --backend vector --scale 14
$R '$ alpha $' $OUT/alpha-wire.png --backend vector --scale 14 --wire
