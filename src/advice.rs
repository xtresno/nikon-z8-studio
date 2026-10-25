//! What the camera ACTUALLY did (from the last frame's EXIF), the one next
//! step, and the sentences that name a control that is not in control.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::app::{app, mtime, now, read_json, write_json_atomic};
use crate::camera::cam;
use crate::feedctl::webcam_running;
use crate::load::which;
use crate::store::{self, clips, export_stems, load_picks, session_dir, shots, splitext};

const FIELDS: &[&str] = &["ISO", "AutoISO", "FNumber", "ExposureTime", "ExposureCompensation",
    "FocusMode", "LensID", "WhiteBalance", "Quality", "ShutterCount", "ExposureProgram",
    "TransferCharacteristics", "ColorPrimaries", "PictureControlName", "NEFCompression",
    "BitDepthLuma", "ImageWidth", "ImageHeight", "Orientation"];

fn rename(k: &str) -> String {
    match k {
        "ISO" => "iso", "AutoISO" => "autoISO", "FNumber" => "fNumber",
        "ExposureTime" => "exposureTime", "FocusMode" => "focusMode",
        "ExposureCompensation" => "exposureCompensation", "LensID" => "lens",
        "WhiteBalance" => "whiteBalance", "Quality" => "quality", "ShutterCount" => "shutterCount",
        "ExposureProgram" => "exposureProgram", "TransferCharacteristics" => "transfer",
        "ColorPrimaries" => "primaries", "PictureControlName" => "pictureControl",
        "NEFCompression" => "nefCompression", "BitDepthLuma" => "bitDepth",
        "ImageWidth" => "width", "ImageHeight" => "height", "Orientation" => "orientation",
        other => other,
    }.to_string()
}

static ACTUAL: Mutex<Option<HashMap<(PathBuf, u64), Value>>> = Mutex::new(None);
/// Frames whose EXIF is being read in the background right now.
static READING: Mutex<Vec<(PathBuf, u64)>> = Mutex::new(Vec::new());

/// The frame whose settings are reported: the given one, else the newest
/// in the session (its .heic when the camera wrote one).
fn actual_path(path: Option<&Path>) -> Option<PathBuf> {
    match path {
        Some(p) => Some(p.to_path_buf()),
        None => newest_frame(&shots()),
    }
}

/// The newest frame of a listing already made, as the settings readout
/// wants it (its .heic when the camera wrote one).
pub fn newest_frame(ss: &[Value]) -> Option<PathBuf> {
    let p = session_dir(None).join(ss.first()?["name"].as_str()?);
    let raw = p.with_extension("heic");
    Some(if raw.exists() { raw } else { p })
}

/// actual_settings without the wait, for the status poll: exiftool is Perl
/// and the first poll after every shot stalled on it (219 ms measured). The
/// cached reading if there is one, else one background read of that frame
/// and None until it lands, which the next poll (3 s) then shows.
pub fn actual_settings_now(path: Option<&Path>) -> Option<Value> {
    which("exiftool")?;
    let path = actual_path(path)?;
    let key = (path.clone(), mtime(&path)?.to_bits());
    if let Some(v) = ACTUAL.lock().as_ref().and_then(|m| m.get(&key)) {
        return Some(v.clone());
    }
    let mut r = READING.lock();
    if !r.contains(&key) {
        r.push(key.clone());
        std::thread::spawn(move || {
            let _ = actual_settings(Some(&key.0));
            READING.lock().retain(|k| *k != key);
        });
    }
    None
}

