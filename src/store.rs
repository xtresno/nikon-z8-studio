//! Where everything lives on disk: config (stored and transient), sessions,
//! the strip, clips, takes, picks, edits, thumbnails, exports and the trash.
//! Every rule the Python app learned the hard way is kept, with its reason.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::app::{app, expand_user, mtime, read_json, write_json_atomic};
use crate::img::{self, Rgb};
use crate::load::{load_editable, oriented};
use crate::tools::run_timeout;

pub fn today() -> String {
    chrono::Local::now().date_naive().to_string()
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("regex")
}

pub fn is_date(s: &str) -> bool {
    static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    R.get_or_init(|| re(r"^\d{4}-\d{2}-\d{2}$")).is_match(s)
}

pub fn fname(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// os.path.splitext: the stem and the extension, extension including the dot.
pub fn splitext(n: &str) -> (&str, &str) {
    match n.rfind('.') {
        Some(i) if i > 0 && !n[..i].ends_with('/') && !n[..i].chars().all(|c| c == '.') => (&n[..i], &n[i..]),
        _ => (n, ""),
    }
}

// ------------------------------------------------------------------ config
pub fn default_config() -> Map<String, Value> {
    let v = json!({
        "export_dir": "",
        "name_template": "{stem}_{preset}",
        "autofocus_before_shoot": true,
        "autofocus_settle_ms": 600,
        "webcam_look_id": "clean-headshot",
        "capture_dir": "",
        // The saved camera preset "Restore default" puts back.
        "camera_default_preset": "Headshot",
    });
    v.as_object().cloned().unwrap_or_default()
}

fn config_path() -> PathBuf {
    app().home.join("config.json")
}

/// What is actually on disk, ignoring any transient override.
pub fn stored_config() -> Map<String, Value> {
    let mut cfg = default_config();
    if let Some(Value::Object(m)) = read_json(&config_path()) {
        for (k, v) in m {
            cfg.insert(k, v);
        }
    }
    cfg
}

pub fn load_config() -> Map<String, Value> {
    let mut cfg = stored_config();
    for (k, v) in app().transient.lock().iter() {
        cfg.insert(k.clone(), v.clone());
    }
    cfg
}

pub fn cfg_str(k: &str) -> String {
    load_config().get(k).map(|v| match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }).unwrap_or_default()
}

/// Starts from what is STORED, never from load_config(): a save made while a
/// test held a transient override used to write the test's /tmp folders into
/// the real config.
pub fn save_config(patch: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let _g = app().sidecar.lock();
    let defaults = default_config();
    let mut cfg = stored_config();
    for (k, v) in patch {
        if defaults.contains_key(k) {
            cfg.insert(k.clone(), v.clone());
        }
    }
    let d = cfg.get("export_dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if !d.is_empty() {
        let p = expand_user(&d);
        if !p.is_dir() {
            std::fs::create_dir_all(&p).map_err(|e| format!("cannot use that folder: {e}"))?;
        }
    }
    write_json_atomic(&config_path(), &Value::Object(cfg.clone()), 2).map_err(|e| e.to_string())?;
    Ok(cfg)
}

// ------------------------------------------------------------------ sessions
pub fn active_day() -> String {
    app().active_session.read().clone().unwrap_or_else(today)
}

/// A capture always asks for today explicitly; the UI reads the active day.
pub fn session_dir(which: Option<&str>) -> PathBuf {
    let day = which.map(|s| s.to_string()).unwrap_or_else(active_day);
    let d = app().root.join(day);
    let _ = std::fs::create_dir_all(d.join("exports"));
    d
}

/// Where a NEW capture lands: today, unless a test redirected it.
pub fn today_dir() -> PathBuf {
    let d = cfg_str("capture_dir");
    if !d.is_empty() {
        let p = expand_user(&d);
        if p.is_dir() {
            return p;
        }
    }
    session_dir(Some(&today()))
}

/// The configured exports folder, or the session's own. A configured folder
/// that has GONE falls back and says so once; it is never recreated here.
pub fn export_dir(which: Option<&str>) -> PathBuf {
    let raw = cfg_str("export_dir");
    if !raw.is_empty() {
        let d = expand_user(&raw);
        if d.is_dir() {
            return d;
        }
        let mut warned = app().export_dir_warned.lock();
        if !warned.contains(&raw) {
            warned.push(raw.clone());
            eprintln!("exports folder {raw:?} is gone, falling back to the session's own folder");
        }
    }
    session_dir(which).join("exports")
}

