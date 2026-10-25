#!/usr/bin/env python3
"""Studio screen layout for the recorder's "Studio ready" check.

  python3 tools/screens.py status   -> JSON: screens, Elgato on USB / on screen, layout ok
  python3 tools/screens.py fix      -> apply the studio layout, then print status

The studio layout: the two big monitors side by side at the top (the one that is
primary now stays primary and on the left), the Elgato Prompter (a DisplayLink
1024x600 panel, USB 17e9:ff1a) centred below the seam between them, which is
where it sits on the desk. GNOME added it between the two monitors in a row on
2026-10-15, so the mouse could only cross into it through a 600 px band.
"""
import json
import os
import subprocess
import sys

from gi.repository import Gio, GLib

BUS = ("org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig",
       "org.gnome.Mutter.DisplayConfig")
ELGATO_USB = "17e9:ff1a"


def state():
    bus = Gio.bus_get_sync(Gio.BusType.SESSION)
    serial, monitors, logical, _ = bus.call_sync(
        *BUS, "GetCurrentState", None, None, 0, -1, None).unpack()
    modes = {}
    for spec, ms, _props in monitors:
        cur = [m for m in ms if m[6].get("is-current")] or [m for m in ms if m[6].get("is-preferred")]
        if cur:
            modes[spec[0]] = cur[0]
    screens = []
    for x, y, _scale, _t, primary, mons, _p in logical:
        conn = mons[0][0]
        m = modes.get(conn)
        screens.append({"conn": conn, "x": x, "y": y, "w": m[1] if m else 0,
                        "h": m[2] if m else 0, "primary": primary, "mode": m[0] if m else None})
    return bus, serial, screens


def is_elgato(s):
    return s["w"] == 1024 and s["h"] == 600


PREFERRED = os.path.expanduser("~/.config/z8-studio/screens-preferred.json")


def load_preferred():
    try:
        return json.load(open(PREFERRED))
    except (OSError, ValueError):
        return None


def save_preferred(screens):
    """Remember the owner's own arrangement (2026-10-15: he wants the Elgato
    centred between the monitors; the old fixed plan kept moving it below)."""
    os.makedirs(os.path.dirname(PREFERRED), exist_ok=True)
    want = {s["conn"]: [s["x"], s["y"], s["primary"]] for s in screens}
    with open(PREFERRED + ".tmp", "w") as f:
        json.dump(want, f)
    os.replace(PREFERRED + ".tmp", PREFERRED)
    return want


def plan(screens):
    pref = load_preferred()
    if pref and set(pref) == {s["conn"] for s in screens}:
        return {c: tuple(v) for c, v in pref.items()}
    big = sorted([s for s in screens if not is_elgato(s)], key=lambda s: (not s["primary"], s["x"]))
    small = [s for s in screens if is_elgato(s)]
    want, x = {}, 0
    for i, s in enumerate(big[:2]):
        want[s["conn"]] = (x, 0, i == 0)
        x += s["w"]
    if small and len(big) >= 2:
        seam = big[0]["w"]
        want[small[0]["conn"]] = (seam - small[0]["w"] // 2, max(b["h"] for b in big[:2]), False)
    return want


def status():
    _bus, _serial, screens = state()
    usb = ELGATO_USB in subprocess.run(["lsusb"], capture_output=True, text=True).stdout
    want = plan(screens)
    ok = all((s["x"], s["y"]) == want[s["conn"]][:2] for s in screens if s["conn"] in want)
    return {"ok": True, "screens": screens, "elgatoUsb": usb,
            "elgatoScreen": any(is_elgato(s) for s in screens), "layoutOk": ok}


def save(want):
    """Write the layout into ~/.config/monitors.xml so it survives a logout.

    Applying with method 2 (persistent) makes GNOME Shell show "Keep these
    display settings?" and silently revert after 20 s when nobody clicks Keep,
    which is exactly what undid the first fix on 2026-10-15. So the change is
    applied as temporary (method 1, no dialog) and the saved file is edited
    here instead: only the <configuration> naming all of these connectors.
    """
    import os
    import re
    import shutil
    path = os.path.expanduser("~/.config/monitors.xml")
    try:
        text = open(path).read()
    except FileNotFoundError:
        return False
    def edit(cfg):
        body = cfg.group(0)
        if not all(f"<connector>{c}</connector>" in body for c in want):
            return body
        def lm(m):
            block = m.group(0)
            conn = re.search(r"<connector>([^<]+)</connector>", block)
            if not conn or conn.group(1) not in want:
                return block
            x, y, prim = want[conn.group(1)]
            block = re.sub(r"<x>-?\d+</x>", f"<x>{x}</x>", block, count=1)
            block = re.sub(r"<y>-?\d+</y>", f"<y>{y}</y>", block, count=1)
            block = re.sub(r"\s*<primary>yes</primary>", "", block)
            if prim:
                # GNOME writes <primary> right after <scale>; keep its order.
                block = re.sub(r"(<scale>[^<]*</scale>)", r"\1\n      <primary>yes</primary>", block, count=1)
            return block
        return re.sub(r"<logicalmonitor>.*?</logicalmonitor>", lm, body, flags=re.S)
    new = re.sub(r"<configuration>.*?</configuration>", edit, text, flags=re.S)
    if new == text:
        return False
    shutil.copy2(path, path + ".bak-z8")
    with open(path + ".tmp", "w") as f:
        f.write(new)
    os.replace(path + ".tmp", path)
    return True


def fix():
    bus, serial, screens = state()
    want = plan(screens)
    by = {s["conn"]: s for s in screens}
    cfg = [(x, y, 1.0, 0, prim, [(c, by[c]["mode"], {})]) for c, (x, y, prim) in want.items()]
    # Method 1 = temporary: no "Keep these settings?" dialog to revert it.
    bus.call_sync(*BUS, "ApplyMonitorsConfig",
                  GLib.Variant("(uua(iiduba(ssa{sv}))a{sv})", (serial, 1, cfg, {})),
                  None, 0, -1, None)
    out = status()
    out["saved"] = save(want)
    return out


if __name__ == "__main__":
    try:
        print(json.dumps(fix() if sys.argv[1:] == ["fix"] else status()))
    except Exception as e:  # noqa: BLE001 - the page shows the reason
        print(json.dumps({"ok": False, "error": str(e)}))