/// What the camera did, from the newest frame's own EXIF (the .heic when the
/// camera wrote one). Cached on path and mtime: this shells out.
pub fn actual_settings(path: Option<&Path>) -> Option<Value> {
    which("exiftool")?;
    let path = actual_path(path)?;
    let key = (path.clone(), mtime(&path)?.to_bits());
    if let Some(v) = ACTUAL.lock().as_ref().and_then(|m| m.get(&key)) {
        return Some(v.clone());
    }
    let mut args: Vec<String> = vec!["exiftool".into(), "-s".into()];
    args.extend(FIELDS.iter().map(|f| format!("-{f}")));
    args.push(path.to_string_lossy().into_owned());
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = crate::tools::run_text(&refs, 8);
    if out.is_empty() {
        return None;
    }
    let mut res = Map::new();
    res.insert("file".into(), json!(store::fname(&path)));
    for line in out.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim();
            if !v.is_empty() {
                res.insert(rename(k.trim()), json!(v));
            }
        }
    }
    // Width and height are the SENSOR's; a turned body is only the tag.
    let turn = live_turn_of(res.get("orientation").and_then(|v| v.as_str()).unwrap_or(""));
    res.insert("turn".into(), json!(turn));
    if (turn == 90 || turn == 270) && res.contains_key("width") && res.contains_key("height") {
        let (w, h) = (res["width"].clone(), res["height"].clone());
        res.insert("width".into(), h);
        res.insert("height".into(), w);
    }
    let v = Value::Object(res);
    let mut g = ACTUAL.lock();
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 24 {
        m.clear();
    }
    m.insert(key, v.clone());
    Some(v)
}

pub fn live_turn_of(o: &str) -> i64 {
    let o = o.trim();
    if o == "6" || o.contains("90 CW") {
        90
    } else if o == "8" || o.contains("270 CW") {
        270
    } else if o == "3" || o.contains("180") {
        180
    } else {
        0
    }
}

fn live_turn_file() -> PathBuf {
    app().home.join(".live-turn.json")
}

/// How far to turn the live view: the newest REAL frame's orientation, else
/// the last one remembered from any day.
pub fn live_turn(actual: Option<&Value>) -> i64 {
    let a = actual.cloned().unwrap_or_else(|| json!({}));
    let real = crate::app::truthy(a.get("lens")) || crate::app::truthy(a.get("shutterCount"));
    if real && a.get("orientation").is_some() {
        let turn = a.get("turn").and_then(|v| v.as_i64()).unwrap_or(0);
        let known = read_json(&live_turn_file()).and_then(|v| v.get("turn").and_then(|t| t.as_i64()));
        if known != Some(turn) {
            let _ = write_json_atomic(&live_turn_file(),
                &json!({"turn": turn, "from": a.get("file"), "at": now()}), 0);
        }
        return turn;
    }
    if crate::app::truthy(a.get("orientation")) {
        return a.get("turn").and_then(|v| v.as_i64()).unwrap_or(0);
    }
    read_json(&live_turn_file()).and_then(|v| v.get("turn").and_then(|t| t.as_i64())).unwrap_or(0)
}

// ------------------------------------------------------------------ next step
/// The session's fingerprint: its folder, how many files, the newest mtime.
type SessionSig = (PathBuf, usize, i64);
/// (computed at, the answer, the fingerprint it was computed for)
static NEXT: Mutex<(f64, Option<Value>, Option<SessionSig>)> = Mutex::new((0.0, None, None));

pub fn next_step_changed() {
    NEXT.lock().0 = 0.0;
}

pub fn next_step_cached() -> Value {
    let t = now();
    let d = session_dir(None);
    let sig = std::fs::read_dir(&d).ok().map(|it| {
        let v: Vec<i64> = it.filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| mtime(&e.path()).unwrap_or(0.0) as i64).collect();
        (d.clone(), v.len(), v.iter().copied().max().unwrap_or(0))
    });
    {
        let mut n = NEXT.lock();
        if sig != n.2 {
            n.0 = 0.0;
            n.2 = sig;
        }
        if t - n.0 < 1.0 {
            return n.1.clone().unwrap_or(Value::Null);
        }
    }
    let live = Live::default();
    let actual = newest_frame(&live.shots()).and_then(|p| actual_settings_now(Some(&p)));
    let val = next_step(&live, actual.as_ref(), &current_mode(&live));
    let mut n = NEXT.lock();
    n.0 = t;
    n.1 = val.clone();
    val.unwrap_or(Value::Null)
}

