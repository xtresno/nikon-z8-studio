# Z8 Studio architecture: Rust + TypeScript

The server and the webcam keeper are Rust, the page is TypeScript. Same
routes, same bodies, same files on disk as the Python app they replaced, so
`selftest.py` (1000+ checks, 121 of them in a real browser) drives it over
HTTP unchanged.

**Status (2026-09-25): live.** `z8-studio.service` and `z8-feed.service` run
`target/release/z8-studio` and `target/release/z8-feed`. The Python server,
keeper and page were removed the same day, after the camera-on checks below.

## Layout

| path | what |
|---|---|
| `src/bin/z8-studio.rs` | the server (replaces `server.py`) |
| `src/bin/z8-feed.rs` | the webcam keeper (replaces `z8-feed.py`) |
| `src/http.rs` | all 54 routes |
| `src/img.rs` | the image engine that replaces Pillow |
| `src/grade.rs`, `src/analysis.rs` | grading; skin, focus, catchlight, light, Auto |
| `src/store.rs` | config, sessions, sidecars, thumbnails, exports, trash |
| `src/camera.rs`, `src/gp.rs` | the camera: one lock, time limits, libgphoto2 by hand |
| `src/feedctl.rs` | tether / webcam / off handover |
| `src/frame.rs` | the webcam frame engine (3.6 ms a frame on the Z8) |
| `csrc/jpeg.c` | libjpeg-turbo decode (with draft scaling) and encode |
| `web/src/app.ts` | the page, strict TypeScript |

## Build

    cargo build --release                 # target/release/z8-studio, z8-feed
    cd web && npm install && npm run build   # web/dist/app.js

## Run beside the real app (safe)

A second server that can see the live `mtplvcap` could stop it during a mode
switch, and one on the real photo folder can write into it. So a test copy
gets its own state folder, a sandbox photo root, and a fake `/proc` and USB bus:

    Z8_PORT=8421 Z8_HOME=<state dir with presets.json> Z8_ROOT=<sandbox photos> \
    Z8_PROC_ROOT=<empty dir> Z8_USB_ROOT=<empty dir> Z8_NO_AUTOCONNECT=1 \
    ./target/release/z8-studio

Environment: `Z8_PORT`, `Z8_HOME` (presets.json, config.json, state files),
`Z8_REMOTE_ORIGIN` (optional exact HTTPS origin for a private reverse proxy),
`Z8_WEB` (default `$Z8_HOME/web`), `Z8_ROOT`, `Z8_USB_ROOT`,
`Z8_PROC_ROOT`, `Z8_NO_AUTOCONNECT`, `Z8_MIN_FREE_MB`, `Z8_LOCK_SCALE`,
`Z8_AUDIO_MAX_SECONDS`, `Z8_FEED_CTRL`, `Z8_FEED_STATUS`. The keeper takes the
same variables the Python keeper did, plus `Z8_FEED_THREADS` and
`Z8_FEED_OUTFMT=file` (each frame overwrites a plain file, for looking at it).

## Verified

Phone access uses Tailscale Serve in front of the loopback listener. The
configured remote origin extends the Host/Origin allowlist without accepting
arbitrary hosts or cross-site POSTs. Rust tests cover local compatibility,
remote reads and writes, unconfigured access, and hostile origins. Home Screen
metadata and icons are served from explicit asset routes; no service worker
caches camera state or control requests. Phone CSS enlarges touch controls and
respects safe areas.

    cargo test --release                  # tests/behaviour.rs, in process
    Z8_BASE=http://127.0.0.1:8421 node phone-check.mjs # isolated server, WebKit phones + desktop
    python3 selftest.py --preflight       # the environment, 19 checks
    python3 selftest.py                   # the live server over HTTP

The suite's throwaway servers and keepers are `target/release/*` by default
(`Z8_SERVER_CMD`, `Z8_FEED_CMD` override). Sections that asserted on the
Python SOURCE skip with one named reason, and a ratchet stops that count
growing; port them to `cargo test`, never back to Python.

`phone-check.mjs` uses Playwright (`Z8_PLAYWRIGHT` can point at an existing
installation). It checks Home Screen assets and every panel at three phone
widths, landscape, and desktop without sending camera commands. Set
`Z8_PHONE_SHOTS` to a temporary directory for screenshots. WebKit emulation
does not replace testing the saved Home Screen app on a physical iPhone.