pub const RAW_EXTS: &[&str] = &[".nef", ".dng", ".cr2", ".arw"];
pub const FRAME_EXTS: &[&str] = &[".jpg", ".jpeg", ".heic", ".png", ".nef", ".dng", ".cr2", ".arw"];
pub const VIDEO_EXT: &[&str] = &[".mov", ".mp4", ".m4v", ".avi"];
pub const AUDIO_EXT: &[&str] = &[".wav", ".flac"];

pub fn ends_with_any(n: &str, exts: &[&str]) -> bool {
    let l = n.to_lowercase();
    exts.iter().any(|e| l.ends_with(e))
}

/// One definition of "a photograph in the session", for the strip and the
/// picker alike, so their counts cannot disagree.
pub fn is_frame_file(n: &str) -> bool {
    !n.starts_with('.') && ends_with_any(n, FRAME_EXTS)
        && !n.contains(".conv.") && !n.contains(".dev.") && !n.contains(".poster.")
}

pub fn list_dir(d: &Path) -> Vec<String> {
    std::fs::read_dir(d).map(|it| it.filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default()
}

pub fn sessions() -> Value {
    let root = &app().root;
    if !root.is_dir() {
        return json!([]);
    }
    let mut names = list_dir(root);
    names.sort();
    names.reverse();
    let mut out = Vec::new();
    for n in names {
        let d = root.join(&n);
        if !(d.is_dir() && is_date(&n)) {
            continue;
        }
        let stems: HashSet<String> = list_dir(&d).into_iter().filter(|f| is_frame_file(f))
            .map(|f| splitext(&f).0.to_string()).collect();
        out.push(json!({"date": n, "frames": stems.len()}));
    }
    Value::Array(out)
}

// ------------------------------------------------------------------ sidecars
fn sidecar_path(which: Option<&str>, name: &str) -> PathBuf {
    session_dir(which).join(name)
}

pub fn load_picks(which: Option<&str>) -> Map<String, Value> {
    match read_json(&sidecar_path(which, "picks.json")) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

pub fn set_pick(name: &str, flag: Option<&str>) -> Map<String, Value> {
    let _g = app().sidecar.lock();
    let mut p = load_picks(None);
    match flag {
        Some(f) => {
            p.insert(name.to_string(), json!(f));
        }
        None => {
            p.shift_remove(name);
        }
    }
    let _ = write_json_atomic(&sidecar_path(None, "picks.json"), &Value::Object(p.clone()), 1);
    p
}

pub fn load_edits(which: Option<&str>) -> Map<String, Value> {
    match read_json(&sidecar_path(which, "edits.json")) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

pub fn save_edits(name: &str, adjust: Option<&Value>, crop: Option<&Value>, which: Option<&str>)
    -> Result<Option<Value>, String> {
    let _g = app().sidecar.lock();
    let mut all = load_edits(which);
    let adjust = adjust.filter(|v| !v.is_null());
    let crop = crop.filter(|v| !v.is_null());
    if adjust.is_none() && crop.is_none() {
        all.shift_remove(name);
    } else {
        let a = match adjust {
            Some(v) if crate::app::truthy(Some(v)) => v.clone(),
            _ => json!({}),
        };
        all.insert(name.to_string(), json!({"adjust": a, "crop": crop.cloned().unwrap_or(Value::Null)}));
    }
    write_json_atomic(&sidecar_path(which, "edits.json"), &Value::Object(all.clone()), 1)
        .map_err(|e| format!("could not save edits: {e}"))?;
    Ok(all.get(name).cloned())
}

// ------------------------------------------------------------------ the strip
fn rank_of(ext: &str) -> i32 {
    match ext {
        ".nef" | ".dng" | ".cr2" | ".arw" => 5,
        ".jpg" | ".jpeg" => 3,
        ".png" => 2,
        ".heic" => 1,
        _ => 0,
    }
}

/// One entry per frame, newest first. When one exposure left several files,
/// raw wins, then the JPEG, and a HEIF's converted copy is not a second frame.
/// The folder listing each session was last pruned for: the prunes list the
/// folder and its tile cache (two tiles a frame) on every poll, and only a
/// change in the folder can leave anything for them to remove.
static PRUNED: parking_lot::Mutex<Vec<(PathBuf, u64)>> = parking_lot::Mutex::new(Vec::new());

pub fn shots_in(which: Option<&str>) -> Vec<Value> {
    let d = session_dir(which);
    // One stat a frame (it was three: is_file, size, mtime), followed through
    // links as before.
    let mut best: HashMap<String, (String, String, std::fs::Metadata)> = HashMap::new();
    let names = list_dir(&d);
    for n in &names {
        if !is_frame_file(n) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(d.join(n)) else { continue };
        if !meta.is_file() {
            continue;
        }
        let (stem, ext) = splitext(n);
        let ext = ext.to_lowercase();
        let better = match best.get(stem) {
            None => true,
            Some((_, cur, _)) => rank_of(&ext) > rank_of(cur),
        };
        if better {
            best.insert(stem.to_string(), (n.clone(), ext, meta));
        }
    }
    let flags = load_picks(which);
    let edited = load_edits(which);
    let mut out: Vec<Value> = best.values().map(|(n, _, meta)| {
        let p = d.join(n);
        // Provisional: a raw whose tile is the camera's embedded JPEG until
        // it is developed; the page redraws the tile when this clears.
        let raw = RAW_EXTS.iter().any(|e| n.to_lowercase().ends_with(e));
        let at = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|t| t.as_secs_f64()).unwrap_or(0.0);
        json!({"name": n, "size": meta.len(),
               "at": at,
               "pick": flags.get(n).cloned().unwrap_or(Value::Null),
               "edited": edited.contains_key(n),
               "provisional": raw && !crate::load::raw_is_developed(&p)})
    }).collect();
    out.sort_by(|a, b| b["at"].as_f64().unwrap_or(0.0).partial_cmp(&a["at"].as_f64().unwrap_or(0.0))
        .unwrap_or(std::cmp::Ordering::Equal));
    let live: HashSet<String> = best.values().map(|(n, _, _)| n.clone()).collect();
    // Keyed on EVERY name in the folder, not just the frames: prune_derived
    // decides from the listing alone (a real .jpg arriving beside a .heic's
    // conversion, a develop left by a deleted raw), so any add or remove
    // re-runs it exactly as the per-poll prune did.
    let sig = {
        use std::hash::{Hash, Hasher};
        let mut all: Vec<&String> = names.iter().collect();
        all.sort();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        all.hash(&mut h);
        h.finish()
    };
    let fresh = !PRUNED.lock().iter().any(|(p, s)| *p == d && *s == sig);
    if fresh {
        prune_thumb_cache(&d, &live);
        prune_derived(&d, &live);
        let mut g = PRUNED.lock();
        g.retain(|(p, _)| *p != d);
        g.push((d.clone(), sig));
        if g.len() > 16 {
            g.remove(0);
        }
    }
    out
}

pub fn shots() -> Vec<Value> {
    shots_in(None)
}

pub fn safe_name(n: &str) -> String {
    n.chars().map(|c| if c.is_ascii_alphanumeric() || "_.-".contains(c) { c } else { '_' }).collect()
}

/// Derived files (a raw develop, a HEIF conversion) whose frame is gone.
pub fn prune_derived(d: &Path, live: &HashSet<String>) -> u64 {
    let stems: HashSet<&str> = live.iter().map(|n| splitext(n).0).collect();
    let mut freed = 0;
    for n in list_dir(d) {
        let owner: String = if let Some(o) = n.strip_suffix(".dev.jpg") {
            o.to_string()
        } else if let Some(o) = n.strip_suffix(".conv.jpg") {
            splitext(o).0.to_string()
        } else {
            continue;
        };
        if stems.contains(owner.as_str()) {
            // Keep a conversion unless the frame already has a real .jpg.
            if !(n.ends_with(".conv.jpg") && d.join(format!("{owner}.jpg")).is_file()) {
                continue;
            }
        }
        let fp = d.join(&n);
        freed += std::fs::metadata(&fp).map(|m| m.len()).unwrap_or(0);
        let _ = std::fs::remove_file(fp);
    }
    freed
}

pub fn prune_thumb_cache(d: &Path, live: &HashSet<String>) {
    let cdir = d.join(".cache");
    if !cdir.is_dir() {
        return;
    }
    let safe_live: HashSet<String> = live.iter().map(|n| safe_name(n)).collect();
    for f in list_dir(&cdir) {
        // A provisional raw tile is "<name>.<w>.<stamp>.e.jpg".
        let base = f.strip_suffix(".e.jpg").map(|b| format!("{b}.jpg")).unwrap_or_else(|| f.clone());
        let parts: Vec<&str> = base.rsplitn(4, '.').collect();
        if parts.len() != 4 || parts[0] != "jpg" {
            continue;
        }
        if !safe_live.contains(parts[3]) {
            let _ = std::fs::remove_file(cdir.join(&f));
        }
    }
}

pub fn cache_dir() -> PathBuf {
    let d = session_dir(None).join(".cache");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn tid() -> i64 {
    unsafe { libc::syscall(libc::SYS_gettid) as i64 }
}

/// A 240 px strip tile, cached on disk keyed by the source mtime.
pub fn thumb_path(name: &str, width: usize) -> Result<PathBuf, String> {
    let src = session_dir(None).join(name);
    let stamp = mtime(&src).ok_or("no such frame")? as i64;
    let safe = safe_name(name);
    let cdir = cache_dir();
    // A raw nobody has developed yet tiles from its embedded JPEG (0.3 s)
    // instead of waiting about 4 s for a develop just to draw 240 px. That
    // tile is PROVISIONAL, under its own name, because the camera's JPEG is
    // brighter than the develop: once the raw is developed the tile is drawn
    // again from it, so the strip and the editor show the same picture.
    let provisional = crate::load::is_raw(&src) && !crate::load::raw_is_developed(&src);
    let out = cdir.join(format!("{safe}.{width}.{stamp}{}.jpg", if provisional { ".e" } else { "" }));
    if out.exists() {
        return Ok(out);
    }
    let fast = provisional.then(|| crate::load::raw_embedded(&src, Some(width * 3))).flatten();
    // A strip-sized tile of a camera JPEG comes from the 640 px preview the
    // camera embeds (MPF): ~2 ms instead of entropy-decoding all 17 MB, and
    // sized from the FRAME, within a level or so of the full decode (mean
    // 0.59 on a real portrait frame). Anything else takes the full path.
    let mpf = (!provisional && width <= 320)
        .then(|| crate::load::mpf_preview(&src, width * 3 / 2, width)).flatten();
    let im = if let Some((pre, mw, mh)) = mpf {
        let (tw, th) = img::thumbnail_size(mw, mh, width, width * 3).unwrap_or((pre.w, pre.h));
        oriented(img::resize(&pre, tw, th, img::Filter::Lanczos))
    } else {
        let im = oriented(match fast {
            Some(im) => im,
            None => load_editable(&src, Some(width * 3))?,
        });
        img::thumbnail(&im, width, width * 3)
    };
    let bytes = img::jpeg_encode(&im, &img::JpegOpts::q(86))?;
    let tmp = PathBuf::from(format!("{}.{}-{}.tmp", out.display(), std::process::id(), tid()));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
    let keep = fname(&out);
    for f in list_dir(&cdir) {
        if f.ends_with(".tmp") {
            continue;
        }
        if f.starts_with(&format!("{safe}.")) && f != keep {
            let _ = std::fs::remove_file(cdir.join(f));
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ clips, takes
pub fn video_meta(path: &Path) -> Value {
    let key = (path.to_path_buf(), mtime(path).map(|m| m.to_bits()).unwrap_or(0));
    if let Some(v) = app().meta_cache.lock().get(&key) {
        return v.clone();
    }
    let meta = run_timeout(Command::new("ffprobe").args(["-v", "error", "-select_streams", "v:0",
            "-show_entries", "stream=width,height,duration", "-show_entries", "format=duration",
            "-of", "json"]).arg(path), Duration::from_secs(30)).ok()
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
        .map(|d| {
            let st = d.get("streams").and_then(|s| s.get(0)).cloned().unwrap_or(json!({}));
            let dur = st.get("duration").cloned()
                .or_else(|| d.get("format").and_then(|f| f.get("duration")).cloned());
            let dur = crate::app::num(dur.as_ref(), f64::NAN);
            json!({"w": st.get("width").cloned().unwrap_or(Value::Null),
                   "h": st.get("height").cloned().unwrap_or(Value::Null),
                   "duration": if dur.is_nan() { Value::Null } else { json!(crate::app::round(dur, 1)) }})
        })
        .unwrap_or_else(|| json!({"w": null, "h": null, "duration": null}));
    let mut c = app().meta_cache.lock();
    if c.len() > 400 {
        c.clear();
    }
    c.insert(key, meta.clone());
    meta
}

pub fn video_poster(path: &Path) -> Result<PathBuf, String> {
    let out = PathBuf::from(format!("{}.poster.jpg", path.with_extension("").display()));
    if let (Some(o), Some(s)) = (mtime(&out), mtime(path)) {
        if o >= s {
            return Ok(out);
        }
    }
    for seek in ["00:00:01", "00:00:00"] {
        let _ = run_timeout(Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y",
            "-ss", seek, "-i"]).arg(path).args(["-frames:v", "1", "-vf", "scale=1100:-2"]).arg(&out),
            Duration::from_secs(60));
        if std::fs::metadata(&out).map(|m| m.len() > 0).unwrap_or(false) {
            return Ok(out);
        }
    }
    Err("could not read a frame from that clip".into())
}

pub fn clips() -> Vec<Value> {
    let d = session_dir(None);
    let mut names = list_dir(&d);
    names.sort();
    names.reverse();
    let have: Vec<&String> = names.iter().filter(|n| ends_with_any(n, VIDEO_EXT)).collect();
    for n in &names {
        if let Some(stem) = n.strip_suffix(".poster.jpg") {
            if !have.iter().any(|v| splitext(v).0 == stem) {
                let _ = std::fs::remove_file(d.join(n));
            }
        }
    }
    let mut out = Vec::new();
    for n in &names {
        if !ends_with_any(n, VIDEO_EXT) {
            continue;
        }
        let fp = d.join(n);
        let mut o = json!({"name": n, "size": std::fs::metadata(&fp).map(|m| m.len()).unwrap_or(0),
                           "at": mtime(&fp).unwrap_or(0.0)});
        if let (Some(om), Value::Object(mm)) = (o.as_object_mut(), video_meta(&fp)) {
            om.extend(mm);
        }
        out.push(o);
    }
    out
}

/// Length from the WAV header, so a half-written file still reports.
/// Only the chunk headers are read: every status poll lists the day's takes,
/// and reading each whole file (3.4 MB a take) cost a core 2 % doing nothing.
pub fn wav_seconds(path: &Path) -> Option<f64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return None;
    }
    let (mut rate, mut block) = (0u32, 0u16);
    let mut i = 12u64;
    while i + 8 <= len {
        let mut h = [0u8; 8];
        f.seek(SeekFrom::Start(i)).ok()?;
        f.read_exact(&mut h).ok()?;
        let size = u32::from_le_bytes(h[4..8].try_into().ok()?) as u64;
        if &h[0..4] == b"fmt " && i + 8 + 16 <= len {
            let mut fmt = [0u8; 16];
            f.read_exact(&mut fmt).ok()?;
            rate = u32::from_le_bytes(fmt[4..8].try_into().ok()?);
            block = u16::from_le_bytes(fmt[12..14].try_into().ok()?);
        } else if &h[0..4] == b"data" {
            if block == 0 {
                return None;
            }
            let frames = size / block as u64;
            let r = if rate == 0 { 48000.0 } else { rate as f64 };
            return Some(crate::app::round(frames as f64 / r, 2));
        }
        i += 8 + size + (size & 1);
    }
    None
}

pub fn takes() -> Vec<Value> {
    let d = session_dir(None);
    let mut names = list_dir(&d);
    names.sort();
    names.reverse();
    names.into_iter().filter(|n| ends_with_any(n, AUDIO_EXT) && d.join(n).is_file()).map(|n| {
        let fp = d.join(&n);
        json!({"name": n, "size": std::fs::metadata(&fp).map(|m| m.len()).unwrap_or(0),
               "at": mtime(&fp).unwrap_or(0.0),
               "seconds": wav_seconds(&fp).map(Value::from).unwrap_or(Value::Null)})
    }).collect()
}

// ------------------------------------------------------------------ disk
pub fn disk_free_mb(p: &Path) -> Option<f64> {
    let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as f64 * st.f_frsize as f64 / (1024.0 * 1024.0))
}

pub fn disk_ok(p: &Path) -> (bool, Option<String>) {
    match disk_free_mb(p) {
        None => (true, None),
        Some(f) if f < app().min_free_mb => (false, Some(format!("only {f:.0} MB free on the target disk"))),
        Some(_) => (true, None),
    }
}

pub fn dir_size_mb(p: &Path) -> f64 {
    let mut t = 0u64;
    for n in list_dir(p) {
        if let Ok(m) = std::fs::metadata(p.join(n)) {
            if m.is_file() {
                t += m.len();
            }
        }
    }
    t as f64 / (1024.0 * 1024.0)
}

/// `.name.part-<pid>-<tid>` beside the destination, so the rename is atomic.
pub fn staging_path(fin: &Path) -> PathBuf {
    let dir = fin.parent().unwrap_or(Path::new("."));
    dir.join(format!(".{}.part-{}-{}", fname(fin), std::process::id(), tid()))
}

/// Write bytes under a staging name, then publish them in one rename, so a
/// reader never sees a partial file under the real name.
pub fn write_atomic(fin: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = staging_path(fin);
    let r = std::fs::write(&tmp, bytes).and_then(|_| atomic_replace(&tmp, fin));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

pub fn atomic_replace(staged: &Path, fin: &Path) -> std::io::Result<()> {
    match std::fs::rename(staged, fin) {
        Ok(()) => Ok(()),
        // Another filesystem: copy to a staging name beside the destination,
        // then rename there, so the real name still never shows a partial file.
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            let tmp = staging_path(fin);
            let r = std::fs::copy(staged, &tmp).and_then(|_| std::fs::rename(&tmp, fin));
            if r.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            r?;
            std::fs::remove_file(staged)
        }
        Err(e) => Err(e),
    }
}

/// Staging files left by a process that is no longer running. Keyed on the
/// PID being dead, never on age.
pub fn sweep_staging(dirs: &[PathBuf]) -> Vec<String> {
    let mut removed = Vec::new();
    for d in dirs {
        if !d.is_dir() {
            continue;
        }
        for n in list_dir(d) {
            if !n.starts_with('.') || !n.contains(".part-") {
                continue;
            }
            let Some(pid) = n.rsplit_once(".part-").and_then(|(_, r)| r.split('-').next())
                .and_then(|p| p.parse::<i32>().ok()) else { continue };
            let alive = unsafe { libc::kill(pid, 0) } == 0
                || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
            if alive {
                continue;
            }
            if std::fs::remove_file(d.join(&n)).is_ok() {
                removed.push(n);
            }
        }
    }
    removed
}

pub fn trash_dir(which: Option<&str>) -> PathBuf {
    session_dir(which).join(".trash")
}

/// A free name in the trash: `x~2.jpg`, `x~3.jpg`...
pub fn trash_dest(trash: &Path, name: &str) -> PathBuf {
    let mut dest = trash.join(name);
    let mut n = 2;
    while dest.exists() {
        let (r, e) = splitext(name);
        dest = trash.join(format!("{r}~{n}{e}"));
        n += 1;
    }
    dest
}

// ------------------------------------------------------------------ exports
pub fn export_stems(which: Option<&str>) -> Vec<Value> {
    let d = export_dir(which);
    if !d.is_dir() {
        return vec![];
    }
    let mut live: HashSet<String> = shots_in(which).iter()
        .map(|s| splitext(s["name"].as_str().unwrap_or("")).0.to_string()).collect();
    let trash = trash_dir(which);
    if trash.is_dir() {
        live.extend(list_dir(&trash).iter().map(|n| splitext(n).0.to_string()));
    }
    let mut names = list_dir(&d);
    names.sort();
    let mut out = Vec::new();
    for n in names {
        let fp = d.join(&n);
        if !fp.is_file() || n.starts_with('.') {
            continue;
        }
        let stem = n.split('_').next().unwrap_or("").to_string();
        let state = if !n.contains('_') { "unknown" } else if live.contains(&stem) { "current" } else { "orphan" };
        if let Ok(m) = std::fs::metadata(&fp) {
            out.push(json!({"name": n, "stem": stem, "state": state, "size": m.len(),
                            "at": mtime(&fp).unwrap_or(0.0)}));
        }
    }
    out
}

pub fn tidy_exports(which: Option<&str>, dry_run: bool) -> Value {
    let rows = export_stems(which);
    let gone: Vec<&Value> = rows.iter().filter(|r| r["state"] == "orphan").collect();
    let bytes: u64 = gone.iter().map(|r| r["size"].as_u64().unwrap_or(0)).sum();
    if dry_run {
        return json!({"ok": true, "dryRun": true, "orphans": gone, "bytes": bytes,
                      "kept": rows.iter().filter(|r| r["state"] == "current").count(),
                      "unknown": rows.iter().filter(|r| r["state"] == "unknown").count()});
    }
    let d = export_dir(which);
    let trash = trash_dir(which);
    let _ = std::fs::create_dir_all(&trash);
    let (mut moved, mut failed) = (vec![], vec![]);
    for r in gone {
        let name = r["name"].as_str().unwrap_or("");
        let src = d.join(name);
        let dest = trash_dest(&trash, name);
        // atomic_replace copies across filesystems, which the exports folder
        // may well be on.
        match atomic_replace(&src, &dest) {
            Ok(()) => moved.push(json!(name)),
            Err(e) => failed.push(json!({"name": name, "error": e.to_string()})),
        }
    }
    json!({"ok": failed.is_empty(), "moved": moved, "failed": failed, "bytes": bytes,
           "recoverable": true})
}

/// Python str.format over the few fields a name template may use. Anything
/// it cannot resolve is an error, and the caller falls back.
fn format_template(tpl: &str, fields: &[(&str, String)]) -> Option<String> {
    let mut out = String::new();
    let mut it = tpl.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' if it.peek() == Some(&'{') => {
                it.next();
                out.push('{');
            }
            '}' if it.peek() == Some(&'}') => {
                it.next();
                out.push('}');
            }
            '{' => {
                let mut key = String::new();
                loop {
                    match it.next() {
                        Some('}') => break,
                        Some(ch) => key.push(ch),
                        None => return None,
                    }
                }
                let (name, spec) = key.split_once(':').unwrap_or((&key, ""));
                let v = fields.iter().find(|(k, _)| *k == name)?.1.clone();
                if spec.is_empty() {
                    out.push_str(&v);
                } else {
                    return None;
                }
            }
            '}' => return None,
            _ => out.push(c),
        }
    }
    Some(out)
}