/// Everything the next-step ladder reads about the world. The server passes
/// `Live`; a test passes a fake, so every rung runs with no camera and no
/// session. Methods, not fields, so the live server still reads a folder only
/// when a rung actually needs it.
pub trait World {
    fn camera_open(&self) -> bool;
    fn manual_off(&self) -> bool;
    fn recording(&self) -> bool;
    fn webcam(&self) -> bool;
    fn shots(&self) -> Vec<Value>;
    fn clips(&self) -> Vec<Value>;
    fn picks(&self) -> Map<String, Value>;
    fn stems(&self) -> Vec<Value>;
}

/// The real camera and today's session. One answer lists the session
/// several times (the newest frame, the counts, the picks); it is listed
/// once per Live, as a poll listing 1000 frames four times cost ~20 ms.
#[derive(Default)]
pub struct Live {
    shots: std::sync::OnceLock<Vec<Value>>,
}

impl World for Live {
    fn camera_open(&self) -> bool { cam().is_open() }
    fn manual_off(&self) -> bool { cam().a.lock().manual_off }
    fn recording(&self) -> bool { cam().a.lock().recording }
    fn webcam(&self) -> bool { webcam_running() }
    fn shots(&self) -> Vec<Value> { self.shots.get_or_init(shots).clone() }
    fn clips(&self) -> Vec<Value> { clips() }
    fn picks(&self) -> Map<String, Value> { load_picks(None) }
    fn stems(&self) -> Vec<Value> { export_stems(None) }
}

/// What he is DOING, which decides whether portrait is right or wrong.
pub fn current_mode(w: &dyn World) -> String {
    if w.webcam() {
        return "webcam".into();
    }
    if w.recording() {
        return "movie".into();
    }
    let reels = w.clips();
    if !reels.is_empty() {
        let newest_clip = reels.iter().map(|c| c["at"].as_f64().unwrap_or(0.0)).fold(0.0, f64::max);
        let newest_still = w.shots().iter().map(|f| f["at"].as_f64().unwrap_or(0.0)).fold(0.0, f64::max);
        if newest_clip >= newest_still {
            return "movie".into();
        }
    }
    "tether".into()
}

/// Is there anything to do that needs no camera?
pub fn desk_work(w: &dyn World) -> bool {
    if !w.clips().is_empty() {
        return true;
    }
    let frames = w.shots();
    if frames.is_empty() {
        return false;
    }
    let picks = w.picks();
    let flagged = |n: &str| crate::app::truthy(picks.get(n));
    if frames.iter().any(|f| !flagged(f["name"].as_str().unwrap_or(""))) {
        return true;
    }
    let keepers: Vec<&str> = frames.iter().filter_map(|f| f["name"].as_str())
        .filter(|n| picks.get(*n).and_then(|v| v.as_str()) == Some("pick")).collect();
    let done: std::collections::HashSet<String> = w.stems().iter()
        .filter(|s| s["state"] == "current").filter_map(|s| s["stem"].as_str().map(|x| x.to_string())).collect();
    keepers.iter().any(|k| !done.contains(splitext(k).0))
}

fn hlg_on(a: &Value) -> bool {
    let blob: String = ["transfer", "primaries", "pictureControl"].iter()
        .map(|k| a.get(*k).map(store::py_str).unwrap_or_default()).collect::<Vec<_>>().join(" ").to_uppercase();
    blob.contains("HLG") || blob.contains("B67") || blob.contains("2020") || blob.contains("2100")
}

fn shape(a: &Value) -> (i64, i64) {
    let g = |k: &str| a.get(k).map(store::py_str).and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or(0);
    (g("width"), g("height"))
}

fn rung(l: &[&str], i: usize, step: String, why: &str) -> Option<Value> {
    Some(json!({"step": step, "why": why, "stage": i, "stageName": l[i], "total": l.len(), "ladder": l}))
}

fn s_if(n: usize) -> &'static str {
    if n != 1 { "s" } else { "" }
}

