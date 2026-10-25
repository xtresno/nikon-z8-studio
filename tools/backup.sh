#!/usr/bin/env bash
# Copy the photo folder to an external drive and prove every file arrived.
#
#   tools/backup.sh            find the drive by itself (a Windows drive whose
#                              label or model says SanDisk, else the only
#                              removable/USB drive)
#   tools/backup.sh E          use drive E: (WSL), or any folder path
#
# Copies ~/Pictures/z8-studio to <drive>/Z8-Studio-backup, adding new files and
# never deleting on the drive. Then checks every file's size, and the newest
# session's files byte for byte. Re-running only copies what is new.
set -euo pipefail
SRC=${Z8_ROOT:-$HOME/Pictures/z8-studio}
arg=${1:-}

mount_letter() {             # E -> /mnt/e, mounting it if WSL has not yet
  local l m
  l=$(echo "$1" | tr 'A-Z' 'a-z'); m=/mnt/$l
  if ! mountpoint -q "$m"; then
    sudo mkdir -p "$m" && sudo mount -t drvfs "${l^^}:" "$m" -o metadata,uid=$(id -u),gid=$(id -g)
  fi
  echo "$m"
}

if [ -z "$arg" ]; then
  [ -x /mnt/c/Windows/System32/cmd.exe ] || { echo "Give a destination folder: tools/backup.sh /media/ssd"; exit 1; }
  arg=$(powershell.exe -NoProfile -Command '
    $v = Get-Volume | Where-Object { $_.DriveLetter -and $_.DriveLetter -ne "C" }
    $d = Get-Disk | Where-Object { $_.BusType -eq "USB" } | ForEach-Object { Get-Partition -DiskNumber $_.Number } |
         Where-Object DriveLetter | Select-Object -Expand DriveLetter
    $s = $v | Where-Object { $_.FileSystemLabel -match "sandisk|extreme" } | Select-Object -Expand DriveLetter
    if ($s) { $s | Select-Object -First 1 } elseif (@($d).Count -eq 1) { $d } ' 2>/dev/null | tr -d '\r' | head -1)
  [ -n "$arg" ] || { echo "No external drive found. Plug the SanDisk in, or name it: tools/backup.sh E"; exit 1; }
fi
if [[ "$arg" =~ ^[A-Za-z]:?$ ]]; then root=$(mount_letter "${arg%:}"); else root=$arg; fi
DST=$root/Z8-Studio-backup
mkdir -p "$DST"

need=$(du -sb "$SRC" | cut -f1); have=$(df -B1 --output=avail "$root" | tail -1)
echo "Backing up $SRC -> $DST ($((need/1024/1024/1024)) GB in the folder, $((have/1024/1024/1024)) GB free on the drive)"
t0=$(date +%s)
# exFAT/NTFS keep 2 s timestamps and no Unix owners: compare by size and time
# with a 2 s window, never delete on the drive, skip caches the app rebuilds.
rsync -rt --modify-window=2 --exclude='.cache/' --info=progress2 --no-inc-recursive "$SRC"/ "$DST"/
echo "Copied in $(( $(date +%s) - t0 )) s. Checking..."

bad=$(cd "$SRC" && find . -path '*/.cache' -prune -o -type f -printf '%s %p\n' | while read -r s p; do
  [ "$(stat -c %s "$DST/$p" 2>/dev/null)" = "$s" ] || echo "$p"; done)
if [ -n "$bad" ]; then echo "MISSING OR WRONG SIZE:"; echo "$bad"; exit 1; fi
n=$(cd "$SRC" && find . -path '*/.cache' -prune -o -type f -print | wc -l)
last=$(ls -1 "$SRC" | grep -E '^[0-9]{4}-[0-9]{2}-[0-9]{2}$' | tail -1)
if [ -n "$last" ]; then
  (cd "$SRC/$last" && find . -path '*/.cache' -prune -o -type f -print0 | xargs -0 -r sha1sum) > /tmp/z8-backup-$$.sha1
  (cd "$DST/$last" && sha1sum --quiet -c /tmp/z8-backup-$$.sha1) || { echo "BYTE CHECK FAILED in $last"; exit 1; }
  rm -f /tmp/z8-backup-$$.sha1
fi
echo "DONE: all $n files on the drive, sizes match; session $last checked byte for byte."