/// The one place an export's filename is decided.
pub fn export_name(src_name: &str, pre: &Value, ext: Option<&str>) -> String {
    let fmt = pre.get("format").map(py_str).unwrap_or_default();
    let fmt = if fmt.is_empty() { "JPEG".to_string() } else { fmt.to_uppercase() };
    let stem = splitext(src_name).0;
    let id = pre.get("id").map(py_str).filter(|s| !s.is_empty()).unwrap_or_else(|| "export".into());
    static TAG: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let tag = TAG.get_or_init(|| re("[^a-z0-9]+")).replace_all(&id.to_lowercase(), "-").into_owned();
    let ext = ext.map(|e| e.to_string()).unwrap_or_else(|| match fmt.as_str() {
        "PNG" => ".png", "WEBP" => ".webp", "TIFF" => ".tif", _ => ".jpg" }.to_string());
    let tpl = cfg_str("name_template");
    let tpl = if tpl.is_empty() { "{stem}_{preset}".to_string() } else { tpl };
    let wh = |k: &str| pre.get(k).filter(|v| crate::app::truthy(Some(v))).map(py_str).unwrap_or_default();
    let base = format_template(&tpl, &[("stem", stem.to_string()), ("preset", tag.clone()),
                                       ("date", today()), ("w", wh("w")), ("h", wh("h"))])
        .unwrap_or_else(|| format!("{stem}_{tag}"));
    base.replace(['/', '\\'], "-") + &ext
}

