#!/usr/bin/env python3
"""Glass-to-glass delay of the Zoom/OBS webcam, measured with the real camera.

A full-screen page flips black/white exactly on each wall-clock second. Point
the Z8 at that screen. This reads the camera's own MJPEG stream (a second,
read-only client of mtplvcap) and the finished frames on /dev/video10 at the
same time, finds the flashing area by itself (the pixels that change most),
and times every flip on both, against the same clock the page flips on:

    screen flips -> camera sees it -> USB -> mtplvcap   = "camera"
    ... -> keeper decodes, grades, writes /dev/video10  = "device"

    python3 glass_to_glass.py [seconds]      # the feed must be live

What it includes that is not the camera path: the page flips on the first
animation frame after the second (0-17 ms at 60 Hz) and the monitor then
takes its own frame or two to show it. So the totals are a little HIGH, and
the camera-to-device difference is exact either way. Read only: it never
writes the device, the control file or the camera.
"""
import http.client
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from io import BytesIO

from PIL import Image

W, H = 1024, 576
PORT = int(os.environ.get("PORT", "42839"))
DEV = os.environ.get("Z8_FEED_DEV", "/dev/video10")
SW, SH = 64, 36                     # analysis grid

PAGE = """<!doctype html><title>Z8 delay test</title>
<style>html,body{margin:0;height:100%;cursor:none;font:32px sans-serif}
#t{position:fixed;bottom:4%;width:100%;text-align:center;color:#888}</style>
<body><div id=t>Z8 delay test: point the camera at this screen</div><script>
let last=-1;(function f(){const p=Math.floor(Date.now()/1000)%2;
if(p!==last){document.body.style.background=p?'#fff':'#000';last=p}
requestAnimationFrame(f)})()</script>"""


def cam_reader(out, stop):
    c = http.client.HTTPConnection("127.0.0.1", PORT, timeout=3)
    c.request("GET", "/mjpeg")
    r = c.getresponse()
    while not stop.is_set():
        line = r.readline()
        if not line:
            return
        if not line.startswith(b"--"):
            continue
        n = None
        while True:
            h = r.readline()
            if not h or h in (b"\r\n", b"\n"):
                break
            if h.lower().startswith(b"content-length:"):
                n = int(h.split(b":", 1)[1])
        if n:
            jpeg = r.read(n)
            out.append((time.time(), jpeg))


def dev_reader(out, stop):
    fd = os.open(DEV, os.O_RDONLY)
    try:
        while not stop.is_set():
            raw = os.read(fd, W * H * 3 // 2)
            out.append((time.time(), raw[:W * H]))
    finally:
        os.close(fd)


def cam_grid(jpeg):
    im = Image.open(BytesIO(jpeg))
    im.draft("L", (im.width // 8, im.height // 8))
    im = im.convert("L")
    iw, ih = im.size                       # fill 16:9 like the keeper
    ch = iw * H // W
    y = (ih - ch) // 2
    return list(im.crop((0, y, iw, y + ch)).resize((SW, SH)).get_flattened_data())


def dev_grid(y):
    return list(Image.frombytes("L", (W, H), y).resize((SW, SH)).get_flattened_data())


def flips(frames):
    """Times at which the flashing area crosses the midpoint between its two
    levels, and the brightness step it saw (to tell a flash from noise)."""
    grids = [g for _, g in frames]
    n = len(grids[0])
    mean = [sum(g[i] for g in grids) / len(grids) for i in range(n)]
    var = [sum((g[i] - mean[i]) ** 2 for g in grids) for i in range(n)]
    top = sorted(range(n), key=lambda i: var[i], reverse=True)[:max(8, n // 20)]
    lum = [sum(g[i] for i in top) / len(top) for g in grids]
    lo, hi = sorted(lum)[len(lum) // 10], sorted(lum)[len(lum) * 9 // 10]
    mid = (lo + hi) / 2
    out = []
    for k in range(1, len(lum)):
        if (lum[k - 1] < mid) != (lum[k] < mid):
            out.append(frames[k][0])
    return out, hi - lo


def delays(times):
    """Each crossing measured from the wall-clock second before it."""
    return sorted((t - int(t)) * 1000 for t in times)


def report(name, times, step, secs):
    d = delays(times)
    p = lambda q: d[int((len(d) - 1) * q)] if d else 0     # noqa: E731
    # A real flash flips once a second, every second, at a steady delay.
    # Measured 2026-09-24 with the camera on his face instead of the screen:
    # 7 "flips" in 15 s spread from 42 to 640 ms, which is him moving.
    if len(d) < 0.7 * secs or step < 12 or p(.9) - p(.1) > 120:
        print(f"{name:7} no clear flash ({len(d)} flips in {secs:.0f} s, spread"
              f" {p(.9) - p(.1):.0f} ms, step {step:.0f} levels): point the"
              " camera at the flashing screen")
        return None
    print(f"{name:7} median {p(.5):6.1f} ms   p10 {p(.1):6.1f}   p90 {p(.9):6.1f}"
          f"   ({len(d)} flips, flash step {step:.0f} levels)")
    return p(.5)


def main():
    secs = float(sys.argv[1]) if len(sys.argv) > 1 else 20
    chrome = shutil.which("google-chrome") or shutil.which("chromium")
    td = tempfile.mkdtemp(prefix="z8-g2g-")
    page = os.path.join(td, "flash.html")
    with open(page, "w") as fh:
        fh.write(PAGE)
    br = subprocess.Popen(
        [chrome, f"--user-data-dir={td}/profile", "--no-first-run",
         "--start-fullscreen", "--kiosk", f"--app=file://{page}"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        start_new_session=True)
    try:
        print(f"flashing for {secs:.0f} s ...", flush=True)
        time.sleep(3)                                      # window up
        cams, devs, stop = [], [], threading.Event()
        for fn, buf in ((cam_reader, cams), (dev_reader, devs)):
            threading.Thread(target=fn, args=(buf, stop), daemon=True).start()
        time.sleep(secs)
        stop.set()
        time.sleep(0.3)
    finally:
        os.killpg(br.pid, signal.SIGTERM)
        shutil.rmtree(td, ignore_errors=True)
    if len(cams) < 30 or len(devs) < 30:
        print(f"too few frames: camera {len(cams)}, device {len(devs)}; is the feed live?")
        return 1
    ct, cstep = flips([(t, cam_grid(j)) for t, j in cams])
    dt, dstep = flips([(t, dev_grid(y)) for t, y in devs])
    c = report("camera", ct, cstep, secs)
    d = report("device", dt, dstep, secs)
    if c is not None and d is not None:
        print(f"keeper adds {d - c:.1f} ms; the rest is the screen, the camera and USB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