`mobile-workflow-check.mjs` drives populated panels, crop and grade saves,
single/selected-batch downloads, touch cancellation, offline edit recovery,
and simulated mobile page suspension in WebKit. It refuses a connected camera
or a photo root outside `/tmp/z8-mobile-*`; an empty disposable session is
seeded with synthetic JPEGs using Pillow. Run against the isolated server above
with `Z8_BASE=http://127.0.0.1:8422` and, if needed, `Z8_PLAYWRIGHT`.
The live-view checks mock camera reads; they do not operate the physical camera.
On 2026-09-26, all 28 workflow checks and all 121 existing desktop browser
checks passed against isolated synthetic-photo sessions. The separate layout
matrix passed at 320, 390, 430, and 844 pixels in WebKit and at 1500 pixels in
Chromium. Physical iPhone lock/unlock behavior and saving into Photos remain
manual device checks.

Mobile state handling (2026-09-26): GET requests have a timeout, status polls
are aborted on suspension, and resume starts a fresh live-view request. Offline
recovery does not reload away the selected photo. Edit saves capture the frame
name, session and values when queued, serialize writes, and flush on
selection/visibility changes. Failed writes remain queued and are also stored
in localStorage, scoped to the server's photo root and session. Reload restores
them; successful saves remove only the matching draft, preserving newer writes
from another tab. Storage can be unavailable or cleared by the browser; the UI
warns when a draft cannot be stored. The app still needs the computer for boot,
rendering and exports. A failed sidecar write returns an error and cleans up its
temporary file instead of reporting success. Touch controls, including slider
reset and focus inspection, have explicit tap actions; batch output links are
available directly in Export. Initial boot reuses its first status response.

`state-audit-check.mjs` covers photo Look persistence, shared sessions, queued
edits across reloads, multi-page Call selection, disk-write errors, path
validation and all slider reset buttons. It uses a disposable `/tmp/z8-mobile-*`
server and WebKit. The full suite includes it and the existing phone/workflow/
Call checks. In-page E2E uses real Playwright time through `e2e-check.mjs` so
browser timers do not outrun Rust workers. `--no-camera` excludes the
exposure-fix fuzz action and automatic/explicit USB opens.

Full-resolution point grading now composes byte lookup tables and mutates the
owned image in place. Contrast still uses the mean after exposure and tone;
saturation and every intermediate rounding step preserve the previous
renderer. Spatial processing and grade-before-resize order are unchanged.
Independent reference tests and actual exported JPEG hashes verify parity.

Call look updates (2026-09-26): `/api/call-look` validates and saves the
selection, then atomically updates the keeper control file and restart marker
when Webcam is active. A shared mutex serializes this with mode handovers. Status polls update other
pages without overwriting an in-flight local selection. Automatic slider-follow
requests check the currently saved selection under that mutex, so an older
page cannot replace a named look selected on another device.
The keeper replaces its grader on its next pass without reopening the camera
or virtual device. Slider-following changes are debounced and serialized by
the page. No hardware setting commands are involved.

`call-look-check.mjs` starts disposable server/keeper instances, feeds a
synthetic MJPEG stream, and reads the actual I420 output. Its 21 checks cover
iPhone WebKit and desktop Chromium selection, exact neutral restoration,
slider following, photo-edit isolation, failed requests, restart state,
inactive selection, and unchanged keeper/link/writer counters. Run with
`Z8_PLAYWRIGHT` pointing to an installed Playwright package if needed. The
same pixel-change check failed on the previous app, reproducing the bug.

Before the switch, against a sandbox copy with the camera off:
**1026 passed, 0 failed, 11 skipped** (all camera).

Image engine against Pillow on a real 45 MP portrait frame:

| step | max diff | mean |
|---|---|---|
| full decode + EXIF turn | 0 | 0.000 |
| draft decode | 0 | 0.000 |
| thumbnail | 1 | 0.17 |
| grade, the call look | 5 | 0.19 |
| fit / straighten | 1-3 | 0.17-0.55 |

Keeper: the suite's 8 keeper checks pass (live in 1.0 s, a killed link
replaced by itself, frame hold 2.5 ms median); idle, live and reconnect output
checked by eye.

## Audit pass, 2026-09-24 (camera off)

- **Camera bindings** checked against `/usr/include/gphoto2`: every argument,
  pointer depth, return type, the widget type numbers and the capture-path
  struct. Exercised with no camera (`src/bin/z8-gpcheck.rs`): errors come
  back as `[-105] Unknown model`, the text the app matches on.
