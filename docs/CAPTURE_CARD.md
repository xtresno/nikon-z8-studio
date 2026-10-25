# Capture card (HDMI) for Z8 Studio: design

Status: designed 2026-09-27, not built. Target card: **Elgato 4K X** (UVC,
works in the in-kernel `uvcvideo` driver from Linux 6.19; this box runs 7.0).

## Why

Today the webcam feed is the Z8's USB (PTP) live view, pulled by `mtplvcap`:

| | USB live view (today) | HDMI capture card |
|---|---|---|
| Picture | about 1024 x 683, JPEG-compressed by the camera | clean 1080p or 4K, uncompressed YUV |
| Frame rate | 30 | 30 or 60 |
| Tether while on a call | no: the feed holds the only MTP session | **expected yes**: HDMI carries video, USB stays free (unverified on the Z8, see Risks) |
| Camera lock | live view and every setting read share one lock | video never touches the lock |

Recording (the Record button, stills) does not change: it is already in-camera
and full quality. The card is for anything LIVE: calls, streams, the stage.

## What the user sees

1. Plug in the card with the Z8 on HDMI. The mode bar's **Webcam** button now
   says **Webcam (HDMI)**. No card, or no signal: it stays **Webcam (USB)**,
   exactly as today.
2. The virtual camera in Zoom/OBS keeps its name, "Nikon Z8", so no app needs
   re-selecting. Its picture goes from 1024x576 to 1920x1080.
3. On a call, **Tether keeps working**: shoot, grade and export while the
   webcam stays live (once the Z8 is confirmed to allow it).
4. The stage's live view comes from HDMI too when the card has signal:
   sharper, 30/60 fps, and no longer paused by a settings read or a shot.
5. HDMI unplugged mid-call: a "HDMI signal lost" card within 1.5 s, the same
   way "Reconnecting..." works today, and back to live the moment it returns.

## Camera and card settings (documented in-app, verified day one)

- SETUP MENU > HDMI > Output resolution: **1080p** (4K costs 4x the CPU for a
  call that Zoom sends at 1080p at most). 4K only for OBS recording.
- HDMI > Advanced > **Output range: Auto**, then measured (below).
- HDMI > Advanced: **info display off on the HDMI output** (clean feed).
- Tone mode **SDR**, never HLG/N-Log for calls (the existing HLG notice).
- Power: EH-8P on the power port, c3 Standby timer **No limit**.

## Architecture

The keeper (`z8-feed`) grows a second SOURCE; everything after the source is
shared.

```
 Z8 USB  --mtplvcap--> MJPEG ----\
                                  >-- Source --> Frame engine (look) --> /dev/video10
 Z8 HDMI --4K X/UVC--> NV12/YUYV-/                 (YU12, W x H)          "Nikon Z8"
```

1. **Source trait** (`src/bin/z8-feed.rs`): `next_frame() -> Frame`,
   `signal() -> bool`, `stop()`. `MtpSource` is today's reader, moved behind
   the trait unchanged. `HdmiSource` is new.
2. **Detection** (`src/feedctl.rs`): scan `/sys/class/video4linux/*/name` and
   the device's capabilities for a CAPTURE device that is not a v4l2loopback
   (excludes `/dev/video10` itself); prefer names matching `4K X`, `Cam Link`,
   `USB Capture`. Signal = the device delivers a frame within 1 s. Published
   in `/api/state` as `hdmi: {device, name, signal, format}`.
3. **HdmiSource**: V4L2 mmap streaming with 4 buffers. Format preference:
   **NV12** (no decode, the frame engine's native YUV family) > YUYV >
   MJPEG (decode with the existing libjpeg-turbo shim). Size from the
   camera's signal; the keeper never scales up.
4. **Frame engine** (`src/frame.rs`): `W`/`H` become runtime values (today
   the constants 1024x576). The look is a per-pixel LUT, so its cost scales
   with pixels: measured 3.6 ms a frame at 1024x576 on 4 threads, so about
   12 ms at 1080p, inside the 33 ms of 30 fps and near the 16.7 ms of 60 fps.
   1080p30 is the default; 60 fps only after measuring it on this machine.
5. **Output**: `VIDIOC_S_FMT` on `/dev/video10` at the new size. Changing the
   size makes open consumers re-negotiate, so switch SOURCE only between
   calls, or keep 1920x1080 for both sources (USB frames scaled up once).
   Decision: output is **always 1920x1080** once a card has ever been seen,
   so Zoom never sees a size change mid-call.
6. **Stage live view**: `/preview.mjpg` serves from the keeper's latest HDMI
   frame (encoded once, shared) instead of polling the camera over USB. The
   dedup rule from 2026-09-26 still applies: send a frame only when it is new.
7. **Mode switching** (`set_mode`): `webcam` picks HDMI when `hdmi.signal`,
   else USB. With HDMI, `set_mode("webcam")` no longer disconnects the
   tether, the one behaviour change in the server.

## Colour range (the known trap)

The existing pipeline converts full range to limited because the camera's USB
JPEGs are full range and v4l2loopback carries no range metadata; Zoom and OBS
assume limited. HDMI YUV from a capture card is normally **limited already**.
Converting it again crushes the shadows and clips the highlights, the exact
bug fixed on the USB path. So the range conversion becomes a property of the
SOURCE (`MtpSource`: convert; `HdmiSource`: pass through), and it is decided
by measurement, not assumption: a day-one test captures a black and a white
frame and asserts Y lands at 16..235 on the output.

## Testing without the card

A capture card is just a V4L2 capture device, and v4l2loopback can pretend to
be one: `modprobe v4l2loopback video_nr=11 card_label="4K X test"` fed by
`ffmpeg -f lavfi -i testsrc2=size=1920x1080:rate=30 -pix_fmt nv12 -f v4l2
/dev/video11`. That drives detection, format negotiation, the source switch,
the signal-lost card (stop ffmpeg) and the output size, all in `cargo test`
and the selftest, with no hardware. Pixel checks: the range test above, and
the look applied identically on both sources (same LUT, same bytes).

## Day-one checklist (with the card, ~30 min)

1. `v4l2-ctl --list-formats-ext -d /dev/videoN`: NV12 at 1920x1080 30/60.
2. **Does the Z8 keep HDMI live while tethered over USB?** Shoot a still over
   USB with HDMI in OBS. This decides point 3 of "What the user sees".
3. Range: black and white frames measure 16..235 on `/dev/video10`.
4. Latency glass to glass (the existing `glass_to_glass.py`), USB vs HDMI.
5. CPU at 1080p30 and 1080p60 with the look on.
6. Unplug HDMI mid-feed: signal-lost card within 1.5 s, recovery on replug.

## Risks and unknowns

- **HDMI + USB at once on the Z8** is documented for the Z6/Z7, not found for
  the Z8. If the Z8 blanks HDMI when tethered, HDMI still wins on picture and
  frame rate, but tethering during a call stays impossible.
- The 4K X needs a 10 Gbps port (it falls back to 4K30 at 5 Gbps).
- `exclusive_caps` loopback semantics are unchanged: the keeper still owns
  `/dev/video10` for the whole session.

## Estimate

About 4 to 6 hours of implementation and tests once the card is here (the
source trait, HdmiSource, runtime frame size, detection, mode switch, stage
live view), plus the 30-minute day-one checklist. Most of it can be built and
tested before the card arrives, against the loopback stand-in above.
