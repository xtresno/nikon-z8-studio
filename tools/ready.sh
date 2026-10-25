#!/usr/bin/env bash
# Ready-to-go check before a shoot, written for the Windows (WSL2) laptop but
# safe on Linux (the Windows checks are skipped). Read-only: it never fires
# the shutter or changes a camera setting.
#
#   tools/ready.sh                    run every check
#   tools/ready.sh --install-presets  also add the outdoor camera presets
#
# Exit code: 0 all good, 1 something to fix (the last line says what).
set -uo pipefail
cd "$(dirname "$0")/.."
B=http://127.0.0.1:8420
WIN=$( [ -x /mnt/c/Windows/System32/cmd.exe ] && echo 1 || echo 0 )
fails=() warns=()
ok()   { printf '  ok    %s\n' "$1"; }
warn() { printf '  WARN  %s\n' "$1"; warns+=("$1"); }
bad()  { printf '  FAIL  %s\n' "$1"; fails+=("$1"); }
js()   { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)" 2>/dev/null; }

if [ "${1:-}" = --install-presets ]; then
  python3 - <<'EOF'
import json, os
src = json.load(open('camera-presets.outdoor.json'))
path = 'camera-presets.json'
cur = json.load(open(path)) if os.path.exists(path) else {}
added = [k for k in src if not k.startswith('_') and k not in cur]
for k in added:
    cur[k] = src[k]
json.dump(cur, open(path + '.tmp', 'w'), indent=1)
os.replace(path + '.tmp', path)
print('  presets added:', ', '.join(added) or 'none (already there)')
EOF
fi

echo "Studio"
if systemctl --user is-active -q z8-studio 2>/dev/null || curl -s -m 3 -o /dev/null $B/; then
  ok "server running on $B"
else
  bad "server is not running: systemctl --user start z8-studio"
fi
state=$(curl -s -m 10 $B/api/state)

echo "Camera"
if lsusb 2>/dev/null | grep -q 'ID 04b0:'; then ok "Z8 on USB"; else bad "Z8 not on USB: plug it in and switch it on"; fi
if [ "$(echo "$state" | js "d['connected']")" = True ]; then
  ok "page connected to the camera"
  set=$(curl -s -m 20 $B/api/settings)
  bat=$(echo "$set" | js "d['battery']['value'].rstrip('%')")
  if [ -n "$bat" ]; then
    if [ "$bat" -lt 30 ]; then bad "camera battery $bat%: charge or swap it"
    elif [ "$bat" -lt 60 ]; then warn "camera battery $bat%: bring a spare"
    else ok "camera battery $bat%"; fi
  fi
  q=$(echo "$set" | js "d['quality']['value']")
  lever=$(echo "$set" | js "d['lever']['value']")
  [ -n "$q" ] && ok "image quality $q"
  [ "$lever" = video ] && warn "the PHOTO/VIDEO lever is on VIDEO"
  python3 - "$set" <<'EOF'
import json, sys
for n in json.loads(sys.argv[1]).get('_notices') or []:
    print('  note  ' + n.split('. ')[0] + '.')
EOF
else
  bad "page not connected to the camera (wait 15 s after switching on, then rerun)"
fi
python3 - "$(curl -s -m 5 $B/api/presets 2>/dev/null)" "$state" <<'EOF'
import json, sys
try:
    names = json.loads(sys.argv[2]).get('camPresets') or []
except Exception:
    names = []
out = [n for n in names if n.lower().startswith(('outdoor', 'golden'))]
print('  ok    outdoor presets: ' + ', '.join(out) if out else
      '  WARN  no outdoor camera presets: tools/ready.sh --install-presets')
EOF

