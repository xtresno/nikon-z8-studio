# Z8 Studio on Windows (WSL2)

Z8 Studio runs on Windows 11 inside WSL2 (Ubuntu 24.04). The camera's USB
connection is passed into WSL with usbipd-win, and the page opens in any
Windows browser at http://localhost:8420. Verified on 2026-10-25 on an Asus
ROG Flow X13 (16 GB RAM) with a Nikon Z8 on firmware 2.01. The measurements
are in the table at the end.

## 1. WSL settings (Windows side, once)

Create `%UserProfile%\.wslconfig`:

    [wsl2]
    memory=10GB
    processors=8
    swap=8GB
    networkingMode=mirrored

Leave Windows a few GB: on a 16 GB machine, 10 GB for WSL is the most that
is safe. Mirrored networking lets the Windows browser, and Tailscale running
on Windows, reach the server on `localhost`. Then, in PowerShell, run
`wsl --shutdown` and reopen Ubuntu.

Inside WSL, `/etc/wsl.conf` needs `[boot]` `systemd=true`. Ubuntu 24.04 has
it by default.

## 2. Build (inside WSL)

Work in the WSL filesystem (`~/`), not under `/mnt/c`, which is many times
slower.

    sudo apt install build-essential pkg-config libgphoto2-dev libjpeg-turbo8-dev \
      ffmpeg libimage-exiftool-perl libheif-examples gphoto2 darktable xdg-utils \
      usbutils nodejs npm git
    curl https://sh.rustup.rs -sSf | sh
    git clone <this repo> ~/bin/z8-studio && cd ~/bin/z8-studio
    systemd-run --user --scope -p MemoryMax=6G cargo build --release -j 4
    (cd web && npm install && npm run build)

Let the camera be used without root:

    echo 'SUBSYSTEM=="usb", ATTR{idVendor}=="04b0", MODE="0666", GROUP="plugdev"' \
      | sudo tee /etc/udev/rules.d/60-nikon-z8.rules
    sudo udevadm control --reload-rules

## 3. Camera over USB (usbipd-win)

In PowerShell:

    winget install --exact dorssel.usbipd-win

Plug the Z8 in and switch it on (camera: Network menu > USB > MTP/PTP).
Then, in an admin PowerShell, run this once. Binding by hardware id keeps
working on any USB port:

    usbipd bind --hardware-id 04b0:0451

## 4. Start everything with WSL

Two user services: the server, and an attach loop that brings the camera
back into WSL every time it is switched on.

    mkdir -p ~/.config/systemd/user
    cp z8-studio.service z8-usb-attach.service ~/.config/systemd/user/
    sudo loginctl enable-linger $USER
    systemctl --user daemon-reload
    systemctl --user enable --now z8-studio z8-usb-attach

WSL stops a distro when no Windows program is using it, and then the
services stop too. To keep it running from login, without an open terminal,
add a logon task. Run this in PowerShell; it doesn't need admin:

    $a = New-ScheduledTaskAction -Execute 'C:\Windows\System32\conhost.exe' `
      -Argument '--headless C:\Windows\System32\wsl.exe -d Ubuntu --exec /bin/sleep infinity'
    $t = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
    $s = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit 0
    Register-ScheduledTask -TaskName 'Z8 Studio (keep WSL running)' -Action $a -Trigger $t -Settings $s

Open http://localhost:8420 in Edge or Chrome. Check it with `lsusb`, which
should list `04b0:0451 Nikon Corp. NIKON DSC Z 8`, and with
`gphoto2 --auto-detect` (stop the server first: only one program can hold
the camera).

## 5. Phone (optional)

With Tailscale installed on Windows, open PowerShell and run
`tailscale serve --bg --https=443 http://127.0.0.1:8420`. Then add the HTTPS
origin to the server:

    mkdir -p ~/.config/systemd/user/z8-studio.service.d
    printf '[Service]\nEnvironment=Z8_REMOTE_ORIGIN=https://<machine>.<tailnet>.ts.net\n' \
      > ~/.config/systemd/user/z8-studio.service.d/remote.conf
    systemctl --user daemon-reload && systemctl --user restart z8-studio

## 6. Shooting on a phone hotspot

The studio needs no internet at all: the camera, server and page all stay on
the laptop. On a pay-per-GB hotspot:

- Download everything, packages and any models, on home Wi-Fi first.
- In Windows, go to Settings > Network & internet > Wi-Fi > (the hotspot) and
  turn on **Metered connection**. Windows Update and most OneDrive syncing
  then pause.
- Photos are saved in WSL (`~/Pictures/z8-studio`), not in OneDrive.

## Measured (2026-10-25, ROG Flow X13, usbip at 480 Mbps)

| | |
|---|---|
| live view | 29.9 fps, 2.1 MB/s, 2 gaps over 200 ms in 60 s |
| JPEG Fine* shot, autofocus to file on disk | 2.2-2.4 s, 21.8 MB |
| RAW+JPEG shot (56 MB NEF + 22 MB JPEG) | 5.4-6.2 s, about 14 MB/s |
| camera switched off and on | back in WSL by itself; usbipd's first try fails while the camera boots, it retries about 11 s later, and the page goes Live 1.6 s after that |
| cargo build --release, 4 jobs | 2 min 12 s |

## Known differences from a Linux desktop

- **The webcam (Zoom) feed** needs v4l2loopback, which the WSL kernel does
  not have. Everything else works.
- **USB runs at USB 2 speed (480 Mbps) through usbip**, so RAW transfers are
  slower than with a native USB 3 connection.
- **Ubuntu 24.04's ffmpeg 6.1** fails two webcam-look tests
  (`every_webcam_look_lands_in_broadcast_range`,
  `the_zoom_look_matches_the_export_look`). They cover only the webcam feed.
- **No GPU acceleration is used.** Everything runs on the CPU.