- **Measurements** vs Python on 5 real frames: Auto within 0.006 on every
  knob, the Light report's verdict lines identical, focus score within 2-3%.
- **Crash audit**, three real ones fixed:
  - a live-view frame 1-2 px narrower than 1024 underflowed the crop and
    killed the keeper's frame thread, while its status kept saying live: a
    FROZEN webcam. Measured before the fix: 112 frames in, 0 sent. After:
    every frame sent, and that path now takes 5.9 ms instead of 17.7 ms;
  - one bad frame or one bad pass can no longer end the keeper;
  - a Light report on a crop under 6 px underflowed.
- **Memory**: 6.6 MB at rest (Python: about 340 MB), 97 MB after exporting
  20 full-size frames.
- **Speed**, same 20 frames, both servers side by side:

| job | Python | Rust |
|---|---|---|
| export 20 headshots, 2400x3000 | 14.2 s | 8.8 s |
| Rank 20 frames | 5.7 s | 3.6 s |
| Auto, one 45 MP frame | 510 ms | 274 ms |
| Light report | 591 ms | 280 ms |
| 20 previews | 5.9 s | 3.6 s |
| slider re-render | 7 ms | 4 ms |

- **Speed pass, 2026-09-25** (measured on this machine, real 45 MP frames):

| job | before | after |
|---|---|---|
| batch of 20: export / rank / Auto | 8.07 / 4.44 / 6.27 s | 2.92 / 1.41 / 2.12 s (3 frames at once) |
| lossless PNG export | 8.76 s | 0.80 s (fdeflate, still lossless) |
| peak memory, one full-size export | 672 MB | 410 MB |
| live view sent to the browser | 268 frames/s, 13 MB/s | 30 frames/s (all new), 1.4 MB/s |
| strip tile of a fresh raw | 3.9 s | 0.3 s (embedded JPEG, redrawn once developed) |
| opening a raw after the shot | about 4.7 s | already developed in the background |
| shot over USB 3 (cable, not code) | 1.6 s JPEG, 2.8 s raw | 1.2 s, 1.5 s |

- **The page has no `any` left.** Element lookups are typed from
  `index.html` (`web/gen-elements.py` writes `src/elements.d.ts`), and every
  route has a reply type. That found two display bugs: the Light table
  printed "null levels" when a band could not be measured, and Tidy's
  partial-failure message would have shown 0 moved, 0 failed.
- Child processes start in a new session (`setsid`), as Python's did.

## Camera on, 2026-09-25

- Verified through the Rust FFI: connect, live view (30 MB of MJPEG in 3 s),
  settings read and write-back with read-back, camera presets saved and
  restored to the exact snapshot, mode-aware read-only settings.
- Restart with a live-view stream open: 2.2 s, camera reconnects. It used to
  wait 90 s for the stream and be SIGKILLed, skipping the camera close.
- **Shoot verified through Rust**, on request, into a sandbox capture folder:
  AF-C, A mode, f/4 1/20 ISO 320, a 21 MB 8256x5504 JPEG in 1.6 s that
  decodes and carries the camera's EXIF. The same run proved the reconnect
  fix: the camera left USB asleep and was reopened on its own at return.
- Camera paths tested on request, into a sandbox, camera returned to its
  Headshot preset afterwards:
  - raw capture: a 55 MB NEF in 2.8 s, developed and Auto-graded in 4.9 s;
  - card: 400 files listed (capped, newest first) in 10 s, and a HEIF
    (.HIF) imported, renamed .heic and converted to JPEG in the background;
  - Fix exposure: A mode and the widest aperture, before/after reported;
  - movie: refused in photo mode with a message naming the switch; with the
    switch on video (the camera re-enumerated and was reopened on its own),
    a 4 s take recorded and stopped on command, the recording flag followed,
    and the clip imported from the card: 4K HEVC with 24-bit PCM, 4.00 s.
- HEIF CAPTURE is not testable over USB: the Z8 only offers it with Tone
  mode HLG, a camera menu.

## Found on the way

- The page's self-test read `state.toolsMissing`, which nothing ever set.
- "Use recorded audio" after "Play the cut" sorted takes against a missing
  clip time (`NaN`); now it says it picks the newest take.
- The orientation lease only expired inside auto-connect, so a server started
  without it kept a leased rotation for ever. It has its own timer here.
- The suite's throwaway servers always started `server.py` and ignored the
  sandbox; they now follow `Z8_ROOT` and `Z8_SERVER_CMD`.
