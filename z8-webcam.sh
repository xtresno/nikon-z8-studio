#!/usr/bin/env bash
# Nikon Z8 as a webcam on Linux. No capture card.
#
# The Z8 has no UVC/USB-streaming mode in ANY firmware (latest 3.10), so its
# NETWORK > USB menu only offers MTP/PTP, iPhone and USB-LAN. This pulls the PTP
# live view feed instead, via a locally patched mtplvcap, and pushes it into a
# v4l2loopback device so OBS sees an ordinary webcam.
#
#   z8-webcam.sh          -> /dev/video10 "Nikon Z8"   (OBS: Video Capture Device)
#   z8-webcam.sh http     -> MJPEG at http://127.0.0.1:42839/mjpeg only
#
# The v4l2 route exists because OBS's Media Source keeps its own playback buffer
# on top of Network Buffering, which no setting in that dialog fully removes.
# A V4L2 capture source has no such buffer.
#
set -uo pipefail

MODE="${1:-v4l2}"
PORT="${PORT:-42839}"
FPS="${FPS:-30}"
VIDEO_NR="${VIDEO_NR:-10}"
MTP=~/bin/mtplvcap

MTP_PID=""
FF_PID=""

# mtplvcap closes the camera's MTP session on SIGINT. Killing it any harder
# leaves the Z8 wedged: every later open fails LIBUSB_ERROR_TIMEOUT until the
# USB port is reset.
cleanup() {
  echo
  echo "[z8] stopping"
  [ -n "$FF_PID" ]  && kill      "$FF_PID"  2>/dev/null
  [ -n "$MTP_PID" ] && kill -INT "$MTP_PID" 2>/dev/null
  for _ in 1 2 3 4 5 6; do kill -0 "${MTP_PID:-0}" 2>/dev/null || break; sleep 0.5; done
  kill "${MTP_PID:-}" 2>/dev/null
  exit 0
}
trap cleanup INT TERM

# GNOME auto-mounts the camera as a PTP filesystem and holds USB interface 0.
release_camera() {
  pkill -f 'gvfsd-gphot[o]2' 2>/dev/null
  pkill -f 'gvfs-gphoto2-volume-monito[r]' 2>/dev/null
  sleep 1
}

# Recovers a wedged MTP session without unplugging anything.
reset_usb() {
  python3 - <<'PY'
import fcntl, os, re, subprocess
USBDEVFS_RESET = ord('U') << 8 | 20
m = re.search(r'Bus (\d+) Device (\d+): ID 04b0:', subprocess.check_output(['lsusb']).decode())
if not m:
    raise SystemExit("[z8] camera not on USB")
path = f"/dev/bus/usb/{m.group(1)}/{m.group(2)}"
fd = os.open(path, os.O_WRONLY)
fcntl.ioctl(fd, USBDEVFS_RESET, 0)
os.close(fd)
print(f"[z8] reset {path}")
PY
  sleep 4
}

start_mtp() {
  for attempt in 1 2 3; do
    release_camera
    "$MTP" -max-resolution -fps "$FPS" -port "$PORT" 2>&1 | sed 's/^/[mtp] /' &
    MTP_PID=$!
    sleep 6
    if kill -0 "$MTP_PID" 2>/dev/null; then return 0; fi
    echo "[z8] camera did not open (attempt $attempt), resetting USB"
    reset_usb
  done
  echo "[z8] could not open the camera. Power-cycle it and try again." >&2
  return 1
}

lsusb | grep -qi 'Nikon' || { echo "[z8] no Nikon on USB. Power the camera on." >&2; exit 1; }

start_mtp || exit 1

if [ "$MODE" = http ]; then
  echo "[z8] MJPEG at http://127.0.0.1:${PORT}/mjpeg (1024x576)"
  echo "[z8] running. Ctrl-C to stop."
  wait "$MTP_PID"
  exit 0
fi

if [ ! -e "/dev/video${VIDEO_NR}" ]; then
  echo "[z8] /dev/video${VIDEO_NR} missing. Load the module:" >&2
  echo "     sudo modprobe v4l2loopback" >&2
  cleanup
fi