echo "Laptop"
free=$(echo "$state" | js "d['freeMB']")
if [ -n "$free" ]; then
  # WSL's virtual disk lives on C:, so it can only grow as far as C: allows.
  if [ "$WIN" = 1 ]; then
    cfree=$(powershell.exe -NoProfile -Command "[int]((Get-PSDrive C).Free/1GB)" 2>/dev/null | tr -d '\r')
    [ -n "$cfree" ] && [ $(( cfree * 1024 )) -lt "$free" ] && free=$(( cfree * 1024 ))
  fi
  if [ "$free" -lt 20000 ]; then bad "only $((free/1024)) GB free for photos"
  else ok "$((free/1024)) GB free for photos, about $(( free / 80 )) RAW+JPEG shots (80 MB each)"; fi
fi
if [ "$WIN" = 1 ]; then
  read -r pct status < <(powershell.exe -NoProfile -Command \
    "\$b = Get-CimInstance Win32_Battery; '{0} {1}' -f \$b.EstimatedChargeRemaining, \$b.BatteryStatus" 2>/dev/null | tr -d '\r')
  if [ -n "${pct:-}" ]; then
    pct=$(( pct > 100 ? 100 : pct ))
    plug=$([ "$status" = 2 ] && echo "plugged in" || echo "on battery")
    if [ "$pct" -lt 40 ] && [ "$status" != 2 ]; then bad "laptop battery $pct% ($plug)"
    else ok "laptop battery $pct% ($plug)"; fi
  fi

  echo "Network"
  ssid=$(netsh.exe wlan show interfaces 2>/dev/null | tr -d '\r' | sed -n 's/^ *SSID *: //p' | head -1)
  if [ -n "$ssid" ]; then
    cost=$(netsh.exe wlan show profile name="$ssid" 2>/dev/null | tr -d '\r' | sed -n 's/^ *Cost *: //p' | head -1)
    case "$ssid" in
      *iPhone*) [ "$cost" = Fixed ] && ok "on the hotspot ($ssid), metered" \
                  || bad "hotspot $ssid is NOT metered: Settings > Network & internet > Wi-Fi > $ssid > Metered connection" ;;
      *) ok "Wi-Fi: $ssid (not the hotspot)" ;;
    esac
  else
    warn "no Wi-Fi: Claude needs internet; the studio itself does not"
  fi
  TS="/mnt/c/Program Files/Tailscale/tailscale.exe"
  if [ -x "$TS" ]; then
    "$TS" serve status 2>/dev/null | grep -q 8420 && ok "phone link (Tailscale Serve) on" \
      || warn "phone link off: tailscale serve --bg --https=443 http://127.0.0.1:8420"
  fi
  powershell.exe -NoProfile -Command "(Get-ScheduledTask -TaskName 'Z8 Studio (keep WSL running)' -ErrorAction SilentlyContinue).State" 2>/dev/null \
    | tr -d '\r' | grep -q . && ok "WSL keep-alive task installed" || warn "no WSL keep-alive task: see docs/WINDOWS-WSL2.md"
fi

echo "AI editor"
if [ -x ig-studio/.venv/bin/python ]; then
  missing=$(cd ig-studio && for f in models/depth_anything_v2_small.onnx models/face_landmarker.task; do [ -s "$f" ] || echo "$f"; done)
  [ -s ~/.rembg/models/birefnet-portrait/birefnet-portrait.onnx ] || missing="$missing birefnet-portrait"
  [ -z "$missing" ] && ok "models on disk, nothing to download" || bad "models missing ($missing): run ig-studio/setup.sh on home Wi-Fi"
else
  [ -d ig-studio ] && warn "ig-studio not set up: ig-studio/setup.sh on home Wi-Fi"
fi

echo
echo "On the camera (cannot be read over USB): Setup menu > USB power delivery > OFF saves the laptop battery."
if [ ${#fails[@]} -gt 0 ]; then
  echo "NOT READY. First fix: ${fails[0]}"; exit 1
elif [ ${#warns[@]} -gt 0 ]; then
  echo "READY, with ${#warns[@]} warning(s). Worth fixing: ${warns[0]}"
else
  echo "READY. Go shoot."
fi