/// str(x) as Python prints a JSON value: strings bare, numbers as written.
pub fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True".into() } else { "False".into() },
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn encode(im: &Rgb, fmt: &str, q: i32, subsampling: i32, dpi: i32) -> Result<Vec<u8>, String> {
    match fmt {
        "PNG" => img::png_encode(im, (dpi > 0).then_some(dpi as u32)),
        "TIFF" => img::tiff_encode(im),
        "WEBP" => Err("WEBP export is not supported by this build; use JPEG or PNG".into()),
        _ => img::jpeg_encode(im, &img::JpegOpts { quality: q, subsampling, optimize: true, dpi }),
    }
}

/// Encode, walk quality down to a size cap if the preset names one, and
/// publish atomically: a half-written export never sits under its real name.
pub fn write_out(im: &Rgb, src_name: &str, pre: &Value, outdir: &Path)
    -> Result<(String, PathBuf, u64), String> {
    let fmt = pre.get("format").map(py_str).unwrap_or_default().to_uppercase();
    let fmt = if fmt.is_empty() { "JPEG".to_string() } else { fmt };
    let out_name = export_name(src_name, pre, None);
    let out = outdir.join(&out_name);
    std::fs::create_dir_all(outdir).map_err(|e| e.to_string())?;
    let quality = crate::app::num(pre.get("quality"), 92.0) as i32;
    let dpi = if fmt == "JPEG" || fmt == "PNG" { crate::app::num(pre.get("dpi"), 0.0) as i32 } else { 0 };
    let tmp = staging_path(&out);
    let result = (|| -> Result<(), String> {
        let mut bytes = encode(im, &fmt, quality, 0, dpi)?;
        let cap = crate::app::num(pre.get("max_kb"), 0.0);
        if cap > 0.0 && (fmt == "JPEG" || fmt == "WEBP") {
            let cap = (cap as usize) * 1024;
            let mut q = quality;
            while bytes.len() > cap && q > 30 {
                q = (q - 6).max(30);
                bytes = encode(im, &fmt, q, if q < 70 { 2 } else { 0 }, dpi)?;
            }
            let mut shrink = im.clone();
            while bytes.len() > cap && shrink.w.min(shrink.h) > 400 {
                shrink = img::resize(&shrink, (shrink.w as f64 * 0.85) as usize,
                                     (shrink.h as f64 * 0.85) as usize, img::Filter::Lanczos);
                bytes = encode(&shrink, &fmt, q, 2, dpi)?;
            }
        }
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        atomic_replace(&tmp, &out).map_err(|e| e.to_string())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    Ok((out_name, out, size))
}

// ------------------------------------------------------------------ presets
pub fn presets() -> Result<Value, String> {
    read_json(&app().home.join("presets.json")).ok_or_else(|| "presets.json is unreadable".into())
}

/// Persist a look the user liked, so it joins the preset list.
pub fn save_look(mut look: Map<String, Value>) -> Result<String, String> {
    let _g = app().sidecar.lock();
    let path = app().home.join("presets.json");
    let mut data = presets()?;
    let label = look.get("label").map(py_str).unwrap_or_else(|| "custom".into());
    static ID: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let id = ID.get_or_init(|| re("[^a-z0-9]+")).replace_all(&label.to_lowercase(), "-")
        .trim_matches('-').to_string();
    let id = if id.is_empty() { "custom".to_string() } else { id };
    look.insert("id".into(), json!(id));
    let looks = data.get("look").and_then(|l| l.as_array()).cloned().unwrap_or_default();
    let mut kept: Vec<Value> = looks.into_iter().filter(|l| l.get("id").and_then(|v| v.as_str()) != Some(&id)).collect();
    kept.push(Value::Object(look));
    data["look"] = Value::Array(kept);
    write_json_atomic(&path, &data, 2).map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn cam_presets_path() -> PathBuf {
    app().home.join("camera-presets.json")
}

pub fn load_cam_presets() -> Map<String, Value> {
    match read_json(&cam_presets_path()) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

pub fn save_cam_presets(d: &Map<String, Value>) {
    let mut sorted: Vec<(&String, &Value)> = d.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let m: Map<String, Value> = sorted.into_iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let _ = write_json_atomic(&cam_presets_path(), &Value::Object(m), 1);
}
