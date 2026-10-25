#!/usr/bin/env python3
"""USB-reset the Scarlett (vendor 1235): the software unplug/replug.

The AMD USB controllers on this box intermittently drop the Scarlett's capture
stream ("Transfer event for disabled endpoint"), after which it sends nothing
until it is replugged. A USBDEVFS_RESET brings it back in about 2 s. Write access
comes from /etc/udev/rules.d/91-scarlett-user-reset.rules (plugdev).
"""
import fcntl
import glob
import json
import os

USBDEVFS_RESET = 21780


def main():
    for dev in glob.glob("/sys/bus/usb/devices/*"):
        if not os.path.exists(f"{dev}/idVendor"):
            continue
        if open(f"{dev}/idVendor").read().strip() != "1235":
            continue
        node = "/dev/bus/usb/%03d/%03d" % (int(open(f"{dev}/busnum").read()), int(open(f"{dev}/devnum").read()))
        fd = os.open(node, os.O_WRONLY)
        try:
            fcntl.ioctl(fd, USBDEVFS_RESET, 0)
        finally:
            os.close(fd)
        return {"ok": True, "node": node}
    return {"ok": False, "error": "the Scarlett is not connected"}


if __name__ == "__main__":
    try:
        print(json.dumps(main()))
    except OSError as e:
        print(json.dumps({"ok": False, "error": str(e)}))