# The camera sends full-range JPEG (yuvj420p, color_range=pc, Y spanning 0..255).
# v4l2loopback carries no range metadata, and OBS assumes LIMITED (16..235) for
# YUV, so passing the bytes through untouched crushes the shadows and oversaturates.
# `format=yuv420p` alone does NOT fix this, it only relabels the pixel format and
# leaves the values alone (measured: Y min 0, 6.5% of pixels below 16, unchanged).
# scale=in_range=full:out_range=limited is what actually rescales them.
#
# Set RANGE=full to skip the conversion and keep every code value, in which case
# set Color Range = Full on the OBS source to match.
case "${RANGE:-limited}" in
  full) VF="format=yuv420p" ;;
  *)    VF="scale=in_range=full:out_range=limited,format=yuv420p" ;;
esac

# nobuffer/low_delay stop ffmpeg holding frames back for reordering it will
# never need: MJPEG has no B-frames and the source is live.
# Z8_VF lets Z8 Studio push a look onto the Zoom/OBS feed. It already ends in
# format=yuv420p, which v4l2loopback needs, so it replaces the chain entirely.
if [ -n "${Z8_VF:-}" ]; then
  VF="$Z8_VF"
  echo "[z8] applying look: $VF"
fi

# NEVER STRETCH. v4l2loopback keeps whatever frame size the first writer gave
# it until every consumer closes the device, so once Zoom is open the size is
# fixed. Switching the camera from photo to video changes the live view from
# 3:2 (1024x680) to 16:9 (1024x576), and feeding that into a device still set
# to 680 tall stretched the picture by 18% vertically. Measured, not guessed.
#
# So: fit the source inside the device's current size and pad the remainder,
# which is correct for any combination of source and device.
# Parse AFTER the colon. `tr -dc '0-9/'` on the whole line also keeps the
# slash in the words "Width/Height", which yielded "/1024/680" and an empty
# width, so the fit was silently skipped and the picture stayed stretched.
# Only fit when the device size is actually FIXED, which is when some other
# consumer holds it open. With the device free, ffmpeg may set any size, so
# forcing it to the size read a moment ago is circular: the device could then
# never adopt the source's own geometry. Fitting unconditionally kept it at a
# stale 1024x680 forever.
HOLDERS=0
for _p in /proc/[0-9]*; do
  if ls -l "$_p/fd" 2>/dev/null | grep -q "video${VIDEO_NR}"; then
    HOLDERS=$((HOLDERS+1))
  fi
done

DEV_WH="$(v4l2-ctl --device="/dev/video${VIDEO_NR}" --all 2>/dev/null \
          | sed -n 's/^[[:space:]]*Width\/Height[[:space:]]*:[[:space:]]*//p' \
          | head -1 | tr -dc '0-9/')"
DEV_W="${DEV_WH%%/*}"; DEV_H="${DEV_WH##*/}"
if [ "$HOLDERS" -eq 0 ]; then
  echo "[z8] device is free, sending the camera's native geometry"
  FIT=""
elif [ -n "${DEV_W:-}" ] && [ -n "${DEV_H:-}" ] && [ "$DEV_W" -gt 0 ] 2>/dev/null; then
  FIT="scale=${DEV_W}:${DEV_H}:force_original_aspect_ratio=decrease,"
  FIT="${FIT}pad=${DEV_W}:${DEV_H}:(ow-iw)/2:(oh-ih)/2:color=black,"
  echo "[z8] fitting the feed to the device's ${DEV_W}x${DEV_H} without stretching"
else
  FIT=""
fi

# ALWAYS 16:9, 1024x576. Zoom and OBS are 16:9, and the Z8's live view is 3:2
# (1024x680) in photo mode and 16:9 (1024x576) only in video mode. Sent as-is,
# a photo-mode feed showed black bars at the sides in OBS, and switching modes
# with OBS holding the device added bars top and bottom (2026-09-22). Filling
# 16:9 and trimming the overflow, as any webcam does, gives one shape in both
# modes, so the device never has to change shape under a running app.
FILL="scale=1024:576:force_original_aspect_ratio=increase,crop=1024:576,"

ffmpeg -hide_banner -loglevel warning \
  -fflags nobuffer -flags low_delay -probesize 32 -analyzeduration 0 \
  -f mpjpeg -i "http://127.0.0.1:${PORT}/mjpeg" \
  -vf "${FILL}${FIT}${VF}" -f v4l2 "/dev/video${VIDEO_NR}" 2>&1 | sed 's/^/[ff] /' &
FF_PID=$!

sleep 3
echo "[z8] feeding /dev/video${VIDEO_NR} as 'Nikon Z8' (1024x576)"
echo "[z8] OBS: Video Capture Device (V4L2) -> Device: Nikon Z8"
echo "[z8] running. Ctrl-C to stop."
wait "$MTP_PID"
