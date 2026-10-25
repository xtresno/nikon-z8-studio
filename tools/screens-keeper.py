#!/usr/bin/env python3
"""Keep the owner's screen layout across Elgato reconnects.

GNOME re-inserts the Elgato Prompter in its own spot every time it reconnects.
This remembers the arrangement HE chooses and restores it on reconnects:

- the set of screens is unchanged  -> he moved something: save it as preferred
- the set of screens changed       -> a screen (dis)connected: restore preferred

2026-10-15: the first version forced a fixed "Elgato below the middle" layout
and kept undoing his own arrangement.
"""
import os
import sys
import time

from gi.repository import Gio, GLib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import screens  # noqa: E402

pending = None
last_set = None
ours_until = 0.0  # ignore the change our own apply causes


def conns():
    return frozenset(s["conn"] for s in screens.state()[2])


def check():
    global pending, last_set, ours_until
    pending = None
    try:
        _, _, scr = screens.state()
        now_set = frozenset(s["conn"] for s in scr)
        if time.time() < ours_until:
            last_set = now_set
            return False
        if last_set is not None and now_set == last_set:
            screens.save_preferred(scr)
            print(f"saved his layout: {[(s['conn'], s['x'], s['y']) for s in scr]}", flush=True)
        else:
            st = screens.status()
            if st.get("elgatoScreen") and not st.get("layoutOk") and screens.load_preferred():
                ours_until = time.time() + 4
                out = screens.fix()
                print(f"restored his layout: {[(s['conn'], s['x'], s['y']) for s in out['screens']]}", flush=True)
        last_set = now_set
    except Exception as e:  # noqa: BLE001 - never die
        print(f"check failed: {e}", flush=True)
    return False


def changed(*_):
    global pending
    if pending:
        GLib.source_remove(pending)
    pending = GLib.timeout_add_seconds(2, check)


bus = Gio.bus_get_sync(Gio.BusType.SESSION)
bus.signal_subscribe(None, "org.gnome.Mutter.DisplayConfig", "MonitorsChanged",
                     "/org/gnome/Mutter/DisplayConfig", None, Gio.DBusSignalFlags.NONE, changed)
last_set = conns()
print("watching monitor changes; remembers the owner's layout", flush=True)
GLib.MainLoop().run()