/// ONE thing to do next, decided from what is true right now. The ladder
/// depends on the mode: portrait advice is backwards during a call.
pub fn next_step(w: &dyn World, actual: Option<&Value>, mode: &str) -> Option<Value> {
    if mode == "webcam" {
        let l = ["Connect", "Camera setup", "On the call"];
        if let Some(a) = actual {
            let (w, h) = shape(a);
            if w > 0 && h > 0 && h > w {
                return rung(&l, 1, "Still turned for portrait? Turn it back to landscape".into(),
                    "Your last photo was portrait, and a call is 16:9: a portrait camera sends a \
                     narrow, sideways picture. Already landscape? Ignore this; it clears with your \
                     next photo.");
            }
            if hlg_on(a) {
                return rung(&l, 1, "Turn HLG off on the camera".into(),
                    "HLG is an HDR curve, and everything downstream of it here is ordinary sRGB, so \
                     you look flat and washed out to everyone on the call.");
            }
        }
        return rung(&l, 2, "Pick Nikon Z8 as your camera in Zoom or OBS".into(),
            "The feed is live. Zoom: Video, Camera. OBS: Video Capture Device, Nikon Z8. Opened \
             before the feed started? Re-select Nikon Z8 there: the device only offers a picture \
             while the feed is running, and an app that looked earlier does not look again.");
    }
    let offline = !w.camera_open() && !w.manual_off();
    if mode == "movie" {
        let l = ["Connect", "Camera setup", "Record", "Review"];
        let reels = w.clips();
        if offline && reels.is_empty() {
            return rung(&l, 0, "Turn the camera on".into(), "Nothing records until it is awake and plugged in.");
        }
        let actual = if offline { None } else { actual };
        if let Some(a) = actual {
            let (w, h) = shape(a);
            if w > 0 && h > 0 && h > w {
                return rung(&l, 1, "Turn the camera back to landscape".into(),
                    "YouTube is 16:9. A portrait frame is for a Short, and everything else here \
                     assumes a wide one.");
            }
        }
        if reels.is_empty() {
            return rung(&l, 2, "Record a take".into(),
                        "Press Record. The take lands in today's session with the stills.");
        }
        return rung(&l, 3, format!("Review the {} take{}", reels.len(), s_if(reels.len())),
            "Play it back below, trim the head and tail, and the cut lands in the exports folder.");
    }
    let l = ["Connect", "Camera setup", "Shoot", "Cull", "Export"];
    if offline && !desk_work(w) && w.shots().is_empty() {
        return rung(&l, 0, "Turn the camera on".into(),
                    "Nothing here works until it is awake and plugged in. It reconnects on its own.");
    }
    let actual = if offline { None } else { actual };
    if let Some(a) = actual {
        let (w, h) = shape(a);
        if w > 0 && h > 0 && w > h {
            return rung(&l, 1, "Turn the camera to portrait".into(),
                "An 8x10 headshot cropped out of a landscape frame keeps 53% of the sensor and \
                 leaves the face dead centre. Turned, you keep 83% and the eyes sit in the upper third.");
        }
        if hlg_on(a) {
            return rung(&l, 1, "Turn HLG off on the camera".into(),
                "HLG writes an HDR file that is flat and desaturated as an ordinary JPEG, and no \
                 grading recovers it. Photo Shooting Menu, Tone mode, SDR.");
        }
        if a.get("autoISO").map(store::py_str).unwrap_or_default().to_lowercase() == "on" {
            return rung(&l, 1, "Turn Auto ISO off".into(),
                "It overrides the ISO you set, so two frames a second apart can be exposed differently.");
        }
    }
    let frames = w.shots();
    if frames.is_empty() {
        return rung(&l, 2, "Take a frame".into(),
                    "Nothing in today's session yet. Shoot one and the strip below fills in.");
    }
    let picks = w.picks();
    let flagged = frames.iter().filter(|f| crate::app::truthy(picks.get(f["name"].as_str().unwrap_or("")))).count();
    if flagged == 0 {
        return rung(&l, 3, format!("Cull the {} frame{}", frames.len(), s_if(frames.len())),
                    "Keep or reject each one. Everything you keep is what the export batch will use.");
    }
    if flagged < frames.len() {
        return rung(&l, 3, format!("Cull the remaining {}", frames.len() - flagged),
                    &format!("{flagged} of {} decided so far.", frames.len()));
    }
    let keepers: Vec<&str> = frames.iter().filter_map(|f| f["name"].as_str())
        .filter(|n| picks.get(*n).and_then(|v| v.as_str()) == Some("pick")).collect();
    if keepers.is_empty() {
        return rung(&l, 3, "Every frame is rejected".into(),
                    "Nothing is marked keep, so there is nothing to export. Shoot again, or \
                     un-reject the ones worth having.");
    }
    let done: std::collections::HashSet<String> = w.stems().iter()
        .filter(|s| s["state"] == "current").filter_map(|s| s["stem"].as_str().map(|x| x.to_string())).collect();
    let todo = keepers.iter().filter(|k| !done.contains(splitext(k).0)).count();
    if todo > 0 {
        return rung(&l, 4, format!("Export the {todo} keeper{}", s_if(todo)),
                    "Pick a preset and use Export all. For DarkTable, choose Original file or Full \
                     size, lossless.");
    }
    rung(&l, 4, format!("Done: {} exported", keepers.len()),
         "Every keeper has an export on disk. Nothing left in this session.")
}

