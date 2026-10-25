#!/usr/bin/env bash
# A small preview of a frame for Claude to look at, to save hotspot data.
#
#   tools/peek.sh                 the newest frame of today
#   tools/peek.sh <photo> [px]    a given frame (default 800 px wide)
#
# Prints the preview's path. Measured 2026-10-25: every image Claude looks at
# is sent again with each later message in the same conversation (a 92 KB
# image added about 70-100 KB to every following request). Keep previews
# small, and start a fresh conversation after a batch of reviews.
set -euo pipefail
ROOT=${Z8_ROOT:-$HOME/Pictures/z8-studio}
src=${1:-$(ls -t "$ROOT/$(date +%F)"/*.{jpg,JPG,nef,NEF} 2>/dev/null | grep -v '\.dev\.jpg$' | head -1 || true)}
px=${2:-800}
name=$(basename "$src" | sed 's/\.[^.]*$//')
[ -n "$src" ] && [ -f "$src" ] || { echo "no frame found" >&2; exit 1; }
# A raw: use its developed JPEG if the server made one, else its embedded preview.
case "${src,,}" in
  *.nef)
    dev="${src%.*}.dev.jpg"
    if [ -f "$dev" ]; then src=$dev
    else
      tmp=$(mktemp --suffix=.jpg)
      exiftool -b -JpgFromRaw "$src" > "$tmp" 2>/dev/null || exiftool -b -PreviewImage "$src" > "$tmp"
      src=$tmp
    fi ;;
esac
out=${TMPDIR:-/tmp}/z8-peek
mkdir -p "$out"
dst="$out/$name-${px}.jpg"
ffmpeg -v error -y -i "$src" -vf "scale=${px}:-2" -q:v 6 "$dst"
echo "$dst ($(( $(stat -c %s "$dst") / 1024 )) KB)"
