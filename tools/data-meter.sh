#!/usr/bin/env bash
# How much data this machine has sent and received over the network it is on,
# for shooting on a pay-per-GB hotspot.
#
#   tools/data-meter.sh start   remember the counters now (do this when the
#                               laptop joins the hotspot)
#   tools/data-meter.sh         MB used since 'start'
#
# It counts WSL's traffic: Claude Code and anything run here. Windows' own
# traffic is in Settings > Network & internet > Data usage.
set -euo pipefail
STATE=${XDG_STATE_HOME:-$HOME/.local/state}/z8-data-meter
mkdir -p "$(dirname "$STATE")"
iface=$(ip route show default 2>/dev/null | awk '{print $5; exit}')
[ -n "$iface" ] || { echo "no network"; exit 1; }
rx=$(cat /sys/class/net/"$iface"/statistics/rx_bytes)
tx=$(cat /sys/class/net/"$iface"/statistics/tx_bytes)
if [ "${1:-}" = start ] || [ ! -f "$STATE" ]; then
  echo "$iface $rx $tx $(date +%s)" > "$STATE"
  echo "Meter started on $iface."
  exit 0
fi
read -r i0 rx0 tx0 t0 < "$STATE"
if [ "$i0" != "$iface" ] || [ "$rx" -lt "$rx0" ]; then
  echo "The network changed since 'start' ($i0 -> $iface); run: tools/data-meter.sh start"
  exit 1
fi
mins=$(( ( $(date +%s) - t0 ) / 60 ))
awk -v r=$((rx - rx0)) -v t=$((tx - tx0)) -v m=$mins \
  'BEGIN { printf "Since start (%d min): %.1f MB down, %.1f MB up, %.1f MB total\n", m, r/1e6, t/1e6, (r+t)/1e6 }'