/// "+1/3", "-0.7", "0" as a number.
pub fn ev_value(v: &str) -> f64 {
    let t = v.trim().replace('+', "");
    let t = if t.is_empty() { "0".to_string() } else { t };
    if let Some((n, d)) = t.split_once('/') {
        match (n.trim().parse::<f64>(), d.trim().parse::<f64>()) {
            (Ok(n), Ok(d)) if d != 0.0 => n / d,
            _ => 0.0,
        }
    } else {
        t.parse().unwrap_or(0.0)
    }
}

fn sget(a: &Value, k: &str) -> String {
    a.get(k).map(store::py_str).unwrap_or_default()
}

/// Say when a setting fixed BY HAND is still right, and only when the frame proves it.
pub fn control_confirms(actual: Option<&Value>, mode: &str) -> Vec<String> {
    let Some(a) = actual else { return vec![] };
    let mut ok = Vec::new();
    let (transfer, primaries, pic) = (sget(a, "transfer"), sget(a, "primaries"), sget(a, "pictureControl"));
    if !transfer.is_empty() || !primaries.is_empty() || !pic.is_empty() {
        let hlg = transfer.to_uppercase().contains("HLG") || transfer.to_uppercase().contains("B67")
            || primaries.contains("2020") || primaries.contains("2100") || pic.to_uppercase().contains("HLG");
        if !hlg {
            ok.push("Tone mode is SDR, not HLG, so your colour and contrast are real this time.".into());
        }
    }
    let prog = sget(a, "exposureProgram").to_lowercase();
    let auto_iso = sget(a, "autoISO").to_lowercase() == "on";
    if !prog.is_empty() && !auto_iso && prog.contains("manual") {
        let iso = sget(a, "iso");
        ok.push(format!("Manual exposure with Auto ISO off{} Every frame will match the last one.",
                        if iso.is_empty() { ".".into() } else { format!(", locked at ISO {iso}.") }));
    }
    let (w, h) = shape(a);
    if w > 0 && h > 0 && h > w && mode != "webcam" {
        ok.push(format!("You are shooting PORTRAIT ({w}x{h}). The headshot presets will crop top and \
                         bottom and put your eyes in the upper third, and you keep about 56% more \
                         resolution than landscape."));
    }
    ok
}

