# Z8 Studio

Tethered capture, editing and export for a Nikon Z8 on Linux, and on Windows
through WSL2 ([setup](docs/WINDOWS-WSL2.md)).

Plug the camera in by USB and get live view, remote shooting, a fast photo
editor and exporter, a video recorder with a teleprompter, and a webcam feed,
all in the browser, on the computer or on your phone. Built to be driven by an
AI assistant too (for example Claude Code), through the same HTTP API the page
uses.

**Quick start (Ubuntu 24.04 or WSL2 on Windows):**

    sudo apt install build-essential pkg-config libgphoto2-dev libjpeg-turbo8-dev \
        ffmpeg libimage-exiftool-perl libheif-examples nodejs npm
    curl --proto '=https' -sSf https://sh.rustup.rs | sh     # Rust toolchain
    cargo build --release                    # target/release/z8-studio
    (cd web && npm install && npm run build) # web/dist/app.js
    ./target/release/z8-studio               # then open http://127.0.0.1:8420

On Windows, run it inside WSL2 and pass the camera's USB in with
[usbipd-win](https://github.com/dorssel/usbipd-win). The webcam output
(v4l2loopback) does not work inside WSL2. See [ARCHITECTURE.md](ARCHITECTURE.md)
for how it fits together. Wireless upload without a cable:
[z8-wifi-drop](https://github.com/xtresno/z8-wifi-drop). MIT License.

The rest of this file is the full manual.

    http://127.0.0.1:8420          also in the GNOME app grid as "Z8 Studio"

It runs as a user service, so it is already up after login. **Plug the Z8 in and
switch it on, that is the whole ritual**, the page connects by itself.

Press **?** in the page for the keyboard list.

## iPhone and Home Screen app

The computer runs Z8 Studio and keeps the camera connected over USB. On the
iPhone, leave Tailscale connected to the same private network, then open the
computer's private HTTPS address in Safari. This works on the same Wi-Fi.
The computer must stay awake; shooting and live view also need the camera on.

In Safari, tap **Share > Add to Home Screen**, leave **Open as Web App** on
if shown, and tap **Add**. The Z8 Studio icon opens the same camera, sessions,
and controls. It needs a connection to the computer; it is not an offline app.
Photo exports are generated on the computer; use their download links to save
them on the phone. **Open folder** opens a folder on the computer.

On a phone, **Select frames** lets you tap several thumbnails for one export.
Each finished file gets a **Download** link in Export. Tap a slider's value
to reset it, and use **Check focus at 1:1** to inspect the centre of the crop.
Panel tabs stay available while scrolling through editing controls.

Live view closes while the page is hidden and reconnects when you return.
Brief connection failures preserve the selected photo. Pending edits are tied
to their photo and session, stored in browser storage when available, and
retried after reconnecting, including after a page reload. If storage is
unavailable the page warns you to keep it open until edits save. The computer
must still be reachable to open the app and render or export photos; this is
not a full offline workspace.

One-time setup on the computer (replace the example with its Tailscale DNS name):

1. Set `Z8_REMOTE_ORIGIN=https://your-computer.your-tailnet.ts.net` in the
   `z8-studio.service` environment and restart that service.
2. Run `tailscale serve --bg --https=443 http://127.0.0.1:8420`.
3. Open the HTTPS URL reported by `tailscale serve status` on the iPhone.

Use **Serve**, which is private to your Tailscale network. The app still binds
only to loopback; only the configured HTTPS origin and existing local origins
are accepted. No router forwarding is needed. To remove phone access, run
`tailscale serve --https=443 off` and remove `Z8_REMOTE_ORIGIN` from the service.

> **Since 2026-09-25 the app is Rust + TypeScript** (`src/`, `web/`); the
> Python server, keeper and page are gone. See `ARCHITECTURE.md` for layout,
> build and tests. Sections below that name `server.py`, `z8-feed.py` or
> `app.js` are history: the behaviour they describe carried over.

Built because **NX Tether has no Linux build**, and Wine cannot help: it needs
Nikon's USB driver layer, not just the application.

## Shooting a headshot session, start to finish

The short version, for when this has not been touched for a while.

**On the camera, once.** These four cannot be set over USB and the app can only
measure a frame afterwards and tell you they are wrong:

1. Photo Shooting Menu > **Tone mode** > **SDR**. Not HLG. This is the one that
   makes everything look washed out, and it is not subtle.
2. ISO sensitivity settings > **Auto ISO** > **OFF**, or the ISO and aperture
   controls in the app do nothing.
3. **Picture Control** > **Portrait**. Inert until SDR is on.
4. Only if shooting raw: RAW recording > **RAW compression** > **Lossless
   compressed**. High Efficiency raw cannot be developed by anything here.

**In the app.** Plug in, switch on, the page goes Live by itself.

1. Zoom to **100-120mm** and step back. The panel shows the focal length.
2. Press **Restore** on the *Headshot* preset, or dial it in and press **Save**.
3. Shoot a test frame and press **Light**. It measures the lighting and says
   what to move. Catchlight and range are the two that matter.
4. Shoot the session. Cull with `p` / `x` / `u` and the arrows, or press
   **Rank** to see which frames have a fault.
5. Press **Auto all**, which corrects every frame on its own rather than
   copying one grade around.
6. Crop the keepers, pick an export preset, **Export all**.

**The numbers a good frame hits**, all of which the app measures for you:

| | want |
|---|---|
| face brightness | about 138 of 255 |
| skin red-blue spread | about +36 |
| catchlight in the eye | above 0.05% of the eye area |
| tonal range used | 90% or more |
| face above backdrop | 40 to 80 levels |
| clipped pixels | zero |

## The four things this is for

Each one, start to finish. Everything else in this file is reference.

**A headshot session.** Plug in and switch on, the page goes Live by itself.
Shoot. Cull with `p` / `x` / `u` and the arrow keys. Grade the best frame, then
**Look to picks** to push that look across the keepers. Choose
*Actor headshot, 8x10 print* and **Export all** with *picks only* ticked.

**A skit with proper sound.** **Check audio**, press **Record audio**, start
the camera recording, and **clap once**. Stop both, open the clip, then press
**Use recorded audio**: it finds the clap in each track, lines them up, and
replaces the camera's microphone with the real one.
Then **Play the synced take** to check it.

**A self-tape.** **Check audio** first, because a take with no sound is a
wasted take. Record, click the clip, **Set in** / **Set out** (`i` / `o`) to
cut the slate off the head, **Save trim**, then **Play the cut** to check it
before you send it. Precise is the default because it lands on the frame you
chose.

**A YouTube thumbnail.** Open the clip, scrub to the moment, **Grab frame**. The
still joins the strip as an ordinary shot, so grade it with *Thumbnail punch*
and export at *YouTube thumbnail*, 1280x720.

**A Zoom call or an OBS scene.** Press **Webcam**. The camera becomes
`/dev/video10`, named "Nikon Z8", with your current look applied. In OBS pick
Video Capture Device (V4L2); in Zoom pick the same camera. If you look dark,
**Fix exposure** lets the camera meter itself without leaving the call. If
nobody can hear you, **Check audio** says whether sound is reaching the
computer at all.

## Starting a session

**Plug the Z8 in and switch it on. That is the whole ritual.** The server
watches the USB bus and opens the camera by itself, so the page goes from
*No camera* to *Connecting…* to *Live* without anything being pressed.

Pressing **Off** is a deliberate release and stops the auto-connect, so it
will not immediately take the camera back; press **Tether** when you want it
again. Set the photo/video switch to **photo** before shooting stills.

**Burst keeps every frame.** Capture names are second-resolution and the UI
fires burst frames 700 ms apart, so consecutive shots shared a name and only
the last survived; a 10 frame burst could come back as three files. Colliding
names now get `-2`, `-3` and so on, and the check looks at every representation
of a stem because a HEIF capture leaves both a `.heic` and its converted
`.jpg`.

The two status probes on the polling path both read the kernel directly
instead of spawning a process. `/api/state` went from **49 ms to under 2 ms**
per poll: 26 ms of it was a single `pgrep` for the webcam feed, 98% of the
whole endpoint, now a `/proc` scan cached for two seconds and invalidated the
moment you press Webcam or Tether.

Presence is read from **sysfs**, not by shelling out to `lsusb`. At one probe
every two seconds that is the difference between 865 seconds of CPU a day and
44; `lsusb` stays as a fallback for a system without that sysfs layout, and
`Z8_USB_ROOT` points the scan at a fake tree so the self-test can exercise it
without a camera.

Auto-connect also stands down whenever the webcam feed is running, however it
was started. That matters because `z8-webcam.sh` and the GNOME **nikon-z8**
launcher both grab the camera without going through the server, and without the
check auto-connect would fight mtplvcap every six seconds and wedge the MTP
session until a USB reset.

## Modes

**Tether** shoots stills. **Webcam** hands the camera to the OBS and Zoom feed
on `/dev/video10`. **Off** releases it. They cannot run at once, which is why
the switch exists: the webcam pipeline holds the MTP session, so a tether
connect fails while it runs.

## Layout (redesigned 2026-09-22)

- **Stage**: the big picture is the GRADED, CROPPED render, the same code path
  as the export. Dragging a slider nudges it instantly with a CSS filter, then
  the real render swaps in (about 0.5 s on a 45 MP frame). Clicking a thumbnail
  shows that thumbnail at once (about 60 ms) and sharpens when the render lands.
  Hold **Compare** to see the original, badged "Before".
- **Filmstrip** under the stage: day, filter, Pick / Reject / Clear / Rank /
  Delete, and the thumbnails.
- **Sidebar tabs**: **Camera** (settings, saved settings, on a call, sound),
  **Edit** (histogram, crop, look, apply to other frames), **Export** (preset,
  export, whole session, output folder), **Library** (rank results, clips,
  audio takes, folders, card, housekeeping). `?tab=edit` opens a tab directly.
- Messages go to the footer under the tabs AND a toast over the stage. They
  used to land at the bottom of the Export panel, out of sight.
- Save and Tidy ask in an in-page dialog, not the browser's `prompt()` /
  `confirm()`, which froze every timer on the page including the state poll.

Bugs the redesign found, each verified in a real browser:

- The page never re-read `/api/state` after boot. A camera that auto-connected
  still said "No camera" until a reload. It polls every 3 s now, and the strip,
  clips and takes rebuild only when their content changes. The takes list used
  to be rebuilt under a playing take and deleted its player.
- `display:` rules beat the `hidden` attribute, so a hidden empty `<img>` drew
  a broken-image icon in the middle of the stage.
- A portrait frame spilled off the bottom of the stage: `max-height:100%`
  resolved against an auto-sized grid row.
- Selecting a frame decoded the same 45 MP file three times in parallel (stage,
  crop pane, histogram): **8.1 s** from click to picture. One decode per frame
  now, drafted at 1/2 scale (937 ms to 425 ms, max 2/255 after the resize).
- The crop box vanished for the length of every preview reload (`naturalWidth`
  reads 0 mid-load), so a drag begun then grabbed nothing.
- Also: a marked batch said "cancel" on a disabled button; space fired the
  shutter behind the clip player; arrow keys walked frames the filter hid; the
  call look list never learned a newly saved look; rotation showed 0 degrees at
  boot while the server rotated 90; the "Showing <day>" warning named the FIRST
  past day forever; the session dropdown's counts never moved; a late histogram
  answer painted over the current frame's.

Server fixes from the same review:

- **`save_config()` wrote transient test overrides to disk.** It started from
  `load_config()`, which merges them, so any ordinary save during a suite run
  persisted the suite's `/tmp` export and capture folders. Both were on disk
  when found. Guarded by `a_save_never_persists_a_transient_override`.
- The session picker counted a clip's `.poster.jpg` as a frame; one predicate,
  `is_frame_file`, now serves both counts.
- Cross-site requests are refused. Listening on 127.0.0.1 keeps out other
  machines, not other websites: any open page could POST a no-preflight
  `text/plain` body to `/api/empty-trash`. Host must name this server and a POST
  with an Origin must be this app's. Scripts send no Origin and are unaffected.

Second review, same day, each guarded by `production_review_2026_09_22`:

- Thumbnails raced: concurrent requests for one new tile shared a `.tmp`
  name and the pruner deleted other requests' in-flight files. 2 of 12 came
  back 415 (a broken tile); now 0 of 480.
- `Range: bytes=-100` served the FIRST 100 bytes; a start past the end is now
  416. A CR/LF in an `/export/` name reached `Content-Disposition`.
- The multi-page contact sheet wrote non-atomically and ignored the session,
  and one unreadable frame failed the whole sheet. One builder now, atomic,
  skipping what it cannot read.
- Exposure comp `+1/3` parsed as zero. The fallback `heif-convert` had no
  timeout. The audio recorder's stderr was a pipe nobody read during a take.
- Live view ENDED after ten seconds of failed frames even while connected,
  and a browser shows a finished MJPEG `<img>` exactly like a live one
  (`complete` true, no event; measured in Chromium), so the page could never
  restart it. The stream now ends only when the camera is gone.

First run with the camera ON, portrait, same day:

- **A Nikon portrait frame is landscape-sized.** A real frame from this Z8:
  ImageWidth 8256, ImageHeight 5504, Orientation "Rotate 270 CW". The app
  compared the pair and told a turned camera to turn to portrait. Shape is now
  read as displayed (`actual_settings` swaps on the tag, `turn` rides along),
  guarded by `a_turned_body_is_portrait`.
- **Live view is sensor-oriented,** so a portrait body showed it sideways.
  `/api/state.liveTurn` carries the newest frame's turn and the page rotates
  AND refits the live view (a CSS turn keeps the landscape layout box). It is
  learned from a frame, so the first frame of a day fixes it.
- **Review mode.** Tethered, the stage was always live view, so editing never
  showed the big render. Clicking a frame, arrowing, culling or editing now
  reviews it on the stage; **Back to live**, `l`, Shoot or Focus return. The
  MJPEG stream is closed while reviewing.
- **The suite changed his camera.** The malformed-body fuzz POSTed to
  `/api/exposure-fix`, which really re-metered: M f/4.5 1/100 became A f/4
  1/30. The fuzz now skips it with a camera on, the run snapshots every
  settable value at the start and restores it (mode first) at the end, and
  checks it: `the camera is left exactly as the run found it`.

Colour and analysis review of a real portrait export, same day:

- **Auto landed skin too warm on every frame** (+42 to +51 red-blue against a
  +36 target, twelve real frames). Warmth was solved before brightening, and
  brightening scales red and blue together. A colour pass now measures the
  FINISHED look on the same fixed pixels and corrects warmth, then saturation
  (never below 0.85) if warmth is at its cap. Now +35.8 to +36.0 where the caps
  allow. An already-clipped source is also never brightened further.
- **Catchlights were reported missing when they were visible.** The Light
  report counted pixels over a fixed 235 on a 640 px copy; a flat frame tops
  out near 180. Now counted at full resolution as small NEUTRAL pixels well
  above the brightest skin: 119 on a frame with visible catchlights, 0-1 on
  six overhead-key frames.
- **Ranking measured the cheek, not the eyes.** Its patch was chosen for being
  70% skin, so it sat below the eyes, and a 6 px blur moved its focus score by
  0-11%. Ranking and the Light report now share one eye band; the same blur
  moves it 10-48%.
- **Rotate turns the live view only.** Frames are uprighted from EXIF, so the
  old Rotate turned correct portrait exports sideways.
- **The body's turn is remembered** (`.live-turn.json`) from the last real
  camera frame, so live view is upright before the first shot of the day.
- **The suite now lives in its own throwaway day** (1998-12-31). It used to
  run in today whenever today had a frame in it, and its cleanup deleted his
  empty `picks.json`. His session files are now compared by size and mtime
  before and after every run.

What the analysis says about the LIGHT (not fixable in code): the face is the
brightest thing in frame (top of histogram 174-195 even after grading), so a
headshot reads flat however right the colour is; the teal backdrop bounces
onto the skin nearest it; and the key gives a small catchlight. A brighter,
lower key in front, something white under the chin, and a light on the
backdrop are what would change it.

Follow-up, same evening:

- **An ungraded export is said, not blocked.** Above Export: "No grade on this
  frame" with **Auto-grade it**; under Whole session: "N of M frames have no
  grade" with **Auto grade them**. Silent for the copy-the-camera-file preset.
- **Ranking judges softness against the 90th percentile**, not the single best
  frame: one frame scoring 412 against a pack of 150-250 made half a sharp
  session "soft". On 29 real frames plus blurred copies: old 13/29 flagged,
  23-24/29 blurs caught; now 9/29 and 26-27/29.
- **Known limit, kept on purpose:** the eye band is proportional to the skin's
  height, so on a tank-top framing it can land on the forehead. A located eye
  line was tried three ways; every version traded blur detection or catchlight
  separation away. Documented at `eye_band()`.
- **Thumbnails needed no caching.** Measured: five strip rebuilds made zero
  thumbnail requests; the browser reuses an image already in the page.
- **`selftest.py` now runs `browser-audit.cjs`** and reports each of its 121
  results as a check, so the UI is verified behaviourally on every run.
- **Not yet verified: Webcam/Zoom mode with the camera on.** The Z8 left the
  USB bus (asleep or off) before it could be run.

Webcam/Zoom, verified with the camera live (2026-09-22):

- Tether -> Webcam 4.1 s; Zoom reads live 1024x680 frames from /dev/video10
  with the call look applied; Webcam -> Tether 7.4 s; camera settings
  identical before and after.
- **Webcam with no camera** waited 21.1 s before refusing; now 0.03 s.
- **The feed is never turned by the app (reverted at his request).** An
  automatic turn from the body orientation learned off the last photo was
  WRONG in real use: he turned the camera back to landscape for a call, the
  photo was hours old, and the feed arrived sideways. The Z8 reports no
  orientation over USB (none of its 55 config nodes). He rotates in OBS.
  `z8-webcam.sh` is back to its original filter chain.
- A consumer holding `/dev/video10` (OBS, Zoom) freezes its SHAPE. After a
  portrait feed, a landscape one is letterboxed inside the old portrait frame
  until the app releases the device: pick another device in OBS, press Off
  then Webcam, pick Nikon Z8 again.
- **A feed started outside the app** (the GNOME launcher) left the app holding
  a dead camera session: Tether said "connected" and every settings read came
  back empty until Off then Tether. Tether now drops the stale session, the
  watcher drops it as soon as a feed appears, and state no longer reports
  "connected" while a feed holds the camera.
- `z8-webcam.sh` now lives in this repo; `~/bin/z8-webcam.sh` is a symlink to
  it, so the launcher and the server find it where they always did.

## The webcam never goes dark (z8-feed.py, 2026-09-22)

Built for an hour-long podcast. `z8-feed.service` (a user unit, enabled at
login, `Restart=always`) runs `z8-feed.py`, which owns `/dev/video10` for the
whole session through ONE ffmpeg writer and never lets it stop:

- **Webcam** in the app writes `.feed.json` live=true; the keeper opens the
  camera (mtplvcap) and sends live frames, always 1024x576 (16:9).
- If frames stall it shows a **"Reconnecting the camera..."** card over the
  last frame, restarts the link, and resets USB if the link is wedged.
- **Off / Tether** write live=false; the keeper releases the camera with
  SIGINT (never harder: that wedges it) and shows an "in use for photos" card,
  so OBS and Zoom still see the device.
- The reader keeps only the NEWEST frame and a pump sends it: under load the
  feed drops frames instead of queuing them, so delay cannot build up.
  Measured with all 12 cores at 100%: 867 read, 719 sent, frame age 0.01 s.

Measured with OBS attached throughout, OBS never lost the device:

| Fault | Result |
|---|---|
| camera link crashes | live again in 7.6 s |
| device writer killed | restarted invisibly, never left live |
| app restarted | no effect |
| keeper killed | systemd restarts it, live in 16 s |
| camera USB link wedged | two failed opens, USB reset, live in 18 s |

**Latency: no ffmpeg in the live path.** Measured by frame order, ffmpeg held
every frame 80-117 ms (three frames always inside it, however the input was
framed or threaded), which was most of the lag he felt. The keeper now decodes,
grades (the call look's own numbers: brightness, contrast, saturation, warmth,
vignette, luma-only sharpening) and writes limited-range I420 to the device
itself: about 22 ms a frame, and 30% of a core where ffmpeg took 100%. Colour
checked by decoding the output back. ffmpeg remains the fallback if the device
cannot be opened for direct writing.

Traps found building it: mtplvcap answers with chunked encoding (read through
HTTPResponse, not the raw socket); a zombie mtplvcap still has its name in
/proc; a keeper finds its link BY NAME, so each keeper only manages the link
on its own port (a test keeper must never SIGINT the real one). Tested by
`the_feed_keeper_never_lets_the_webcam_go_dark` against a fake camera.

OBS/Zoom opened before any feed existed will not find the device; with the
keeper running from login that cannot happen any more.

`config.json`, `camera-presets.json` and `.last-mode.json` are runtime state
and ignored by git.

`browser-audit.cjs` drives all of this in Chromium with a real mouse and
keyboard (111 checks plus a benchmark), in a throwaway session of copied real
frames, and puts everything back:

    Z8_PLAYWRIGHT=/path/to/node_modules/playwright node browser-audit.cjs

## Panels

**Camera card** lists what is on the card and imports anything not already
pulled in, which is how a skit shot without the laptop gets here. Re-importing
is idempotent because the camera's own filename is kept.

**Clips** shows video with a poster frame taken a second in, since frame zero is
usually a fade and every clip then looks the same. Clicking one opens a player
that supports range requests, so a long take scrubs instead of buffering whole.

**Session** browses any past day. Captures always land in today even while an
older shoot is open. Pick and reject with `p` and `x`; the selection advances
automatically so a shoot can be culled at speed. Rejected frames are never
rendered on export.

**Histogram** is drawn from the graded frame and calls out blown and crushed
percentages, because a blown forehead cannot be recovered and is hard to see on
a small preview.

**Crop** double-clicks into a 1:1 loupe pulled from the full-resolution file.
The live view is 1024x576, so focus genuinely cannot be judged from it.

**Output** sets a destination folder and a filename template (`{stem}`,
`{preset}`, `{date}`, `{w}`, `{h}`). Point it at a thumbnail folder and exports
land there directly.

## The day rolls over

The shutter always writes to TODAY and the strip shows whatever day you
browsed. Both are right on their own and a trap together: browse yesterday to
compare something, press Shoot, and the frame lands in today's folder while the
strip still shows yesterday. It looks like the capture failed. It matters most
the morning after a late session, when the date has changed and the app is
still showing the night before.

A warning now sits **beside the shutter**, not in the session panel, because
the mistake is pressing Shoot and that is where the eye is. It names the day
being shown and offers one click back to today. Pressing Shoot while browsing
an older day jumps to today rather than hiding the frame: pressing the shutter
is not a request to add to yesterday.

And a new day's empty strip says where the work is, rather than "No shots yet":
it names the most recent day that has frames, how many, and offers to open it.

## Portrait, and the orientation tag

**Shoot headshots in portrait.** An acting headshot is 8x10 portrait, and a 4:5
crop out of a landscape frame throws away both sides. Measured on this camera:

| | pixels kept in the finished 8x10 |
|---|---|
| landscape 8256x5504 | 4403x5504, **24.2 MP, 53%** |
| portrait 5504x8256 | 5504x6880, **37.9 MP, 83%** |

Turning the camera is worth **56% more resolution** for nothing. The app says so
in the Camera panel whenever the last frame was landscape, and stops saying it
the moment you turn.

**The camera does not rotate pixels when you turn it.** It records the sensor as
it is, 8256x5504, and writes an EXIF Orientation tag saying which way is up.
Every reader is expected to honour that, and this app did not: the moment the
camera was turned, every frame would have displayed, graded and EXPORTED on its
side.

It never showed in months of use because a landscape session carries
Orientation 1, where the transform is the identity, and every frame up to that
point had been landscape. A default that hides a missing step.

`upright()` applies it, in `load_editable`, which is the one function every
reader goes through, so the whole app sees an upright frame and the Rotate
control stays what it always was: the user's own extra choice on top. A
malformed tag returns the frame unchanged rather than costing it. Verified with
a real Orientation 6 fixture: 8256x5504 on disk, exports 5504x8256, thumbnails
240x360.

## Three files, and a type check

The page was one 2,938 line file with the CSS and the whole application
inlined. It is `index.html` (281 lines of markup), `app.css` and `app.js` now.
Not a build step: the browser fetches all three from the server exactly as
written, and there is nothing to compile.

Splitting is what makes the type check possible, because a checker cannot be
pointed at a script tag. `app.js` carries `// @ts-check` and
`npx -p typescript tsc -p tsconfig.json` runs it with `checkJs`, `noEmit` and
the DOM lib. The suite runs it too, and skips honestly if npx cannot fetch
TypeScript.

It found 13 real errors on the first run, all the same shape: `querySelector`
returns `Element`, which has no `.value`, `.checked`, `.hidden`, `.dataset` or
`.click()`. Typing the one `$` helper fixed most of 457 call sites at once; the
rest were `e.target`, which is an `EventTarget`. Zero errors now.

## Nothing is defined and never called

A function nobody calls is either dead weight or a **lost call site**, and here
it was the second. `await_conversion` makes a reader wait for the background
HEIF conversion instead of opening a file that is still being written. A later
edit adding EXIF orientation rewrote the tail of `load_editable` and dropped
both that call and the sibling reuse beside it.

Nothing failed. The window is about four seconds after a capture and no test
clicked a frame inside it, so the only symptom was the duplicate-conversion bug
coming back: 4 seconds and 7 MB per HEIF frame, silently.

The suite audits it now. Every function in `server.py` and `app.js` must be
called, with an allowlist for the legitimately uncalled: thread targets,
dispatch methods the HTTP base class invokes, and closures. 128 functions and
46 respectively, none orphaned. The two specific call sites that were lost are
also asserted by name, so a future rewrite of `load_editable` cannot quietly
drop them again.

## Derived files do not outlive their frame

Four kinds of file get made beside a frame and only two were ever cleaned. A
thumbnail has a pruner and a video poster has one. A raw develop cache
(`.dev.jpg`) and a HEIF conversion (`.conv.jpg`) did not, and both are
full-size JPEGs of several megabytes that outlived their source for ever.

They are pruned on every listing, keyed on the stem like everything else here,
and only when no live frame claims one. A frame restored from the trash simply
regenerates its derivatives, which is the contract the thumbnail cache has
always had.

A conversion is also **redundant once its frame has a real `.jpg` beside it**,
which every tethered capture now writes, because the reader reuses the sibling
instead of converting a second time. Measured at 6.7 MB each. It is kept only
when there is no sibling, which is how a `.heic` imported from the card stays
readable at all.

## Tidy exports

Exports of frames you have since deleted pile up silently: one session ended
with 123 of them mixed in with 40 real ones, and it was cleaned by hand twice
before it became a button.

**Tidy exports** finds them and moves them to the trash. It is a **dry run
first, always**, and shows you the list, because it decides on a filename. A
frame sitting in the trash counts as present, since it is one button from
coming back. An export whose name does not match `{stem}_{preset}` is reported
as *unknown*, never as an orphan: a changed name template must not read as
"delete this".

It moves rather than unlinks, and falls back to a copy across filesystems.
`os.replace` cannot cross one, and the exports folder is configurable precisely
so it can live on another drive: measured with exports on tmpfs and the session
on nvme, tidying failed outright with "Invalid cross-device link".

## Selecting several frames

Click a frame to select it. **Shift-click** another and the whole run between
them is marked. **Ctrl-click** (Cmd on a Mac) adds or removes one. The export
button then reads "Export 9 selected frames" and runs them as a batch.

Every frame keeps its OWN saved grade and crop. That is the point: a crop
framed on one face is wrong on the next, so the sliders showing right now apply
only to the frame being graded, not to the selection.

Marking is separate from selecting on purpose. You mark ten to export and still
edit one of them; `state.sel` is the frame being graded, `marked` is the batch.

## Full frame means the whole frame

Two things cropped independently and neither was obvious, reported as
"everything seemingly is getting cropped", which it was.

**The button lied.** It set the crop to `defaultCrop()`, which returns the
whole frame only when *Lock to preset* is off, and that defaults to ON. So it
handed back the largest rectangle of the PRESET's shape and called it full
frame.

**And the export cropped again.** A preset with `fit: cover` fills its target
size and discards the overflow, so even a cleared crop came out cropped.

Full frame now releases the lock, clears the crop, and switches the export to
not crop. There is an explicit **"Whole frame, do not crop"** checkbox as well,
and all three export paths honour it. Measured on an 8256x5504 source into the
casting-profile size: `cover` gives 1200x1600, cropped to 4:5; `contain` gives
1200x800, the source 3:2 kept whole.

## A test can never shoot into your session

Several sections of the self-test fire the real shutter, so a run with the
camera attached used to mix test frames into the working session. That is not
untidiness. On 2026-09-21 it led to sixteen of the user's own frames being
deleted as suspected test captures, and only the recoverable trash saved them.

The fix is not better cleanup, because cleanup means deciding whose frame it is
after the fact, which is the mistake. `capture_dir` redirects where a new
capture lands, the suite points it at a temporary directory for the whole run,
and a section that finds it NOT redirected refuses to fire the shutter at all
rather than risk it. Verified: a full run with the camera attached left the
session at exactly 29 frames.

Deliberately a separate setting from `export_dir`. A capture and an export move
for different reasons, and redirecting one must never move the other.

## Export quality, measured

The JPEG encoder is not where quality goes. Measured on a real 8256x5504 frame
against the same pixels saved at quality 100: **0.94 of 255 mean error at
quality 92**, 0.70 at 97. Invisible either way.

Neither is the resize. The export path does a full decode with no drafting and
one LANCZOS step, and a hand-rolled single-step resize with Pillow's
`reducing_gap` shortcut disabled produced **identical detail, 28.7 for all
three**. There is nothing left to recover there.

What a small export loses is the downscale itself: a casting profile is
1200x1600, which is 3% of the frame's pixels. That is what the size means. Raw
would not change it either: NEF buys grading latitude, not resampling or
compression, and tethered NEF does not work here anyway.

**Every preset was leaving headroom unused**, though, and that is worth having.
casting-profile encoded at 269 KB against its own 2048 KB ceiling, which is a
casting site's limit and not a target to undershoot sevenfold. Ten presets
raised, every one still inside its cap:

| preset | was | now | KB | ceiling |
|---|---|---|---|---|
| headshot-8x10 | 95 | **97** | 2005 | none |
| casting-profile | 92 | **96** | 472 | 2048 |
| web-2k | 90 | **95** | 555 | none |
| agent-email | 90 | **94** | 301 | 500 |
| x-post | 90 | **94** | 259 | none |

For anything you look at yourself, use **headshot-8x10** or **web-2k**.
casting-profile is small only because casting sites reject big uploads.

## Panels are grouped by what they are for

**Record audio**, **Check audio** and **Fix exposure** used to sit under a
heading called "Camera card", which they have nothing to do with. They are now
**Sound**, **On a call** and **Camera card**, each with a line saying what it
is for.

The takes list also built an HTML5 player per take. Four takes, three of them
accidental 0.6 second recordings, ate 200px of the sidebar and pushed the
histogram and the crop below the fold. A take now shows a **Play** button and
builds its player only when pressed, one at a time.

## Rank: which frames to keep

**Rank** in the session panel scores every frame on the three faults that make a
headshot unusable, worst fault first, and names each one in words. It never
changes a pick: culling is the photographer's call, and a tool that silently
rejected frames would be trusted exactly once.

- **Focus**, first, because it is the only unrecoverable one. At 100% a soft
  frame has no pore detail at all.
- **Exposure** and clipping. A blown forehead does not come back.
- **Whether the eyes are lit** at all.

The whole difficulty is WHERE it measures, and getting that wrong produced
confident nonsense twice.

**Hair outscores a sharp eye.** Hair carries more high-frequency detail than
anything else in a headshot, so a focus box that lands on a fringe beats one
sitting on a perfect iris. Measured on 67 real frames: the top-ranked frame
scored 1075 with its box entirely on hair, against 991 for a frame whose box
was squarely on a sharp eye. The ranking was a hair detector. Demanding 25%
skin in the box was not enough on a subject with a fringe, where a band that is
half forehead still scores off the hair: the same frame came back at 48% skin
and 1566. The box now has to be **70% skin**, slides down the head until it is,
and the frame is refused outright below 45%. On this session that measured 62
of 67 frames with a median of 92% skin in the box, and the five refusals were
the HLG frames that were too desaturated to find a face in at all.

**A focus number scales with contrast**, so a darker frame scores lower at
identical focus. The patch is normalised to a fixed mean first.

Focus is judged RELATIVE to the best frame in the same set, never against an
absolute. There is no absolute: it depends on the lens, the subject, the
framing and the light, and a threshold that suits one sitting calls every frame
of the next one soft. Verified differentially, which is the only comparison a
badly placed box cannot fool: a deliberately blurred copy of the best frame
scored 452 against its own original's 984, and was called soft.

## The zoom, and dropdowns that stay open

The panel shows **Zoom** in millimetres, live and read-only, tracking the ring.
It comes over USB and the panel simply never asked for it. For a portrait that
is not a detail: 24mm and 120mm on the same lens are different pictures of the
same face. 100 to 120mm is the range to stay in.

**Dropdowns used to close the instant they opened.** Rebuilding a `<select>`'s
options destroys the node the browser is showing the popup for. The page
refreshes several lists from `/api/state` and reloads the camera panel every 15
seconds, so any of them could do it under the pointer. `selectBusy()` is the one
helper that answers "the user is using this", and every rebuild consults it. It
tracks `mousedown` as well as focus, because a click that opens a popup is not
always a focus event first.

## Exposure compensation that has stopped working

It biases a **meter**, and M mode with Auto ISO off has no automatic parameter
left for it to bias. Measured: frames at -3, +1 and +3 came back at 101.2, 101.2
and 101.3 mean brightness with the ISO never leaving 320. The panel was showing
+1, which had genuinely mattered an hour earlier while Auto ISO was on.

A control that USED to work and silently stopped is worse than one that never
did, so the panel says so and names what to use instead. It is deliberately not
warned about with Auto ISO on, in an automatic mode, or at a compensation of
zero, because in all three it is doing its job.

## The call look is its own setting

The Webcam button used to send `adj()`, the EDIT sliders, as the feed's look.
So grading a dark frame to exposure 1.8 with a vignette and then joining a Zoom
call sent exactly that out, live, to whoever was on the other end. How you look
on a call should not depend on which photograph you were editing.

The selector next to the Webcam button chooses it, and it is saved in the
config. **Clean headshot** is the default. "No look" still applies the full to
limited range conversion, which is the thing that stops Zoom crushing your
shadows; only the grade is dropped. "Follow the edit sliders" is there for when
the old behaviour is what you actually want, and only the page can honour it,
because only the page knows where the sliders are.

Changing the selector updates a running Webcam feed immediately, without
reopening USB or restarting the output. "No look" removes the current grade;
"Follow the edit sliders" also updates as you edit. Outside Webcam mode, the
selection is saved for the next call. A failed update reports the error and
returns the selector to its last confirmed choice.

This is a software look for **Zoom/OBS**, not a camera preset. It does not
change ISO, shutter, aperture, white balance, the Tether preview's separate
edit-slider approximation, or movies recorded onto the camera's card. The
camera's physical photo/video switch does not enable Webcam mode in the app.
Use the Camera settings panel for the camera's exposure settings.

The running look is also saved in the mode marker, so a service restart
restores the latest choice. Previously the dropdown only saved configuration:
an already-running feed kept its old look until a mode restart.

## Export refuses to race the grader

The export batch pins the edits when it starts, which is what keeps one export
self-consistent. Start it while **Auto all** is still working and it pins a
half-finished set.

Measured on 81 frames: every frame ended up graded in `edits.json`, and the
exports came out with a **110-level brightness spread**, because most were
rendered from the neutral grade the batch had captured a second earlier.
Nothing on screen said so, which is the part that matters.

So an export started while a grading job runs is refused, with the reason and
what to do about it. Nineteen seconds of waiting beats a silently wrong set.
Both jobs running together is otherwise safe: measured 81/81 each, zero errors,
`edits.json` still valid, because `save_edits` re-reads under its lock rather
than writing a stale snapshot back.

## Auto all: one correction per frame

**Look to all** copies one grade across the shoot. That is right when a hero
frame sets the look, and wrong when the frames differ. Measured on a real
66-frame sitting: face brightness ran from **52 to 141**, because the subject
moves relative to the key light. No single exposure correction is right for
that set.

**Auto all** measures every frame and corrects each one on its own. Measured on
those 66 frames: 19 seconds, 0.28s a frame, **59 distinct exposure values**, and
the face spread across the set fell from **57.6 levels to 16.2**. That spread is
the difference between a set and a pile of photographs, and it is the thing a
casting director notices before anything else.

It never touches a crop or a straighten. Those are geometry, and a crop framed
on one face is wrong on the next.

## The export path applies the look in two phases

`apply_edits` splits into the per-pixel operations and the two that depend on
neighbouring pixels. The export runs the point operations at full resolution,
fits to the output size, then runs the spatial ones. Faster and more correct:

- **Sharpening a 8256x5504 frame and then downscaling it seven times throws the
  sharpening away.** The added edge contrast is averaged out by the resize.
  Measured on four real frames, moving it after the fit raised high-frequency
  detail in every one, 369.9 to 402.4 on the largest. The old order paid 1.11
  seconds a frame for an effect the next step deleted.
- The preview has always applied everything at preview size, so the export used
  to deliver LESS sharpening than the preview promised. Now they agree.
- A real 66-frame batch went from about 170 seconds to **83 seconds**, 1.26s a
  frame, zero errors.

The point operations stay at full resolution, where they are exact. Moving them
too was measured at a mean difference of 1.16/255, which is not free.

## A vanished exports folder

A configured exports folder that has GONE is not a folder to keep using: an
external drive gets unplugged, a synced folder moves, a temporary directory is
deleted. `save_config()` creates the folder when it is CHOSEN and refuses the
setting if it cannot, so anything in the config existed once and a later absence
is a disappearance. Exports fall back to the session's own folder and say so.

`export_dir()` deliberately does not create anything. A first attempt guessed
from whether the parent directory existed, and cheerfully recreated a deleted
`/tmp` directory, which is the failure it was written to prevent. Creation
belongs to the one place that has the user's intent.

## Saved camera settings

The Camera panel has a **Restore / Save** row. Save names the camera's current
settings; Restore writes them back. Measured: a full restore of seven settings
takes 0.3 seconds and every value comes back exactly.

It stores only what the camera will actually accept over USB. `focusmode` reads
back fine and is `readonly=True`, so a preset containing it would report a
failure on every restore through no fault of yours, and `battery` is a reading,
not a setting. Both are excluded.

A restore reports **each setting separately**. One verdict for eight settings
hides which one is stuck, and the camera declines writes for reasons that have
nothing to do with this app: the mode dial, a menu option, or a value the
current mode does not allow.

The preset that produced the good frames on 2026-09-21 is saved as **Headshot**:

```
M, f/4.5, 1/100s, ISO 320, exp comp +1, JPEG Fine*, Auto WB
```

What a preset CANNOT hold, because the Z8 does not expose it over USB at all:
tone mode, picture control, colour space, Auto ISO, and RAW compression. Those
are camera menu settings, and the app can only measure a frame afterwards and
tell you they are wrong. See the HLG section below for why that matters.

## HLG, the one that cost an evening

**Every file was 10-bit HDR.** The camera was in HLG tone mode and the metadata
said so all along:

```
TransferCharacteristics : BT.2100 HLG, ARIB STD-B67
ColorPrimaries          : BT.2020, BT.2100
PictureControlName      : Standard(HLG)
```

HLG is for HDR televisions. Rendered to 8-bit sRGB it is flat and desaturated by
construction, and no grading recovers it, because the tone curve and the gamut
are both wrong. It also forces a minimum ISO of 400, adds noise, disables
Picture Controls entirely, and makes the camera write HEIF, which costs 4
seconds and a gigabyte of RAM to convert per frame.

The fix is `G` > Photo Shooting Menu > **Tone mode** > **SDR**, on the camera.
Measured immediately after, with no software correction at all: skin red-blue
went from +24.5 to **+34.5**, green from +1.5 to **-1.5**, and the darkest
pixels from 20 to **4**, which is the first time these files had real blacks.

The app now reads the colour metadata from the last frame and says so, with the
menu path, because saying so is the only thing it can do.

## Focus, and what the camera will and will not let this app do

**Every frame refocuses before the shutter**, on by default, toggled with
"Focus first" next to Frames. Focus is the one unrecoverable mistake: exposure
and colour are fixable afterwards, and at 100% a soft frame has no pore detail
at all. A burst focuses on the FIRST frame only, because re-driving the lens
between frames of one pose costs half a second each and can hunt. A focus that
does not confirm never costs the frame: it shoots anyway and says so in the
banner.

**Honest limit, so nobody re-derives it.** The gain from driving autofocus over
USB could not be measured on this Z8. Six interleaved frames with it on and off
scored 128.1 against 136.6 on a focus measure, and repeating with live view
held open gave 130.7 against 135.6. The request is sent and the camera reports
success. Whether it acts on it is the camera's business. The camera's own
half-press, and Custom Setting a1 (AF-C priority selection) set to Focus, are
the controls that definitely work.

**Manual focus and the focus point are API only.** `POST /api/focus`
`{"steps": -500}` turns the focus ring by whole lens steps (libgphoto2
`manualfocusdrive`): minus focuses closer, plus further away, from -32767 to
32767; 0, a fraction that rounds to 0, a string or anything past the range is a
400. It works with live view on and in AF-S/AF-C, measured on 2026-10-06: +3000
then -6000 took the live view sharpness from 230 to 2.6, and `/api/af` brought
it back to 280. Shoot with `{"focus": false}` after a manual drive, or the
autofocus before the shutter undoes it. `POST /api/af-area` `{"x": 512, "y":
288}` moves the focus point to that live view pixel (libgphoto2
`changeafarea`); a missing, negative or non-number x or y is a 400. The camera
accepts it (`{"ok": true}`) but in auto-area AF it has no visible effect; set
the Z8's AF-area mode to single-point or wide-area for it to act (not yet
verified). Both answer 503 "busy" like `/api/af` when the camera is held.

**A setting write is now read back.** libgphoto2 returns success for a write the
camera then declines. Setting focusmode to AF-S returned `{"ok": true}` and the
camera stayed in AF-C, which sent the whole autofocus investigation above
chasing the wrong variable for twenty minutes while the UI displayed AF-S. Now
a declined write fails with the value the camera actually kept, and says that
some settings are locked by the mode dial or by a menu option.

**The Camera panel now tells you when a control is not in control.** Under the
sliders it prints what the last frame ACTUALLY used, read from its EXIF, plus
the lens and the shutter count, and an amber notice for any control the camera
is overriding. There is no PTP property that reports this, so a written frame is
the only honest source. The notice says what to use instead, because the bare
fact left this exact problem undiagnosed for twenty minutes.

**Auto ISO makes the ISO and aperture fields cosmetic.** With Auto ISO on, the
camera holds a metered target, so changing aperture or ISO moves the other one
and the brightness does not budge. Measured on 2026-09-21: the app showed ISO
640 while the camera shot at 1400, 900, 800 and 560 across four frames, and a
full stop of aperture produced no change in face brightness at all. Exposure
compensation is the only live exposure control in that state, and it works: at
+1 stop the face landed at 131.7 against a 138 target across three consecutive
frames within 0.6 of each other.

**A measured focus number is not a focus verdict.** Two traps, both of which
produced confident nonsense here. A Laplacian variance scales with contrast, so
a frame half a stop darker scores lower at identical focus. And a crop box aimed
at "where the eyes are" lands on an eyelash in one frame and a forehead in the
next, and forehead skin has less high-frequency detail than an eyelash whatever
the focus is: that alone reported 13 of 23 sharp frames as soft. Crop at native
resolution and LOOK at it before saying anything about focus.

## A batch has one destination

The exports folder is configurable, and `export_dir()` reads the config, which
a batch was re-reading on every single frame. Change the folder while two
hundred frames are running and half the batch lands in each. Measured the other
way round on 2026-09-21: a test lifting a redirect for one second sent thirty
frames of an in-flight batch into the user's own folder. A batch now pins its
destination when it starts, the same way it already pinned the session. Proven
with 41 frames: the folder was switched mid-run and 41 of 41 landed in the
folder the batch started with, 0 in the new one.

`/export/<name>` had the same class of bug from the other side: it joined
`session_dir()` and `"exports"` by hand, so with a custom exports folder set
every download from the page 404'd while the file sat on disk exactly where it
had been asked for. It resolves through `export_dir()` now, and still falls back
to the session's own folder so a link to something exported before the folder
changed keeps working.

## Auto

**The Look select starts on "As shot", which applies nothing.** That is the
correct behaviour for a preset called "as shot" and it is also a trap: an
export taken straight out of the box carries no correction at all, and looks
it. The 2026-09-21 headshot was flat and grey for exactly that reason, not
because the export was broken.

**Auto**, next to the Look select, measures the frame and works out the
correction. Every value it sets is solved from a measurement and shown to you
in words underneath, so you can check it rather than trust it. On that frame:

```
face measured 93, wants about 138: +0.57 stops
skin red-blue spread +17, wants about +36: warmth +0.30
white balance alone reaches +27: saturation 1.30 for the rest
green sits +1.0 above the red/blue average: tint +0.22
the file uses only 12..156 of 0..255: contrast 1.22
added contrast would crush the low end: shadows +0.15 to hold it open
the whole look delivered 141 rather than 138: exposure corrected to 1.45
```

How it decides, and why each part is the way it is:

- **It meters the skin, not the frame.** A 4x5 crop against a grey backdrop is
  mostly backdrop. That frame's whole-frame average was Y 76.5 with R 76.1 /
  G 76.8 / B 76.0, which reads as a perfectly balanced mid grey, while the face
  inside it sat at 81.9 with a red-blue spread of +10.4: a stop dark and
  visibly cold. Skin under any light keeps R > G > B, so that is the mask.
- **The mask is chosen once and then followed.** Warming an image makes MORE
  pixels pass "R > G > B", so hair edges and the shadow side of the neck join
  the set and its average falls. A correction loop chasing the re-derived
  average drove the real cheek to 157 while reporting a perfect 138.
- **It corrects for the fact that the stages compound.** Contrast steepens the
  curve around mid grey and warmth pulls blue down, so an exposure solved to
  land the face at 138 actually delivered 121.5 once the rest of the look ran.
  Auto renders the thumbnail, measures the result and takes a corrective step.
  Open-loop numbers that are each individually right still add up to a wrong
  picture.
- **White balance is capped at 0.30 and saturation finishes the job.** Warmth
  moves every pixel, backdrop included, so a large warmth correction turns a
  grey studio wall brown. Saturation scales chroma, and a neutral pixel has
  none, so it warms skin and leaves the wall alone.
- **It will not brighten into the ceiling.** The highlight test reads the
  GRADED frame, never the original: the raw file's p99 of 156 scaled by a trial
  exposure crossed 252 and aborted a correction that the graded frame it stood
  in for had 0.000% of its pixels at 255 and a full stop of room left.
- **A correct frame comes back with nothing to say.** Measured: a frame pushed
  3.2x over is told to darken to 0.847, a frame at 0.45x is told to brighten to
  2.2, and the same file always returns the same answer.

Measured on the real 2026-09-21 export, against an independently sampled
cheek-and-forehead box the algorithm does not use:

| | as shot | after Auto | target |
|---|---|---|---|
| face brightness | 90.3 | 135.5 | 138 |
| skin red-blue spread | +12.8 | +40.0 | +36 |
| pixels in the top 5 levels | | 0.000% | near 0 |
| pixels crushed to black | 1.81% before the shadow lift | 0.171% | near 0 |

Auto is a starting point, not a verdict. It writes the ordinary sliders, so
every one of them is still yours to move, and `Hold to compare` still shows
the original. It never touches the crop or the straighten, because those are
geometry, not a look.

Two things it cannot fix, because they happen before the file exists: a frame
shot at 1/1000s in M mode is dark because the camera was told to make it dark,
and `Quality: JPEG Normal` on the Z8 writes HEIF. Both are camera settings and
the Camera panel sets them.

## Editing

Sliders render server-side through the same code path as the export, so the
crop panel shows the real result. Measured drift between preview and full
export: **0.15/255**.

`shadows` and `highlights` are tone curves weighted to one end of the range, so
they open the shadow side of a face without blowing the forehead. Contrast
moves both ends at once, which is why it is the wrong tool on skin. Measured at
±0.8: shadows move the 5th percentile by +78 and the 95th by +17; highlights
move the 95th by 23 and the 5th by 0.

`warmth` is blue/amber and `tint` is green/magenta. Together they are a white
balance. `vignette` uses a cubic falloff so the centre is untouched: at 0.9 the
centre drops 0.0 and the corners drop 11.7.

Every frame keeps its own grade and crop, so moving between shots does not
carry the last one's look. `c` and `v` copy a look between frames, "Look to
all" pushes one across the shoot. Crops never travel, because a crop framed on
one face is wrong on the next. `ctrl+z` undoes.

Raw develops through `darktable-cli` into a cached JPEG beside the file. It
uses its own config dir so it never fights an open darktable.

## Grading a set

Grade the hero frame, then **Look to picks** pushes that look across the picked
frames only, or **Look to all** across the whole session. The look travels, the
crop never does: a crop framed on one face is wrong on the next.

`/api/copy-edits` is **API only, on purpose**: the buttons cover the common
cases and it exists for scripting. That is recorded in the self-test, because an
endpoint nobody calls is exactly how this one stayed broken unnoticed, and the
suite now flags any *other* route that becomes unreachable.

`/api/copy-edits` copies one frame's grade to a named list of others. It had
never worked: handed a list it raised `cannot use 'list' as a dict key` and
returned a 500. Nothing caught that because no caller in the UI reached it, and
it only surfaced when a full workflow run exercised the API directly.

## Presets for acting work

The first four export presets are the ones the job actually asks for:

| preset | what it is for |
|---|---|
| Actor headshot, 8x10 print | 2400x3000 carrying **300 dpi**, so a lab prints 8x10 and not some other size |
| Casting site profile | 1200x1600 held under 2 MB, because Actors Access and Backstage refuse an oversized upload rather than resizing it |
| Self-tape slate frame | 1920x1080 |
| Email to an agent | long edge 1600 under 500 KB |

A preset with `max_kb` walks the JPEG quality down to meet the ceiling, and if
the quality floor still is not enough it shrinks the image rather than hand
back a file that gets rejected on upload. Add `dpi` to any preset to stamp
print resolution.

## Exporting a whole shoot

**Export all** renders every frame in the session with each frame's own edits,
falling back to the sliders for frames never touched. **Picks only** narrows it
to flagged frames; rejected frames are always skipped, so a cull costs nothing
to honour.

The session is pinned when the batch starts, so browsing to another day while
it runs is safe. It was not always: `session_dir()` reads a global, and
switching mid-batch failed 15 of 25 frames with "no such shot", wrote one
export into the other day's folder, and still reported "done".

Every job runner is wrapped so it **must** reach a terminal state. Each one
caught only the exception types it expected, and the card import had no `try`
at all, so anything else escaped and skipped the finish: the job stayed
"running" forever while the page polled it. Proven by injecting a `KeyError`
into a runner, which now lands as `state=failed` within half a second with the
exception reported.

It runs as a background job, not inside the request, because forty raw frames
take minutes and a browser gives up long before that. The button becomes a live
counter and a **cancel**: press it again and the batch stops at the frame it is
on rather than finishing work you no longer want.

## A thumbnail out of a video

Open a clip, scrub to the moment, **Grab frame**. The still lands in the session
as an ordinary shot, so it grades, crops and exports through the same presets as
anything off the camera: scrub, grab, punch the look, export at 1280x720.

The seek is accurate rather than keyframe-snapped, so you get the moment you
stopped on. On a clip that is green for four seconds then red, a grab at 5 s
really is red.

## Importing from the card

**List card** walks the card and lists files as it goes, so the 400 file cap
bounds the USB round trips and not just the result; it used to build the whole
folder tree first, holding the camera lock the entire time, which blocks live
view and the shutter. There is a 600 folder ceiling and cycle protection, and a
listing that stopped early says so rather than presenting a partial card as the
whole card. One unreadable folder no longer loses the rest.

**Import new** runs as a job too, with a live count and a cancel, because ten
4K takes is minutes of USB: inline it meant a browser timeout, no progress and
no way to stop. The disk is re-checked between files, not just once at the
start, so a long import cannot quietly fill the drive.

Verified against a stub camera: 200 files in 24 calls normally, 25 files in 7
calls with a limit, a cyclic tree terminates, and a folder that throws still
returns what was found.

## Recording

**Record** starts and stops the camera's own movie recording, so you can shoot a
skit without walking back to the camera. The Z8 only accepts this in **video**
mode.

The button follows the **camera**, not the page. It used to be a page variable,
so reloading mid-take showed "Record" while the camera kept rolling, and the
next press sent start again instead of stopping: the take could not be stopped
from the app. Verified without hardware only in part, the flag logic and its
clearing on release; the round trip through the camera still needs a real take.

## Trimming a self-tape

Click a clip to open the player, then **Set in** / **Set out** (`i` and `o`)
and **Save trim**. The cut lands in the session's `exports/` folder beside the
stills, numbered rather than overwritten, so several takes off one recording
are fine.

After the cut, **Play the cut** opens it in the same player. Checking the cut
before sending an audition tape is the point of making it, and the file lives in
`exports/`, which the clip list deliberately does not show; it used to 404 and
mean a trip to the file manager.

**Precise is the default and re-encodes**, which is what makes it land on the
frame you chose. **Fast cut** copies the stream instead: instant, but it can
only cut on a keyframe, and on the Z8's long GOP that missed a 3.5 s request by
2.1 s in testing. Use it only when the exact frame does not matter.

## Deleting is recoverable

**Delete shot** moves the frame and every representation of it (the HEIF
original, the working JPEG, any raw develop cache) into `.trash/` inside the
session, and **Undo delete** puts it back byte for byte. A restore never
overwrites a live frame that has taken the same name.

**Empty trash** is the only action in the app that cannot be undone, so it asks
once before doing it. Nothing empties it automatically, on purpose: deleting
frames to save disk is the wrong trade. The button carries the count and the
size (`Empty trash (42, 1.8 GB)`) so it cannot quietly grow unnoticed.

A full disk is caught before anything is written, and a **batch stops at the
first frame** rather than producing one identical error per frame. The
threshold is 300 MB, overridable with `Z8_MIN_FREE_MB`, which is also how the
self-test drives the real full-disk path.

## Zoom and OBS

Webcam mode hands the camera to `/dev/video10` and applies the current look on
the way through, so a call gets the same grade as an export. The mapping can
only be close, because ffmpeg's brightness is additive where the export path's
exposure is multiplicative. Exports stay exact.

**Measured, not assumed:** across all six looks the worst channel difference
between the Zoom feed and the export is **6.6 of 255, about 2.6%**, on
`bright-flat`. The additive factor is 0.45, chosen by sweeping it against the
real export pipeline (8.6 at 0.35, 6.5 at 0.45 and 0.50, 7.0 at 0.60). An `eq`
gamma of 1/exposure looks like the principled answer and is far worse at 20.0.
The self-test holds the drift under 7.

**Every look now carries the full-to-limited range conversion itself.** `Z8_VF`
REPLACES the webcam script's whole filter chain, so a look that omitted it
silently put the crushed-shadow colour bug back: the camera sends full range
(Y 0..255), v4l2loopback carries no range metadata, and OBS and Zoom assume
limited (16..235). Measured on a full-range JPEG, the source shape the camera
sends: Y 12..231 without the conversion, inside 16..236 with it, for all six
looks. `format=yuv420p` alone does NOT do this, it only relabels.

Add `RANGE=full` to `z8-webcam.sh` to keep every code value instead, and set
Color Range = Full on the OBS source to match.

Add new export sizes and looks in `presets.json`.

## How it behaves on a big shoot

Measured on a 200 frame session, which is a full day of headshots:

| | |
|---|---|
| `/api/state` | 49 ms |
| film strip, cold | 78 ms per thumbnail of a full 8256x5504 frame |
| film strip, warm | under 1 ms per thumbnail, cached on disk by mtime |
| preview, first load of a 45MP frame | 383 ms, then cached |
| slider move | 6 ms, or 38 ms with all nine knobs |
| batch export of all 200 | 23.7 s, 0.12 s per frame, 0 errors |
| contact sheet | 10.7 s over 7 pages |

**Thumbnails and contact sheets decode the JPEG at reduced scale** (libjpeg's
draft mode), which is where most of that speed comes from. Measured
like-for-like on a real 8256x5504 frame: 311 ms per thumbnail without it
against 78 ms with, and a 30 frame contact sheet 5.58 s against 2.40 s, for a
worst channel difference of 2/255. The **export path deliberately does not**,
and the self-test asserts that, because a shortcut that leaks into exports
would quietly degrade every headshot.

The contact sheet **paginates**, 5 columns by 6 rows per page. One sheet of 200
frames came out 2200 by 12,880 pixels, which nothing opens comfortably and
nothing prints; the pages are 2200 by 1932 and numbered `1-of-7` so a set stays
in order.

## A failed Webcam press leaves the camera reachable

Pressing **Webcam** releases the tether first. If the feed then fails to start,
usually because the camera is off, `manual_off` stayed set and auto-connect was
dead: **the camera would not come back when powered on again.** Found by the
suite's own leak check, not by using the app. A failed attempt now re-arms
auto-connect and says why, naming "no camera on USB" or that the camera may be
busy or asleep.

## Recording the good microphone

A skit recorded in camera gets the camera's **built-in microphone**, which is
the weakest link in the whole chain when an interface and a real mic are
sitting right there.

**Record audio** captures the proper input in parallel, as 24-bit 48 kHz mono
WAV, into the same session folder beside the video. **Clap once at the top** so
there is a transient to line the two up on in the edit.

**The button follows the server, not the page.** While a take runs it shows the
elapsed time, and a page reload keeps showing it: the camera's Record button had
exactly this bug, where a reload showed "Record" while the camera kept rolling.
For audio it would be worse, since `pw-record` writes about 9 MB a minute and
nothing in the page would stop it.

A forgotten take is stopped by a **watchdog**: after an hour by default
(`Z8_AUDIO_MAX_SECONDS`), or sooner if the disk gets tight, because the disk
was only checked when the recording started. Left overnight a take would be
about 4.3 GB. Proven with a 5 second cap: the take stopped on its own, and the
partial recording is **kept**, not discarded.

Each take carries an **x** to bin it, because takes cost roughly **9 MB a
minute** and would otherwise only be clearable from a file manager. It goes to
the same recoverable trash as a deleted frame, so **Undo delete** brings it
back byte for byte.

Deleting a take works by **name, not by stem**, unlike a frame. A clip and a
take recorded in the same second share a stem, and the frame path removes every
file matching it, so deleting the audio would have taken the video with it. The
self-test builds exactly that pair and checks the clip survives.

Each take is **listed in the app with its length and a player**, so you can
hear it without opening a file manager. That is not a nicety: `shots()` lists
images and `clips()` lists video, so a `.wav` appeared **nowhere** in the UI
and the recording was invisible, which is the same trap as the trim that could
not be played back.

Verified end to end against a Focusrite Scarlett Solo: a 4.5 s take produced a
633 KB WAV, 48000 Hz, 1 channel, 24-bit, matching the reported length. Starting
twice does not spawn a second recorder, and stopping when nothing is running is
refused rather than crashing.

## Using the recorded audio on a take

**Use recorded audio**, in the clip player, replaces the camera's audio with
the recorded take and lines the two up **on the clap**. The video stream is
copied, not re-encoded, so nothing is lost. It picks the take recorded closest
in time to the clip, and offers **Play the synced take** straight after, because a synced file you
cannot watch is the un-playable trim all over again.

Verified against a known ground truth rather than by eye: a clap at 1.0 s in
the clip and 2.5 s in the take gives a true offset of +1.5 s, which it detected
**exactly**, and re-analysing the output found the clap at 1.04 s where the
video's sits at 1.00 s. Detecting an offset and muxing a file are not the same
as being in sync, so the test checks the result, not the intent.

The onset threshold is **relative**, not a fixed level. A hardcoded one looked
sensible and rejected a real clap in a quietly recorded take: the same clap
reads 525 at one gain and thousands at another. A clap is identified by
towering over the rest of the track, so it asks for a rise that is a good share
of the loudest moment and far above typical variation. A steady tone and pure
silence both correctly yield no clap.

## Sound

**Check audio** reports the input device, whether it is muted, phantom power,
and a measured peak level. A self-tape with no sound is a wasted take, and
diagnosing a dead microphone by hand took half a dozen shell probes: default
source, mute, volume, phantom, and finally the interface's own level meter.

The verdict comes from **recording a moment and measuring the samples**, and is
an explicit "unknown" when that is not possible. It does NOT come from the
interface's ALSA `Level Meter`: a first version did, read **4095 off a playback
channel**, and reported healthy audio while the microphone was completely dead.
That meter mixes capture and playback and the mapping is interface specific, so
it is shown as information and never used as the answer.

Measured on a real rig (Audio-Technica into a Focusrite Scarlett Solo 4th Gen):
a dead input read a peak of ~0%, and a working one 36 to 71%. Over about 85% it
warns that the gain is too hot.

What it cannot see is the interface's **GAIN knob**, which is analogue. When no
sound arrives, that knob is the first thing to check, then the cable at both
ends, then the microphone's own switch.

## Fix exposure, during a call

While the webcam feed is up, the app cannot change a single camera setting: the
live view pipeline holds the USB session. That is exactly when you notice the
exposure is wrong. Measured on a real call: **M at 1/1000s and f/6.3 indoors**,
1.5 stops dark, and the only way to fix it was the camera's own dials.

**Fix exposure** does the handover: takes the tether back, sets aperture
priority so the camera meters the shutter itself, opens the lens to the widest
its choices offer, and hands the camera straight back to the feed. About six
seconds. The handback sits in a `finally`, so a refused setting can never leave
you with no camera.

## The webcam feed never stretches

**Measured on the real feed 2026-09-21.** v4l2loopback keeps whatever frame
size its first writer gave it until every consumer closes the device, so once
Zoom is open the size is fixed. Switching the camera from photo to video
changes live view from 3:2 (1024x680) to 16:9 (1024x576), and feeding that into
a device still set to 680 tall stretched the picture **18.1% vertically**.

`z8-webcam.sh` now reads the device's current size and fits the source inside
it with padding, so the geometry is right for any combination. Close every
consumer and reopen to get the native size with no bars.

The first attempt at this silently did nothing: `tr -dc '0-9/'` over the whole
`v4l2-ctl` line also kept the slash inside the words "Width/Height", so the
width parsed as empty and the fit was skipped. A stretched picture appears in
no log, which is why the self-test drives the real parse and keeps the broken
expression as its control.

## Tethered RAW did not work, measured

Setting image quality to **NEF+Fine star** over USB is accepted and reads back
correctly, and then the camera shoots a plain JPEG anyway. Measured
2026-09-21: the app reported `NEF+Fine*`, the frame that arrived was 18 MB of
`Quality: JPEG Fine` at 8256x5504, no NEF came down the cable, and a listing of
the card's newest files found no NEF on it either.

**Update 2026-10-06, measured:** the cause was the capture target. libgphoto2 defaults it to
*Internal RAM*, and `shoot` downloads only the one file the camera reports, so the NEF of a NEF+JPEG shot
was lost and nothing reached the card. With `gphoto2 --set-config capturetarget=1` (Memory card; run once
with the app stopped, it persists in `~/.gphoto/settings`) a shot through the app still downloads the JPEG,
and DSC_xxxx.NEF + .JPG stay on the card: a Lossless 14-bit NEF, imported with Import from card.

So raw over the tether is not something to rely on. If raw matters for a
session, set it on the camera, confirm on the camera that it is writing NEF,
and pull the files with **Import from card** afterwards. The JPEG path is
measurably good now that the camera is in SDR with the Portrait picture
control, and it is what the tether actually delivers.

## The card listing shows the newest files

It used to stop at the first 400 files the tree yielded, so on a card holding
more than that the recent shots were unreachable, which are the only ones
anybody imports. Measured on a real card: 400 old frames returned, truncated,
with a shot taken a minute earlier nowhere in the list. A camera numbers its
DCIM folders and their files upwards, so the walk now takes folders in
descending order and the TAIL of any folder that overflows the cap. Same card
afterwards: DSC_7560 first instead of DSC_6625.

## Raw: set the camera to Lossless compressed

**Measured on a real frame 2026-09-21.** With RAW recording set to **High
Efficiency** or **HE\***, darktable's rawspeed refuses the file as
`corrupt: Out of bounds access`. It is not corrupt, it is Nikon's TicoRAW,
which rawspeed cannot decode, and the only embedded preview is 160x120, so
there is nothing to fall back to. A 27.7 MB HE* NEF is therefore unusable here.

The compression mode is **not exposed over PTP** (only `imagequality` is), so
the app cannot change it. On the camera:

    PHOTO SHOOTING MENU > RAW recording > RAW compression > Lossless compressed

Frames already shot as HE have to be re-shot. The app detects HE and HE* via
exiftool and says all of this instead of "darktable-cli produced nothing".

## Things that will otherwise waste an hour

- **`PTP Access Denied` on capture means the photo/video switch is on VIDEO.**
  The tell is `focusmode` stuck on AF-F while a drive-mode change still works.
- **GNOME's gvfs claims the camera instantly.** It is masked two ways, both
  needed: `systemctl --user mask gvfs-gphoto2-volume-monitor.service` and a
  D-Bus override at
  `~/.local/share/dbus-1/services/org.gtk.vfs.GPhoto2VolumeMonitor.service`.
  Side effect: the camera no longer appears in Files.
- **Killing a holder wedges the MTP session.** Every later open fails with a
  -10 timeout until a `USBDEVFS_RESET`. The server does this automatically on
  the first failed connect, so it usually self-heals.
- **On "JPEG Normal" the Z8 emits HEIF, confirmed on real captures.** Every
  frame lands as a `.heic` plus a converted `.jpg`, which is why `normalise()`
  exists. A tethered capture takes about 3.7 s end to end.
- **The camera reports a `.nef` filename while emitting HEIF.** Files are
  renamed by magic bytes, never by the extension the camera claims, and the
  renderer sniffs the bytes too, so a file copied in by hand keeping the lie
  is still routed correctly. Measured on a real 45MP frame: 3.7 s through the
  raw developer against 0.88 s once routed to the HEIF path.
- **`capturetarget` defaults to Internal RAM**, so tethered frames never reach
  the card. Change it in the Camera panel if you want a card backup.
- **The "Nikon Z8 Webcam" launcher is now safe, and was not.** It ran
  `pkill -9 -f mtplvcap` and started a second webcam pipeline behind this app's
  back. SIGKILL wedges the MTP session, so every later open failed with a -10
  timeout until the USB port was reset, and clicking it during a call killed
  the feed that call was using. It now asks Z8 Studio to switch mode, or starts
  the app if nothing is running. The original is kept at
  `~/bin/z8-launcher.sh.bak-2026-09-21`.

## If something external goes missing

The app shells out to `ffmpeg`, `ffprobe`, `heif-convert`, `darktable-cli` and
`xdg-open`. If any is gone, a red banner at the top of the page names it, what
it breaks, and the `apt install` that fixes it, and each operation refuses with
the same sentence instead of an errno.

The page itself keeps working with **all five missing**, verified in the
self-test. That is not free: a missing `pgrep` used to throw straight out of
`/api/state`, which the page polls constantly, so the whole UI died over a
status probe. Every request is now wrapped, so an unexpected error becomes a
JSON 500 that names itself and a traceback in the log, never a dead socket.

## If it crashes

It comes back by itself. `Restart=always`, three seconds, with a crash-loop
limit of 5 in 120 seconds because the camera is USB and hammering it is worse
than being down for a moment. Verified by killing it with SIGSEGV (recovered in
5 s) and SIGKILL (4 s).

**Memory is returned to the OS after heavy work.** Measured: a 30 frame batch
peaks at 585 MB and settles back to 62 MB, but a contact sheet used to leave
**440 MB resident forever**. Python frees the objects and glibc keeps the
arenas, so `malloc_trim` is called after a batch and after a sheet; the same
reproduction now settles at 50 MB. The self-test guards this only against a
catastrophic regression: the stranding is cumulative over a process's whole
life, the fixed and unfixed bands overlap within a single run, and a tighter
threshold was tried and withdrawn rather than ship one that looks
discriminating and is not.

**One real segfault was observed**, during a contact sheet under sustained
back-to-back test runs, at a 2.8 GB memory peak. The cause is not identified;
it is in a C extension, so PIL or libgphoto2. It has not recurred. The app
restarted on its own and lost nothing, because the only state that matters is
already on disk.

If it dies **mid-batch**, the page is left polling a job id that no longer
exists, because jobs live in memory. It now gets a 410 saying the server
restarted and that anything already exported is on disk, rather than the button
quietly going back to normal with no explanation. Verified by killing the
server during a 30 frame batch: it came back in 5 s, the page was told, and the
2 frames finished before the crash were still there.

The `StartLimit*` settings used to sit in `[Service]`, where systemd **silently
ignores them**, so the real window was the 10 second default rather than the
120 the comment claimed. They are in `[Unit]` now and the preflight asserts it.

## Testing and health

    cargo test --release                              # in-process behaviour tests
    python3 ~/bin/z8-studio/selftest.py               # the live server, over HTTP
    python3 ~/bin/z8-studio/selftest.py --preflight   # 19 checks, 0.2 seconds

**Run `--preflight` after a reboot or a system update.** It checks the things
the app cannot fix for itself, all of which fail looking like a camera fault:
the service enabled and running, gvfs masked BOTH ways (the unit and its D-Bus
activation, because the unit alone is not enough), v4l2loopback loaded with
`/dev/video10` present and set to come back after a reboot, the three scripts
executable, both Rust binaries built and run by their units, libgphoto2
installed, and every external tool installed.

Every text colour is measured against **all three** backgrounds (page, panel
and the button panel) for WCAG AA. `--mute` was 3.59:1 and `--tally`, the error
colour, was 4.10:1 on a button, both below AA on 11px text: filenames, notes,
the player bar, error messages. Now `#818D99` and `#F2555A`, worst case 4.73:1
and 4.75:1.

The second of those was only found after fixing the guard itself: the variable
name pattern was `--[a-z]+`, which silently excluded `--panel2`, the button
background, from this check AND from the undefined-variable check. A name
pattern that cannot match every real name is a guard with a hole in it.

The layout is measured at 1920x1080, 1366x768, 1280x720 and 900x600 by asking
the page to report its own geometry: a DOM dump is byte-identical at every
window size and proves nothing. It checks for horizontal overflow and for any
button, select or input that is zero-sized or positioned outside the viewport,
which is a control the reader cannot reach however correct the markup is. No
overflow and nothing unreachable down to 700x900.

Checks that assert "none of X" also assert that X was not empty. An `all()`
over an empty list is True, so four checks here would have passed with nothing
to examine, including one that asserts a shell-out never receives a
caller-supplied path: with the stub neutered so nothing launched at all, it
used to report "none escaped" and pass.

Every performance check proves its own workload was real. That is not
pedantry: a slider benchmark once reported **2 ms** for a nine-knob grade
because the knobs were passed under the wrong names and the render was a no-op,
and the thumbnail benchmark ran over a single frame while reporting a
per-frame average. A guard that measures nothing reports success.

A failing run appends its check names to `~/.cache/z8-studio/failures.log`.
That exists because one run in about twenty failed a single check, the name was
lost to a shell pipeline, and twelve clean runs could not reproduce it. The log
named it on its first real use: **rotation and mirror are global server state
that outlive a page**, so a browser harness torn down mid-sequence left the
server on 270 degrees, which rotated every frame and broke the next run's
geometry checks with no local cause.

The run ends by proving it gave back everything global it touched: camera
rotation and mirror, the auto-connect flag (left off, the camera never opens by
itself again and the next morning looks like a broken app), the active session,
the output settings, and `presets.json`. Two of those five were falsified by
dirtying the state on purpose; the rotation one had already been falsified by a
real leak.

**Coverage cannot shrink quietly.** Only the camera may legitimately be
skipped; any other skip fails the run, and the total is ratcheted at a floor.
Both exist because silent loss has happened twice: the browser harness ran 18
interactions instead of 24 for a while, and the memory section skipped itself
because it ran after the frame it needed was deleted. Raise the floor when you
add checks; if it ever drops, a section stopped running.

**Every run proves it left your files alone.** It snapshots every file in every
session before starting and compares afterwards, reporting anything left behind
or removed and failing the run. That exists because the suite littered a real
session twice: a 7 MB seed frame after a crash, and a recorded `.wav` after a
teardown patch silently failed. Falsified both ways, by breaking the harness
cleanup (caught the stray WAV) and by deleting a file outright (caught the
removal).

Running the whole suite inside a scratch session was attempted instead and
**reverted**: it touches 14 call sites and four attempts broke it in different
ways. Detecting the harm is a smaller change that catches it whatever the cause.

Cleanup is unconditional. A crashed or interrupted run used to leave a 7 MB
`selftest-seed.jpg` in the day's session, which you would find in the morning;
fixtures and global state are now both swept in a `finally`, verified by
interrupting a real run with SIGINT.

**A headless run never opens the live view.** `/preview.mjpg` is an endless
stream, so `chrome --dump-dom` waits for a page that never finishes loading and
simply hangs. With the camera off that was invisible; the first run with real
hardware reported 0 of 4 window sizes and left Chrome sitting there. `?e2e=1`
and `?layout=1` therefore suppress the stream, and every headless invocation
now carries a bounded timeout so a hang cannot stall the suite.

The full run works with the camera off, which is deliberate: the camera is the
least available part of the system and the export pipeline is the part that
breaks silently. Camera-dependent checks report SKIP, not failure.

**50 of those checks are real interactions driven in a headless browser**, and
`?e2e=1` runs them in a live page too. That exists because the whole UI was once
dead, a handler bound to a missing element, while 62 API checks passed green.
The destructive ones only ever act on a frame the harness created for itself, so
opening `?e2e=1` by hand cannot touch a real shot.

    systemctl --user status z8-studio
    systemctl --user restart z8-studio

`~/bin/z8-studio.sh` starts it by hand instead, if you prefer.

## Layout

| path | what |
|---|---|
| `target/release/z8-studio` | HTTP server, owns the camera, does all rendering (`src/`) |
| `target/release/z8-feed` | the webcam keeper (`src/bin/z8-feed.rs`) |
| `web/` | the page: `index.html`, `app.css`, `src/app.ts` built to `dist/app.js` |
| `presets.json` | export sizes and looks. Edit freely, it is read live |
| `~/Pictures/z8-studio/<date>/` | captures |
| `~/Pictures/z8-studio/<date>/exports/` | renders |
| `~/Pictures/z8-studio/<date>/picks.json` | pick / reject flags |
| `~/Pictures/z8-studio/<date>/.trash/` | deleted frames, recoverable |
| `~/Pictures/z8-studio/<date>/.cache/` | strip thumbnails, self-pruning |

## Adding things

A key the renderer does not read does **nothing**, silently: `quailty: 95`
would export at the default quality forever. The self-test checks every setting
in `presets.json` against the code, so a typo fails the run instead of quietly
doing nothing.

New export size or look: add an object to `presets.json`. Export entries take
`w`, `h`, `fit` (cover/contain), `anchor` (center/face/top/bottom), `format`,
`quality`. Look entries take exposure, shadows, highlights, contrast,
saturation, warmth, tint, vignette, sharpness. Looks can also be saved from the
UI with the name field under the sliders.

## Why portrait changes the framing, not just the resolution (2026-09-22)

The `face` anchor is `ImageOps.fit` centering `(0.5, 0.35)`. The `0.35` lifts the
eyes; the `0.5` is dead centre. So the anchor only does anything on the axis that
is actually being cropped, and that axis is decided by the source orientation.

Measured against this camera's real frame sizes:

| preset | landscape 8256x5504 | portrait 5504x8256 |
|---|---|---|
| headshot-8x10 | trims 2100px of width, 0 tall | trims 600px of height |
| casting-profile | 1200px wide, 0 tall | 200px tall |
| headshot / -cutout | 512px wide, 0 tall | 512px tall |
| ig-portrait | 945px wide, 0 tall | 270px tall |

Shot landscape, all five print presets crop left and right. The anchor's x is
centred, so it moves nothing, and the export keeps every pixel of headroom the
frame had. Shot portrait, all five crop top and bottom and the anchor is live on
every one.

This is asserted, not fixed. The fix is turning the camera: a 4:5 crop out of a
3:2 frame has no vertical excess to anchor into. `the_face_anchor_has_an_axis_to_work_on()`
guards the property so a preset dimension edit cannot silently kill it, and the
landscape notice in `control_notices()` states it in the app.

## Two halves of the dead-control guard

`every id the script binds exists in the markup` catches a binding to markup that
was deleted. Its reverse, added here, catches the other direction: a control the
markup offers that the script never wires, which is a button the user can press
that does nothing.

It matches on a word boundary, not on a quoted selector, because `mTether`,
`mWebcam` and `mOff` are wired as keys of the `MODES` dispatch map and a
selector-only search calls all three dead. A second check goes further, since
named is not wired: every `<button>` must reach an `onclick`, an
`addEventListener`, or a dispatch-map entry, resolved through the variable it was
assigned to. Falsified three ways; a button assigned to a variable and never
wired passes the first check and fails the second, which is why both exist.

## Portrait framing, driven through the real pipeline (2026-09-22)

`a_portrait_frame_actually_frames_the_head()` builds a genuinely 5504x8256
subject, a head at a headshot's proportion with shoulders below, exports it
through `/api/export` and measures where the skin lands. The older portrait test
copies a landscape frame and tags it `Orientation=6`, which proves the export is
upright and nothing about the framing, because the content inside is still
landscape.

| source | head centre | crown at |
|---|---|---|
| portrait 5504x8256 | 0.246 to 0.270 | 0.07 to 0.14 |
| landscape 8256x5504 | 0.418 to 0.420 | 0.24 |

Portrait is textbook: eyes near the upper third, headroom kept, nothing clipped,
and all four headshot presets agree within 2.4% of frame height. Landscape parks
the head dead centre with a quarter of the frame empty above it.

No face detection is involved and none is claimed. This measures crop geometry.

Falsified by setting the anchor to `(0.5, 0.5)`, which the guard caught twice
over, including one result worth keeping: at dead centre the square `headshot`
preset **clips the crown**. The `0.35` is not a taste setting, it is protecting
the top of the head on the 1:1 crop.

## The suite refuses to grade a stale server

`the_server_under_test_is_the_code_on_disk()` runs first. The server hashes the
five files it loaded (`server.py`, `app.js`, `index.html`, `app.css`,
`presets.json`) at import and reports `sourceId` in `/api/state`; the suite
compares it against the same hash taken from disk.

The incident: a mutation test left a server started as `python3 server.py` from
inside the directory. That relative argv means `pkill -f "z8-studio/server.py"`
never matches it, so it held port 8420 through three restarts, kept serving the
mutated crop anchor after the file was already correct, and left the systemd
unit sitting in `failed` while the API answered normally. Two framing checks
failed and read as real defects.

The guard also asserts the unit is `active`, since an orphan holding the port is
exactly what puts it in `failed`. To recover: find the PID holding :8420
(`ss -lptn 'sport = :8420'`), kill it by number rather than by pattern, then
`systemctl --user restart z8-studio.service`.

## The JPEG-loses-quality complaint, answered (2026-09-22)

The camera writes JPEG (tethered NEF does not work on this body), so every
graded export re-encodes and that is a **second lossy generation**, paid again
each time DarkTable exports on top. Measured on a real 8256x5504 frame with the
clean-headshot grade applied:

| format | size | mean error | max error |
|---|---|---|---|
| JPEG q98 4:4:4 | 16.3 MB | 0.6386 levels | 9 |
| JPEG q95 | 11.0 MB | 0.7281 levels | 12 |
| **PNG** | **51.6 MB** | **0.0000** | **0** |
| TIFF LZW | 94.9 MB | 0.0000 | 0 |

Two presets close it, and TIFF is now a supported output format for a tool that
insists on one, though PNG is half the size for the same exactness.

- **`original`** copies the camera's file byte for byte. Generation 1, nothing
  re-encoded, and nothing here can improve on it. This is the right start if he
  is going to edit in DarkTable anyway.
- **`edit-lossless`** is a full-resolution PNG carrying the grade with zero
  error, for when he wants the look kept and still wants to edit on top.

`original` applies no edits, so reporting "exported" and letting him discover
that in DarkTable would be the worst version of the feature. The payload carries
`copied: true` and `ignored_edits`, and the UI names each dropped slider and
points at the lossless preset.

`ADJ_DEFAULTS` is the single declaration of what "untouched" means, because the
copy path has to know which edits it is dropping and guessing would make it lie.
`the_adjustment_defaults_are_the_real_ones()` proves it through the real export
route, not by reading the source back and not by importing `server.py` (which
would run its module-level camera setup against the live process): a slider at
its declared default must give a byte-identical export, and the same slider
nudged must not. That second half also proves all nine sliders are live.

## One naming rule for every preset (2026-09-22)

`export_name()` is the single place an export's filename is decided. The COPY
preset does not go through `write_out`, and when first written it grew its own
`f"{stem}_{tag}"`: every other preset honoured `name_template` and that one
silently did not, the kind of divergence that surfaces months later as an odd
file in the folder.

A copy also keeps the SOURCE extension rather than the preset's. Calling a
byte-for-byte copy of a JPEG `.png` because the preset says PNG would be a lie
about the file, so `export_name` takes an `ext` override for exactly that.

Verified against a custom `{date}-{stem}-{preset}` template through the real
route: all three of `original`, `yt-thumb` and `edit-lossless` obey it, with
`.jpg`, `.jpg` and `.png` respectively. Falsified by restoring the hand-rolled
name, which the behavioural check caught and named. Both source-text checks
passed on that mutant, because it spelled the concatenation differently, which
is why the behavioural check is the one that matters.

Batch copy verified too: four frames through `/api/export-many` with the
`original` preset, every one byte-identical to its source on disk.

## Confirming a setting he fixed by hand (2026-09-22)

Three of the settings that matter most on this body cannot be reached over USB:
HLG tone mode, Auto ISO, and the camera's physical orientation. He changed all
three on the camera itself. A warning that simply stops appearing is not
confirmation that the change took, and it can never tell him about a knocked
dial putting one back weeks later.

`control_confirms()` is a separate channel from `control_notices()`, rendered in
its own green box, because a green line inside the red warnings box teaches him
to stop reading both.

Driven against his own shoot on 2026-09-21:

| frame | reading | earns |
|---|---|---|
| 15:20 | `Standard(HLG)`, Auto ISO On | nothing |
| 21:04 | SDR, Auto ISO Off, ISO 320 | SDR line, fixed-ISO line |
| 21:06 | same | same |
| portrait (tomorrow) | 5504x8256 | adds the portrait line |

It stays silent when it cannot read a frame rather than reassuring him about
something it did not check, and each wrong setting withdraws exactly its own
line and no other, which the guard verifies in both directions. HLG is detected
through `PictureControlName`, `TransferCharacteristics` and `ColorPrimaries`
independently, since any one of the three can carry it.

## "Reset crop" no longer reframes the picture (2026-09-22)

`fit_to` places a face-anchored crop at centering y=0.35, which lifts the eyes.
`defaultCrop` in the browser centred vertically. On a portrait frame those two
disagree, so pressing reset in the crop tool gave different framing from letting
the preset crop on its own. Measured through the real export route on a portrait
subject:

| path | crop y | head centre |
|---|---|---|
| old, centred | 0.0833 | 0.235 |
| new, face-anchored | 0.0583 | **0.266** |
| preset, no crop at all | | **0.266** |

2.5% of frame height, 206 pixels on an 8256-tall frame.

The map now lives in `ANCHOR_CENTERING` on the server and rides to the browser
inside `/api/presets` as `_anchors`, so there is one definition instead of two
copies that drift. The guard asserts the server keeps exactly one and that
app.js holds no second copy.

It carries its own control: it renders the centred framing too and requires it
to differ from the preset's. Without that, a fix that changed nothing would pass
just as happily, and the old behaviour would have been harmless all along.

Landscape sources are unaffected: there the crop runs on the x axis, and every
anchor has x=0.5.

## Ten routes crashed on a bad body (2026-09-22)

Swept 30 POST routes against 6 malformed bodies, 180 requests. Ten came back
HTTP 500, all one class: the right KEYS with the wrong TYPES.

| body | what it reached |
|---|---|
| `name: [1,2]` | `os.path.join` |
| `id: {}` | used as a dict key |
| `date: 5` | `.strip()` |
| `preset: {"format": 9}` | `.upper()` |

Ten separate one-line crashes in ten handlers, so patching them individually
would have fixed ten and missed the eleventh. `body_type_error` now rejects at
the single POST entry point, and **rejects rather than coerces**: coercion
invents an intent the caller never expressed, while a 400 naming the field and
both types is honest and leaves no handler needing to defend itself.

One case could not be caught there, because it lives INSIDE the preset dict, so
`format` is `str()`-wrapped at its three use sites.

`BODY_TYPES` is deliberately narrow. A first draft also listed `look`, `to`,
`value`, `flag` and `mode`, and two were wrong: `/api/look` posts a whole
adjustment OBJECT as `look`, and `/api/copy-edits` accepts `to` as either a name
or a list. The suite caught both on the next run, which is the argument for
constraining only names that are not polymorphic across routes.

Result: 180 malformed requests, zero 5xx, zero silent drops. A separate pass
drives ten malformed PRESETS against a real frame, since the body check cannot
see inside a dict, and none of those crashes either.

## An export is never half-written under its real name (2026-09-22)

`im.save(out)` publishes the final NAME the instant encoding begins. Measured by
SIGKILLing the server 2.2 seconds into a 51.6 MB lossless PNG: an **8.3 MB file
sat under the real name and PIL refused to open it**. He would have found that
in the folder and taken it to DarkTable as a finished export. The `max_kb`
quality search made it worse, rewriting the same published path up to a dozen
times as it walked quality down.

All four writers (render, the `max_kb` search, the COPY preset, the contact
sheet) now write to `.name.part-<pid>-<tid>` beside the destination and move it
in with `os.replace`. Same directory on purpose: `os.replace` is only atomic
within one filesystem, and the exports folder can be anywhere. The cleanup
catches `BaseException`, so a staging file does not survive a shutdown either.

Repeating the identical kill test afterwards left **zero** files under the final
name.

### The sweep

Nothing runs after SIGKILL, so the staging file itself does survive a kill. It
is hidden and never mistaken for a finished export, but it occupies real disk,
so `sweep_staging()` collects them at startup, which is exactly when the process
that owned them is known to be gone.

Keyed on the PID being dead, **not on age**: another Z8 Studio, or a long batch
in this one, may be legitimately mid-write, and deleting its staging file to
tidy up after a dead process would corrupt a live export. Verified with four
probes at once: a pid-1 file, a dead-pid file, an unparseable one and an
ordinary file. Only the dead-pid one was removed.

### Why the probe is trustworthy

The guard does not kill anything, which would break every later section. It
polls the destination every 50 ms during a slow export and requires the final
name to be a complete readable image from the first instant it exists.
Falsified by restoring the direct save: **27 of 28 sightings unreadable**,
against 0 with the fix. All three source-text checks passed on that mutant,
because it spelled the change differently, so the behavioural probe is again
the one carrying the weight.

## Bad URLs: the GET half of the sweep (2026-09-22)

The POST sweep covered bodies. Query strings and the four file-serving prefixes
were untested. Measured: every GET route survived 11 hostile query strings
(traversal, 6 KB names, NUL bytes, negative and absurd widths, repeated keys,
unicode, a bad percent escape), and **no traversal payload escaped the session
folder** across 14 attacks on four prefixes.

Two requests did return HTTP 500: `GET /export/` and `GET /export/.`

An empty name resolves to the exports **folder**, which exists and contains no
`"/"`, so a guard written as `os.path.exists` passed it through and the handler
opened a directory. `/clip/` and `/take/` were saved only by their extension
check, which is luck rather than design. All four now require `os.path.isfile`.

Falsified by restoring `exists`, which failed both the behavioural check and the
source-text one, and the behavioural check found a third case the source text
could not distinguish: `GET /export/..`

### What was already right

Five of the six JSON writers (`config.json`, `edits.json`, `picks.json`,
`presets.json`, camera presets) already write to a temp file and `os.replace`,
and the two sidecar writers hold `_SIDECAR_LOCK` across read-modify-write, so
his grades cannot be lost to a crash or a concurrent edit. Checked this cycle
before looking elsewhere; no change needed.

## The app now says what to do next (2026-09-22)

There was no in-app guidance at all. He opens this after hours away and had to
remember where he was in the job.

`next_step()` returns ONE action, the first unmet rung of a five-stage ladder,
with the reason underneath and a progress track. Deliberately a single action,
not a checklist: a list of five things is five things to forget.

| stage | rung |
|---|---|
| Connect | camera off, unless he switched it off himself |
| Camera setup | portrait, then HLG, then Auto ISO |
| Shoot | nothing in today's session |
| Cull | counts what is LEFT to decide, not what is done |
| Export | skips keepers that already have a file on disk |

Orientation leads the camera-setup rungs, and the guard asserts it outranks the
other two, because a frame shot in the wrong orientation has to be taken again
while the others are merely ugly. All-rejected is called out separately rather
than reported as "done".

Every rung runs against the real function with a fake camera and session, so the
whole ladder is exercised with no hardware. Verified in a headless render too:
the panel paints its stage label, action, reason and a five-dot track with no JS
errors.

### Two bugs caught while building it

`export_stems` labels a live export `"current"`, not `"ok"`. The first draft
tested for `"ok"`, which never matches, so the panel would have sat on "export
the keepers" forever even after exporting them all.

The caching wrapper had a bare `except Exception: val = None`, and that hid a
real `AttributeError` (`CAM.connected` does not exist; the handle is `CAM.cam`)
for a full restart. `None` is also a legitimate answer from this function, so
the swallow was indistinguishable from working correctly. It logs now, and the
guard asserts it does.

## Orientation advice follows the MODE (2026-09-22)

Portrait is right for an 8x10 headshot and **wrong for a Zoom call**. A call is
16:9, and a portrait sensor letterboxes him into a narrow strip with black
either side.

The next-step ladder shipped an hour earlier with one ladder for everything, so
mid-call it would have said "Turn the camera to portrait". The landscape warning
and the portrait confirmation had the same defect: all three judged orientation
without knowing what the frame was for.

All three take `mode` now, resolved from `webcam_running()`:

| | tethered stills | on a call |
|---|---|---|
| landscape | "Turn the camera to portrait" | accepted |
| portrait | accepted, moves to Shoot | "Turn the camera back to landscape" |
| next rung | Shoot, Cull, Export | "Pick Z8 Studio as your camera in Zoom" |
| HLG | caught | caught |

The webcam ladder is a genuinely different three-stage ladder (Connect, Camera
setup, On the call), not the stills one with a rung suppressed.

The guard asserts the advice flips in **both** directions for all three
channels. A rule that fired in neither mode would pass a one-sided test just as
happily, and HLG is checked in both modes to prove the split did not drop it.

## Recording a skit is its own mode (2026-09-22)

Splitting stills from calls left video in the stills ladder, so mid-skit it said
"Turn the camera to portrait". YouTube is 16:9.

`current_mode()` returns one of three, and each has its own ladder:

| mode | when | orientation | ladder |
|---|---|---|---|
| webcam | the feed holds the camera | landscape | Connect, Camera setup, On the call |
| movie | recording, or between takes | landscape | Connect, Camera setup, Record, Review |
| tether | otherwise | **portrait** | Connect, Camera setup, Shoot, Cull, Export |

"Movie" covers being **between takes**, not just mid-record: he stops the camera
to reset a line and the advice must not flip in the gap. The test is whether the
newest thing in the session is a clip, which stops being true the moment he
takes a still. Verified across the whole sequence: record, stop, still, record
again.

### The HLG rung is deliberately absent from movie

On a still, HLG is a defect: the JPEG comes out flat and nothing recovers it. On
video it is a real HDR delivery format somebody may have chosen on purpose, and
calling a deliberate choice an error is worse than staying quiet. The guard
asserts it is silent in movie mode **and** still fires on stills, because a
shared rung would have made one of those wrong.

Also fixed while here: "Cull the 1 frames" and "Export the 1 keepers". Counting
bugs read as sloppiness on every single-frame session, and a one-frame session
is common when he is testing a light.

## The morning, rehearsed end to end (2026-09-22)

Every next-step guard until now supplied a fake camera and a fake session, and
**all of them passed while the real thing was broken**. Driving the live API
instead, with the camera off: five frames shot, culled and exported, and the
panel sat on "Turn the camera on" the entire way, never once offering the work
that was available. Culling and exporting need no camera. Every fake handed it a
CONNECTED camera, so not one of them could see it.

Three real fixes came out of it:

1. **Desk work outranks the connect rung.** Offline with frames waiting, the
   panel names the cull or the export. He opens this in the morning with
   yesterday's frames and the camera asleep, and the useful answer is "cull
   them", not "go plug something in". A finished session still gets its "Done",
   because losing that the moment the camera sleeps would hide the only piece of
   visible progress in the panel.
2. **The cached answer is dropped on anything that moves him a rung**: pick,
   export, delete, undelete, empty trash, batch export, card import, and a
   session switch. Without it the panel lagged a full second behind his own
   click, and a session switch showed the previous day's counts, which is wrong
   rather than merely stale.
3. `capture_dir` moves where the shutter WRITES; the strip and every count read
   `session_dir()`. Redirecting it isolates nothing, so the rehearsal runs in a
   throwaway dated session instead.

### Three mistakes of mine worth keeping

- The first rehearsal asserted "an empty session asks for the camera" against
  the live session and failed on frames an earlier section had seeded. A test
  that only holds when it runs first is not a test.
- Three guards each carried their own copy of the `index()` slice boundaries
  used to load these functions without importing server.py. One broke the moment
  `desk_work` was added above `next_step`: the slice started below it. They share
  `_load_ladder` now.
- The rehearsal cleared `export_dir` with `None` when it finished. That **pops**
  the key, which destroyed the suite's own sandbox override and sent later
  sections' exports elsewhere. Save and restore, never clear. The end-of-run
  check now prints the exact differing key instead of two long JSON blobs, which
  is what made it findable.

## The video path, rehearsed for real (2026-09-22)

The movie ladder was fake-tested only. Driving it with an actual clip found the
same hole the stills path had, in a **second place**: the stills fix bypassed
the connect rung when desk work was waiting, and the movie ladder kept its own
unconditional one, so an 8 second take on disk still read "Turn the camera on".
Reviewing and trimming a take needs no camera.

Verified end to end, no fakes: an 8s clip is written, the app reads its
duration, the panel moves to Review with the camera off, a 4s..8s precise trim
runs as a real job, and the cut lands on disk at 4.00s and 1280x720.

**The cut is verified by pixels.** The fixture is 4s green then 4s red, and the
first frame of the result must come back red. A keyframe-snapping cut would land
in the green half, and on the Z8's long GOP that is the realistic failure, so an
exit code proves nothing here. Measured RGB (254, 0, 0).

Taking a still afterwards hands the session back to the stills ladder, which is
the mode flip checked against real files rather than a fake clock.

### The cache is keyed on the folder now, not on a timer

A frame arriving from the shutter is a FILE appearing with no API call behind
it, so a purely time-based cache left the panel up to a second behind. It also
fooled three test probes in a row into reporting a bug that was not there, which
is its own argument for fixing it.

`_next_step_cached` now fingerprints the session folder with `os.scandir`, which
costs microseconds, and only falls back to the one-second timer when the
fingerprint is unchanged. `actual_settings()` shelling out to exiftool is what
the cache was really protecting. Re-verified with **no sleeps anywhere**: take
appears, panel says Review; still appears, panel says Cull.

## "Turn the camera on" was wrong in all three ladders (2026-09-22)

The connect rung was hand-written into each ladder and wrong in every one,
found one at a time over three cycles:

| ladder | the state | what it said |
|---|---|---|
| stills | camera off, frames waiting | "Turn the camera on" instead of Cull |
| movie | camera off, take on disk | the same; the stills fix did not reach here |
| **webcam** | **a live Zoom call** | **"Turn the camera on" while he was on it** |

The webcam one is the worst and the least obvious. `set_mode("webcam")` calls
`CAM.disconnect()`, because the feed holds the MTP session and the two cannot
share it, so `CAM.cam is None` is true for the **whole duration of a call**.
Reaching that branch at all means `webcam_running()` was true, which is itself
the evidence the camera is working, so the branch has no business checking the
handle.

### The guard tests the property, not the three sites

Three separate fixes are three separate regressions waiting to happen, so
`no_ladder_asks_for_a_camera_that_is_already_working()` asserts the invariant:
**if a mode's own work is available, that mode must not ask for the camera.** It
also asserts the rung still fires where it IS the right answer, since the
property alone could be satisfied by deleting it everywhere, and that a camera
he switched off himself is left alone.

A structural check counts the ladders and fails if a fourth appears, so a new
mode cannot silently inherit a bug that has now been made three times.

Falsified by reintroducing the webcam rung: both the behavioural and structural
checks caught it.

### A guard that punished writing things down

The structural check first searched the branch for `CAM.cam is None` and matched
its own comment, which explains the bug in prose. It strips comments before
matching now. A guard a comment can fail is a guard that discourages comments.

## What a screenshot found that 851 checks did not (2026-09-22)

Significant UI had gone in this session (the next-step panel, the confirmation
box, new CSS) and none of it had ever been LOOKED at. Rendering the page and
reading the picture found two real bugs on an empty day, which is exactly the
state the app is in every morning:

1. **The session dropdown rendered completely blank.** `renderStrip` returns
   early when there are no frames, and the picker was populated *after* that
   return. It is the one control that reaches yesterday's work, blank on the one
   day he needs it.
2. **The empty message wrapped into a ragged column**: "No shots yet / today. /
   2026-09-21 / has 29 / frames." `.strip` is a four-column grid, so its message
   became a grid ITEM one column wide.

Neither is visible to a check that asks whether an element exists, and both were
obvious in one glance. The guard now renders a genuinely empty day in headless
Chrome and reads the real DOM: the dropdown must carry dated options, and the
strip's own message must name the other day and offer its button.

Scoping mattered: an unscoped search for `class="empty"` matched "Connect to
load settings." from another panel, so the check is anchored to `id="strip"`.

**The lesson, which is in the house rules already: a visual change needs a
visual check.** Eight hundred passing assertions did not substitute for opening
the page once.

## The stage shows the frame he is culling (2026-09-22)

Looked at the app with the real 29-frame session loaded. The next-step panel
said "Cull the remaining 26", the strip showed 29 thumbnails, and **the entire
centre of the window said "Plug in the Z8 and switch it on."** The only large
view of the selected frame was the crop pane, in the narrow rail below the fold.
He was culling headshots from 74-pixel thumbnails.

With no live feed and a frame selected, the stage now shows that frame at
`?w=1600`. The feed takes the stage back the moment it starts.

Written without an early return on purpose: that branch still has to reach the
line that disables Shoot, Focus and Record, and an early return skipping later
work is exactly the bug found in `renderStrip` an hour earlier. The guard
asserts there is no `return;` between the still and that line.

## Review pass, 2026-09-24

First full `selftest.py` run since the keeper and DirectWriter commits: 972
passed, 3 failed. All three were real, two of them in the suite itself.

- **The suite ran inside his real day for 70 seconds.** The malformed-body fuzz
  POSTs `{}` to `/api/session`, which is a valid "go to today", and the next
  sections (crop agreement, portrait framing) wrote fixture frames into
  2026-09-24 before cleaning them up, so the end-of-run file check saw nothing.
  Found with a watcher logging every session change. The fuzz now re-pins
  1998-12-31 after each `/api/session` probe, and every section runs through
  `contained()`, which fails loudly, names the section, and puts the run back.
- **A full run refused whenever the webcam was merely wanted.** With the keeper,
  "webcam" stays true with the camera switched off. The guard now refuses on a
  real stream (feed `live` / `reconnecting`, or a camera on the bus), and the
  run puts `.feed.json` and `.last-mode.json` back byte for byte afterwards.
- **The keeper burned 14% of a core idle**, re-decoding the same static card
  10 times a second. A card is now converted once per look: 15 ms to 0.07 ms a
  write, 14% to 2% of a core.
- **A live graded frame went from 21 ms to 12 ms**: box-blur unsharp instead of
  UnsharpMask (was 40% of the frame), vignette on luma only, `reduce(2)` for the
  chroma planes. Mean difference 0.3 levels, max 3, checked side by side.
- `restore_mode_if_interrupted` stands down while the keeper runs: a restart
  inside a keeper reconnect re-ran `set_mode` and rewrote the keeper's control
  file with the marker's older look.
- `lsusb` under the camera lock and the `pkill` in `stop_feed` are time-bounded.

Result: 1006 passed, 0 failed, 9 skipped (all camera-off).

Still open: `browser: and the big picture goes back` failed once (1 in the 256
logged runs) and passed on every rerun. Not reproduced, not fixed.

## Would Rust make the webcam faster? Measured 2026-09-24

`rust-frame/` is the keeper's frame path (`DirectWriter.frame`) written in
Rust: same decode, crop, grade, luma vignette, box unsharp, 2x2 chroma and
limited-range I420. Its output matches `z8-feed.py` within 4 levels (mean 0.7)
on luma and 1 level on chroma, checked byte for byte, so both do the same work.

    cd rust-frame && cargo build --release
    ./target/release/bench <frame.jpg> <out-dir> 300
    python3 bench_python.py <frame.jpg> <out-dir> 300

Per frame, whole path including the JPEG decode, his call look, 300 runs,
two alternating rounds on the Ryzen 5 5600X:

| | median | p99 | CPU at 30 fps |
|---|---|---|---|
| Python (Pillow), today | 7.7 ms | 9-10 ms | 23% of a core |
| Rust, one thread | 5.0-5.3 ms | 5.4-6.5 ms | 15% of a core |
| Rust, all cores (rayon) | 2.5-3.0 ms | 4.8-7.6 ms | spread over cores |

Rust is genuinely 1.5x faster on one thread and about 3x on all cores. That
is **2.5 to 5 ms saved per frame**, out of roughly 100 ms from glass to
screen, most of which is the Z8's own live view and USB. Not a delay anyone
can feel, so the keeper stays Python. Revisit only if a glass-to-glass
measurement shows the keeper is a large share of the delay.

The decoder barely matters: libjpeg-turbo 1.1 ms, pure-Rust zune-jpeg 1.4 ms.
The first Rust vignette was 26 levels off because Pillow's `radial_gradient`
is `sqrt(2 * d^2)`, the 2.0 inside the root; parity caught it.

Limits: the input was a real photo resized to the live-view size (1024x680,
q85), not a frame from the camera, which was off. Glass-to-glass latency and
the Python MJPEG reader's own cost were not measured.

## The webcam keeper runs Rust now (2026-09-24)

`z8-feed.py` loads `rust-frame/target/release/libz8_frame.so` through ctypes
and converts every live frame with it, on a private pool of four threads.
Measured LIVE on the Z8 by the keeper itself, camera frame arrival to written
on `/dev/video10`, three restarts, 20 s each:

| engine | hold p50 | worst | keeper CPU |
|---|---|---|---|
| Python | 10-13.5 ms | 23 ms | 35% of a core |
| Rust, 1 thread | 6-7 ms | 10 ms | 20% |
| **Rust, 4 threads** | **3.6 ms** | 6.6 ms | 23% |

About 8 ms less delay per frame. `.feed-status.json` now reports `engine`,
`rustErrors` and `holdMs` (p50 / p90 / max of the last 90 frames), so this is
visible any time the feed is live.

It cannot cost the webcam: no library, a failed load, or a startup result
that differs from Python's picture (a colour ramp through a strong grade, max
8 levels allowed, real frames are within 4) all mean Python. An error on one
frame falls back to Python for that frame. `Z8_FEED_ENGINE=python` forces
Python; `Z8_FEED_THREADS` sets the pool (default 4). A fresh clone runs
Python until `cd rust-frame && cargo build --release`.

## Where the rest of the webcam delay is (2026-09-24, no camera-at-screen test)

Measured stage by stage, since the camera could not be pointed at a screen:

| stage | cost | action |
|---|---|---|
| reading the camera's MJPEG stream (Python) | 0.1 ms median per frame | none, not worth Rust |
| decode, grade, write (Rust x4) | 3.6 ms | done |
| OBS V4L2 buffering | already off in the Youtube_Videos scene | none |
| camera frame cap `-fps 30` | frames up to 33 ms old when OBS grabs one | test uncapped with the camera ON |

- **`/api/connect` refuses while the webcam has the camera** (409, "press
  Tether"). A gphoto2 open against a running mtplvcap wedges the MTP session;
  the suite's own teardown called it whenever manual_off was set.
- A 13 s dropout at 19:25:55 (frames stopped, the keeper restarted the link)
  is UNEXPLAINED: nothing the suite ran at that moment touches USB.
- `glass_to_glass.py` is the full camera-to-screen test (a flashing Chrome
  window, the camera pointed at it). Built and dry-run only: with the camera
  on his face it correctly refused ("no clear flash"). Not yet run for real.