/// Every control that is not actually in control, saying what to use instead.
pub fn control_notices(requested: &Map<String, Value>, actual: Option<&Value>, mode: &str) -> Vec<String> {
    let Some(a) = actual else { return vec![] };
    let mut notes = Vec::new();
    let transfer = sget(a, "transfer");
    let pctrl = sget(a, "pictureControl");
    if transfer.to_uppercase().contains("HLG") || transfer.to_uppercase().contains("ARIB")
        || pctrl.to_uppercase().contains("HLG") {
        notes.push(format!("YOUR CAMERA IS IN HLG TONE MODE. These are 10-bit HDR files ({}), meant \
            for an HDR television. On any ordinary screen they look washed out and desaturated, and \
            grading cannot recover it because the tone curve and the colour gamut are both wrong. It \
            also forces a minimum ISO of 400, adds noise, and switches Picture Controls off. Fix it \
            on the camera: G button, Photo Shooting Menu, Tone mode, SDR. It cannot be changed over USB.",
            if transfer.is_empty() { "HLG".to_string() } else { transfer.clone() }));
    }
    if matches!(sget(a, "nefCompression").as_str(), "13" | "14") {
        notes.push("Your raws are High Efficiency, which no software here can develop. Camera: Photo \
            Shooting Menu, RAW recording, RAW compression, Lossless compressed.".into());
    }
    let (w, h) = shape(a);
    if w > 0 && h > 0 && w > h && mode != "webcam" {
        let (wf, hf) = (w as f64, h as f64);
        let kept_l = (hf * 0.8) * hf;
        notes.push(format!("You are shooting LANDSCAPE ({w}x{h}). An acting headshot is 8x10 portrait, \
            and cropping 4:5 out of a landscape frame throws away both sides: {:.0}% of the sensor \
            survives, against 83% if you turn the camera. Turning it is worth about 56% more \
            resolution in the finished headshot, for nothing. It also decides the FRAMING: on a \
            landscape frame the five headshot presets crop left and right, so the face anchor that \
            lifts the eyes has no axis to work on and the export keeps all your headroom. Turned, \
            every one of them crops top and bottom instead and the anchor puts the eyes where they \
            belong.", kept_l / (wf * hf) * 100.0));
    }
    let auto_iso = sget(a, "autoISO").to_lowercase() == "on";
    let prog = sget(a, "exposureProgram").to_lowercase();
    let ec_s = a.get("exposureCompensation").map(store::py_str).unwrap_or_else(|| "0".into());
    let ec = ev_value(&ec_s);
    if ec != 0.0 && !auto_iso && (prog.contains("manual") || prog == "m") {
        notes.push(format!("Exposure comp is set to {ec_s} and is doing nothing. It biases a light \
            meter, and in M mode with Auto ISO off there is no automatic setting left for it to \
            bias. Change brightness with aperture, shutter or ISO instead."));
    }
    let req = |k: &str| requested.get(k).and_then(|v| v.get("value")).map(store::py_str);
    if auto_iso {
        let shot_iso = a.get("iso").map(store::py_str).unwrap_or_else(|| "?".into());
        let asked = req("iso").unwrap_or_else(|| "?".into());
        let extra = if shot_iso != asked && shot_iso != "?" && asked != "?" {
            format!(" The panel says ISO {asked}; the last frame was shot at {shot_iso}.")
        } else { String::new() };
        notes.push(format!("Auto ISO is on, so the ISO and Aperture sliders do not change brightness: \
            the camera cancels them by moving ISO the other way. Use Exp. comp, or turn Auto ISO off \
            in the camera menu (ISO sensitivity settings, Auto ISO sensitivity control).{extra}"));
    }
    let numv = |s: Option<String>| -> f64 {
        let t: String = s.unwrap_or_default().chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
        if t.is_empty() { f64::NAN } else { t.parse().unwrap_or(f64::NAN) }
    };
    let (ap_asked, ap_got) = (numv(req("aperture")), numv(a.get("fNumber").map(store::py_str)));
    if !ap_asked.is_nan() && !ap_got.is_nan() && (ap_asked - ap_got).abs() > 0.15 {
        notes.push(format!("Aperture reads f/{} here but the last frame was shot at f/{}.", g(ap_asked), g(ap_got)));
    }
    let fm_asked = req("focusmode").unwrap_or_default();
    let fm_got = sget(a, "focusMode");
    if !fm_asked.is_empty() && !fm_got.is_empty() && !fm_got.contains(&fm_asked) && !fm_asked.contains(&fm_got) {
        notes.push(format!("Focus mode reads {fm_asked} here but the last frame was shot in {fm_got}."));
    }
    notes
}

/// Python's :g format for the small numbers it is used on.
pub fn g(x: f64) -> String {
    if x.fract() == 0.0 { format!("{}", x as i64) } else {
        let s = format!("{x:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}
