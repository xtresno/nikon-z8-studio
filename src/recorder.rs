//! The YouTube recorder (`/record`): the page composes screen + camera + mic
//! and streams its MediaRecorder output here in small chunks, so a crashed
//! tab loses seconds, not the take. `finish` remuxes the raw stream into a
//! seekable file with ffmpeg (stream copy, no re-encode).
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

use crate::store::disk_ok;
use crate::tools::{need, run_timeout};

/// Takes go to `$Z8_REC_DIR`, default `~/Videos/z8-recordings`.
pub fn rec_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("Z8_REC_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Videos").join("z8-recordings")
}

/// A take id is the page's start stamp, `YYYYMMDD-HHMMSS`, nothing else.
fn id_ok(id: &str) -> bool {
    id.len() == 15 && id.char_indices().all(|(i, c)| if i == 8 { c == '-' } else { c.is_ascii_digit() })
}

fn ext_ok(ext: &str) -> bool {
    matches!(ext, "webm" | "mp4")
}

fn raw_path(id: &str, ext: &str) -> PathBuf {
    rec_dir().join(format!(".{id}.part.{ext}"))
}

/// Append one chunk at byte offset `off`. Idempotent: a retry of a chunk
/// that already landed (the reply was lost) answers ok without writing twice.
pub fn chunk(id: &str, ext: &str, off: u64, bytes: &[u8]) -> (u16, Value) {
    if !id_ok(id) || !ext_ok(ext) {
        return (400, json!({"ok": false, "error": "bad take id"}));
    }
    let dir = rec_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return (500, json!({"ok": false, "error": format!("cannot create {}", dir.display())}));
    }
    let (room, why) = disk_ok(&dir);
    if !room {
        return (507, json!({"ok": false, "error": why, "disk_full": true}));
    }
    let p = raw_path(id, ext);
    let have = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    let end = off + bytes.len() as u64;
    if have == end && off < end {
        return (200, json!({"ok": true, "size": have, "dup": true}));
    }
    if have != off {
        return (409, json!({"ok": false, "error": "chunk out of order", "size": have}));
    }
    use std::io::Write;
    let r = std::fs::OpenOptions::new().create(true).append(true).open(&p)
        .and_then(|mut f| f.write_all(bytes).and_then(|_| f.sync_data()));
    match r {
        Ok(()) => (200, json!({"ok": true, "size": end})),
        Err(e) => (500, json!({"ok": false, "error": e.to_string()})),
    }
}

/// Remux the raw stream into `<id>.mp4` (H.264) or `<id>.webm` (VP8/9), with
/// a proper index so editors and YouTube can seek. The raw file is removed
/// only once the remux succeeded and is not smaller than 90% of it.
pub fn finish(b: &Value) -> (u16, Value) {
    let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let ext = b.get("ext").and_then(|v| v.as_str()).unwrap_or("");
    if !id_ok(id) || !ext_ok(ext) {
        return (400, json!({"ok": false, "error": "bad take id"}));
    }
    let raw = raw_path(id, ext);
    let raw_size = std::fs::metadata(&raw).map(|m| m.len()).unwrap_or(0);
    if raw_size == 0 {
        return (404, json!({"ok": false, "error": "no data for this take"}));
    }
    let h264 = b.get("h264").and_then(|v| v.as_bool()).unwrap_or(ext == "mp4");
    let out_ext = if h264 { "mp4" } else { "webm" };
    let out = rec_dir().join(format!("yt-{id}.{out_ext}"));
    if let Err(e) = need("ffmpeg") {
        return (200, json!({"ok": true, "path": raw, "remuxed": false, "warning": e}));
    }
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-fflags", "+genpts", "-i"]).arg(&raw)
        .args(["-map", "0", "-c", "copy"]);
    if h264 {
        cmd.args(["-movflags", "+faststart"]);
    }
    cmd.arg(&out);
    let res = run_timeout(&mut cmd, Duration::from_secs(600));
    let out_size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    let good = matches!(&res, Ok(o) if o.status.success()) && out_size * 10 >= raw_size * 9;
    if !good {
        let err = match res {
            Ok(o) => String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("ffmpeg failed").to_string(),
            Err(e) => e,
        };
        let _ = std::fs::remove_file(&out);
        return (200, json!({"ok": true, "path": raw, "remuxed": false,
                            "warning": format!("kept the raw file, remux failed: {err}")}));
    }
    let _ = std::fs::remove_file(&raw);
    let secs = probe_seconds(&out);
    let mut v = json!({"ok": true, "path": out, "remuxed": true, "size": out_size, "seconds": secs});
    // The browser's audio track crackles whenever Chrome is starved of CPU
    // (it drops 21 ms buffers); the backup WAV does not. So every take also
    // gets `-wav` with the WAV laid under the video, and that is the file to
    // upload. The noise-removed copy is made from it too, never from Chrome.
    let wav_name = b.get("wav").and_then(|v| v.as_str()).map(str::to_string)
        .unwrap_or_else(|| format!("{id}.wav"));
    let mut clean_src = out.clone();
    match find_backup_wav(&wav_name) {
        // First choice: tools/sync-audio.mjs tracks the WAV's lag every 0.25 s,
        // fills only what the WAV lost from the browser track, and refuses to
        // write unless every 5 s checkpoint is within 2 ms (2026-10-20: a 6 min
        // take's WAV lost 3.5 s in cuts, which no single offset can fix).
        Some(wav) if sync_copy(&out, &wav, h264).is_some() => {
            let (p, info) = sync_copy_result(&out, h264);
            v["wav"] = json!(p);
            v["wavSync"] = info;
            clean_src = p;
        }
        Some(wav) => match wav_copy(&out, &wav, h264) {
            Ok((p, offset, measured)) => {
                v["wavOffset"] = json!((offset * 1000.0).round() / 1000.0);
                v["wavOffsetMeasured"] = json!(measured);
                // Safety net: the WAV must still line up near the END of the
                // take. If it lost audio part way it drifts, and the original
                // (browser) track is the better file to upload.
                let end_at = secs.map(|s| (s - 45.0).max(0.0)).unwrap_or(0.0);
                let end_off = if end_at > 30.0 { wav_offset_at(&out, &wav, end_at) } else { None };
                let drift = end_off.map(|e| e - offset);
                v["wavDriftMs"] = json!(drift.map(|d| (d * 1000.0).round()));
                if drift.is_some_and(|d| d.abs() > 0.04) {
                    v["wavDrifted"] = json!(true);
                    v["warning"] = json!(format!(
                        "the backup WAV drifted {:.0} ms out of sync by the end, so upload the original file",
                        drift.unwrap_or(0.0) * 1000.0));
                } else {
                    v["wav"] = json!(p);
                    clean_src = p;
                }
            }
            Err(e) => v["warning"] = json!(format!("no clean-audio copy: {e}")),
        },
        None => v["warning"] = json!(format!("no backup WAV {wav_name}, so no clean-audio copy")),
    }
    if b.get("clean").and_then(|v| v.as_bool()).unwrap_or(false) {
        let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("take").to_string();
        match clean_copy(&clean_src, &format!("{stem}-clean"), h264) {
            Ok(p) => v["clean"] = json!(p),
            Err(e) => v["warning"] = json!(format!("no cleaned copy: {e}")),
        }
    }
    (200, v)
}

/// Run tools/sync-audio.mjs; Some(stats) when it wrote a verified `-wav` file.
fn sync_copy(video: &std::path::Path, wav: &std::path::Path, h264: bool) -> Option<Value> {
    let tool = crate::app::app().home.join("tools/sync-audio.mjs");
    if !tool.is_file() {
        return None;
    }
    let (out, _) = sync_copy_result(video, h264);
    let mut cmd = std::process::Command::new("node");
    cmd.arg(&tool).arg(video).arg(wav).arg(&out);
    let o = run_timeout(&mut cmd, Duration::from_secs(1800)).ok()?;
    let line = String::from_utf8_lossy(&o.stdout).lines().last().unwrap_or("").to_string();
    let stats: Value = serde_json::from_str(&line).ok()?;
    if o.status.success() && stats["ok"].as_bool() == Some(true) && out.is_file() {
        let _ = std::fs::write(out.with_extension("sync.json"), &line);
        Some(stats)
    } else {
        let _ = std::fs::remove_file(&out);
        None
    }
}

/// The `-wav` path for a take and the stats sync_copy left beside it.
fn sync_copy_result(video: &std::path::Path, h264: bool) -> (PathBuf, Value) {
    let stem = video.file_stem().and_then(|s| s.to_str()).unwrap_or("take");
    let ext = if h264 { "mp4" } else { "webm" };
    let out = video.with_file_name(format!("{stem}-wav.{ext}"));
    let info = std::fs::read_to_string(out.with_extension("sync.json")).ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(Value::Null);
    (out, info)
}

/// The backup WAV by name, in one of the three newest session folders
/// (a take that crosses midnight lands in the previous day's folder).
fn find_backup_wav(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') || !name.ends_with(".wav") {
        return None;
    }
    let root = crate::app::home_dir().join("Pictures/z8-studio");
    let mut days: Vec<String> = std::fs::read_dir(&root).ok()?.filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.len() == 10 && n.starts_with("20")).collect();
    days.sort();
    days.into_iter().rev().take(3).map(|d| root.join(d).join(name)).find(|p| p.is_file())
}

/// Mono 8 kHz samples of a file's first audio stream, first `secs` seconds.
fn pcm8k(path: &std::path::Path, af: Option<&str>, start: f64, secs: u32) -> Option<Vec<f32>> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-v", "error", "-ss", &format!("{start:.3}"), "-t", &secs.to_string(), "-i"]).arg(path)
        .args(["-map", "0:a:0"]);
    if let Some(f) = af {
        cmd.args(["-af", f]);
    }
    cmd.args(["-ac", "1", "-ar", "8000", "-f", "s16le", "-"]);
    let o = run_timeout(&mut cmd, Duration::from_secs(120)).ok()?;
    if !o.status.success() {
        return None;
    }
    Some(o.stdout.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect())
}

/// Seconds to delay the WAV so it lines up with the video, measured by
/// cross-correlating 1 s windows of the browser track (on the container's
/// timeline) against the WAV over the first 30 s, median of the confident
/// windows. Takes measured 63, 81 and 130 ms, so a fixed number is wrong.
fn wav_offset(video: &std::path::Path, wav: &std::path::Path) -> Option<f64> {
    wav_offset_at(video, wav, 0.0)
}

/// The same measurement starting `at` seconds into the video. Comparing the
/// start with the end tells whether the WAV lost audio part way (drift).
fn wav_offset_at(video: &std::path::Path, wav: &std::path::Path, at: f64) -> Option<f64> {
    // Both cuts start at `at` on their own clocks, so the lag found is the
    // offset at that point of the take.
    let a = pcm8k(video, Some("aresample=async=1:first_pts=0"), at, 40)?;
    let w = pcm8k(wav, None, at, 42)?;
    const W: usize = 8000;
    const MAX: i64 = 8000;
    let mut lags = Vec::new();
    for t in (2..=30).step_by(2) {
        let s = t * 8000usize;
        if s + W > a.len() || s as i64 + W as i64 + MAX > w.len() as i64 {
            break;
        }
        let ea: f32 = a[s..s + W].iter().map(|x| x * x).sum();
        if ea / (W as f32) < 1e-5 {
            continue;
        }
        let corr = |l: i64, step: usize| -> f32 {
            let (mut c, mut ew, mut ea2) = (0f32, 0f32, 0f32);
            let mut i = 0;
            while i < W {
                let p = a[s + i];
                let q = w[(s as i64 + i as i64 + l) as usize];
                c += p * q;
                ew += q * q;
                ea2 += p * p;
                i += step;
            }
            c / (ea2 * ew + 1e-12).sqrt()
        };
        let (mut best, mut bl) = (-2f32, 0i64);
        let mut l = -(s as i64).min(MAX);
        while l <= MAX {
            let r = corr(l, 2);
            if r > best {
                best = r;
                bl = l;
            }
            l += 2;
        }
        for l in (bl - 2)..=(bl + 2) {
            if s as i64 + l < 0 {
                continue;
            }
            let r = corr(l, 1);
            if r > best {
                best = r;
                bl = l;
            }
        }
        if best > 0.8 {
            lags.push(bl);
        }
    }
    if lags.len() < 3 {
        return None;
    }
    lags.sort();
    Some(-(lags[lags.len() / 2] as f64) / 8000.0)
}

/// `<take>-wav.<ext>`: the take's video with the backup WAV as its audio.
/// Returns the file, the offset used, and whether it was measured.
fn wav_copy(video: &std::path::Path, wav: &std::path::Path, h264: bool) -> Result<(PathBuf, f64, bool), String> {
    let measured = wav_offset(video, wav);
    // 0.081 s: the offset measured on 2026-10-18, the fallback for a take
    // too quiet to correlate.
    let offset = measured.unwrap_or(0.081);
    let stem = video.file_stem().and_then(|s| s.to_str()).unwrap_or("take");
    let ext = if h264 { "mp4" } else { "webm" };
    let out = video.with_file_name(format!("{stem}-wav.{ext}"));
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(video)
        .args(["-itsoffset", &format!("{offset:.4}"), "-i"]).arg(wav)
        .args(["-map", "0:v", "-map", "1:a", "-c:v", "copy", "-af", "highpass=f=60"]);
    if h264 {
        cmd.args(["-c:a", "aac", "-b:a", "256k", "-movflags", "+faststart"]);
    } else {
        cmd.args(["-c:a", "libopus", "-b:a", "192k"]);
    }
    cmd.arg(&out);
    match run_timeout(&mut cmd, Duration::from_secs(1800)) {
        Ok(o) if o.status.success() && out.is_file() => Ok((out, offset, measured.is_some())),
        Ok(o) => {
            let _ = std::fs::remove_file(&out);
            Err(String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("ffmpeg failed").to_string())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&out);
            Err(e)
        }
    }
}

/// The RNNoise speech model, `$Z8_RNNOISE` or ~/.local/share/z8-studio/rnnoise_sh.rnnn.
fn rnnoise_model() -> Option<PathBuf> {
    let p = std::env::var_os("Z8_RNNOISE").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join(".local/share/z8-studio/rnnoise_sh.rnnn")
    });
    p.is_file().then_some(p)
}

/// A second file with the room hiss removed; the original is never touched.
/// Measured on the 2026-10-11 self-tape (AT2020, Scarlett): pauses -48.6 to
/// -65.6 dB, speech -24.2 to -25.3 dB. mix=0.9 keeps a trace of the room so
/// the pauses do not sound gated. afftdn left the hiss almost unchanged (-0.8 dB).
fn clean_copy(src: &std::path::Path, out_stem: &str, h264: bool) -> Result<PathBuf, String> {
    let model = rnnoise_model().ok_or("the noise model is missing (~/.local/share/z8-studio/rnnoise_sh.rnnn)")?;
    let m = model.to_string_lossy().replace('\\', "\\\\").replace('\'', "\\'").replace(':', "\\:");
    let ext = if h264 { "mp4" } else { "webm" };
    let out = src.with_file_name(format!("{out_stem}.{ext}"));
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(src)
        .args(["-map", "0", "-c:v", "copy", "-af", &format!("highpass=f=70,arnndn=m={m}:mix=0.9")]);
    if h264 {
        cmd.args(["-c:a", "aac", "-b:a", "256k", "-movflags", "+faststart"]);
    } else {
        cmd.args(["-c:a", "libopus", "-b:a", "192k"]);
    }
    cmd.arg(&out);
    match run_timeout(&mut cmd, Duration::from_secs(1800)) {
        Ok(o) if o.status.success() && out.is_file() => Ok(out),
        Ok(o) => {
            let _ = std::fs::remove_file(&out);
            Err(String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("ffmpeg failed").to_string())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&out);
            Err(e)
        }
    }
}

fn probe_seconds(p: &std::path::Path) -> Option<f64> {
    need("ffprobe").ok()?;
    let mut cmd = std::process::Command::new("ffprobe");
    cmd.args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(p);
    let o = run_timeout(&mut cmd, Duration::from_secs(30)).ok()?;
    String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok().map(|s| (s * 10.0).round() / 10.0)
}

/// What the page needs to pick devices: the PipeWire default mic's name as a
/// browser shows it ("Scarlett Solo 4th Gen Input 1 Inst/Line").
pub fn info() -> Value {
    let text = crate::tools::run_text(&["wpctl", "inspect", "@DEFAULT_AUDIO_SOURCE@"], 6);
    let desc = text.lines().find(|l| l.contains("node.description"))
        .and_then(|l| l.split_once('=')).map(|(_, v)| v.trim().trim_matches('"').to_string());
    json!({"ok": true, "micLabel": desc, "dir": rec_dir()})
}

/// The newest teleprompter script, written by the video script pipeline to
/// ~/Pictures/z8-studio/scripts/latest.txt (title in latest.name). `at` is the
/// file's mtime in ms so the page loads each new script exactly once.
pub fn script() -> Value {
    let dir = crate::app::home_dir().join("Pictures/z8-studio/scripts");
    let file = dir.join("latest.txt");
    let Ok(text) = std::fs::read_to_string(&file) else {
        return json!({"ok": true, "text": null, "at": 0, "path": file});
    };
    let at = std::fs::metadata(&file).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64).unwrap_or(0);
    let name = std::fs::read_to_string(dir.join("latest.name")).ok()
        .map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    json!({"ok": true, "text": text, "at": at, "name": name, "charts": script_charts(), "path": file})
}

/// Publishing kits for the Videos page: every `*.json` in
/// ~/Pictures/z8-studio/videos, newest file name first. Each file is one video
/// (titles, A/B sets, thumbnail prompts, description); a file that does not
/// parse is listed with its error instead of hiding the others.
pub fn videos() -> Value {
    let dir = crate::app::home_dir().join("Pictures/z8-studio/videos");
    let mut names: Vec<String> = std::fs::read_dir(&dir).map(|rd| rd.filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".json")).collect()).unwrap_or_default();
    names.sort();
    names.reverse();
    let items: Vec<Value> = names.iter().map(|n| match std::fs::read_to_string(dir.join(n)) {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(v) => json!({"file": n, "video": v}),
            Err(e) => json!({"file": n, "error": e.to_string()}),
        },
        Err(e) => json!({"file": n, "error": e.to_string()}),
    }).collect();
    json!({"ok": true, "dir": dir, "videos": items})
}

/// Load one prepared script into the recorder: copy
/// ~/Pictures/z8-studio/scripts/by-video/<slug>/latest.{name,charts.json,checks.json,txt}
/// over scripts/latest.*, writing latest.txt LAST because the recorder keys on its time.
/// The slug is a single path segment (letters, digits, - and _ only).
pub fn load_video_script(b: &Value) -> Value {
    let slug = b.get("slug").and_then(|v| v.as_str()).unwrap_or("");
    if slug.is_empty() || !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return json!({"ok": false, "error": "bad slug"});
    }
    let base = crate::app::home_dir().join("Pictures/z8-studio/scripts");
    let src = base.join("by-video").join(slug);
    if !src.join("latest.txt").is_file() {
        return json!({"ok": false, "error": format!("no prepared script for {slug}")});
    }
    for f in ["latest.name", "latest.charts.json", "latest.checks.json", "latest.check.json", "latest.txt"] {
        let from = src.join(f);
        if !from.is_file() { continue; }
        let tmp = base.join(format!(".{f}.tmp"));
        if let Err(e) = std::fs::copy(&from, &tmp).and_then(|_| std::fs::rename(&tmp, base.join(f))) {
            return json!({"ok": false, "error": format!("{f}: {e}")});
        }
    }
    let name = std::fs::read_to_string(src.join("latest.name")).unwrap_or_default();
    json!({"ok": true, "loaded": slug, "name": name.trim()})
}

/// The links the newest script shows, in order, from latest.charts.json:
/// `[{"title": "...", "url": "https://example.com/charts/<id>", "note": "..."}]`.
/// Only links under Z8_LINK_PREFIX (for example `https://example.com/`) are kept,
/// because Open all hands them to the browser. Unset = no links.
pub fn script_charts() -> Vec<Value> {
    let Some(prefix) = std::env::var("Z8_LINK_PREFIX").ok().filter(|p| p.starts_with("https://")) else {
        return vec![];
    };
    let file = crate::app::home_dir().join("Pictures/z8-studio/scripts/latest.charts.json");
    let Ok(raw) = std::fs::read_to_string(&file) else { return vec![] };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&raw) else { return vec![] };
    items.into_iter().filter(|c| c["url"].as_str()
        .is_some_and(|u| u.starts_with(&prefix))).collect()
}

/// The latest numbers check, written by your scripts repo's
/// scripts/youtube/video-check.mjs as latest.check.json next to the script.
pub fn check_result() -> Value {
    let f = crate::app::home_dir().join("Pictures/z8-studio/scripts/latest.check.json");
    let check = std::fs::read_to_string(&f).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
    json!({"ok": true, "check": check})
}

/// Re-pull every number the newest script quotes. The checker lives in your
/// scripts repo (Z8_SCRIPTS_REPO, default ~/projects/video-scripts).
pub fn run_check() -> (u16, Value) {
    let repo = scripts_repo();
    let script = repo.join("scripts/youtube/video-check.mjs");
    if !script.exists() {
        return (500, json!({"ok": false, "error": format!("{} is missing", script.display())}));
    }
    // The service PATH has no nvm, and Ubuntu's own node cannot run everything the repo uses.
    let node = ["/usr/local/bin/node", "/usr/bin/node"].iter().map(PathBuf::from)
        .find(|p| p.exists()).unwrap_or_else(|| PathBuf::from("node"));
    let mut cmd = std::process::Command::new(node);
    cmd.arg(&script).current_dir(&repo);
    match run_timeout(&mut cmd, Duration::from_secs(150)) {
        Ok(o) if o.status.success() => (200, check_result()),
        Ok(o) => (500, json!({"ok": false, "error": String::from_utf8_lossy(&o.stderr).trim().to_string()})),
        Err(e) => (500, json!({"ok": false, "error": e})),
    }
}

fn screens(cmd: &str) -> Value {
    let tool = crate::app::app().home.join("tools/screens.py");
    let mut c = std::process::Command::new("python3");
    c.arg(&tool).arg(cmd);
    match run_timeout(&mut c, Duration::from_secs(15)) {
        Ok(o) => serde_json::from_slice(&o.stdout)
            .unwrap_or_else(|_| json!({"ok": false, "error": String::from_utf8_lossy(&o.stderr).trim().to_string()})),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

/// Software unplug/replug of the Scarlett, for when the AMD controller drops
/// its capture stream. The page calls this the moment it hears pure silence.
pub fn reset_mic() -> (u16, Value) {
    let tool = crate::app::app().home.join("tools/reset-mic.py");
    let mut c = std::process::Command::new("python3");
    c.arg(&tool);
    match run_timeout(&mut c, Duration::from_secs(10)) {
        Ok(o) => {
            let v: Value = serde_json::from_slice(&o.stdout)
                .unwrap_or_else(|_| json!({"ok": false, "error": String::from_utf8_lossy(&o.stderr).trim().to_string()}));
            (if v["ok"].as_bool() == Some(true) { 200 } else { 500 }, v)
        }
        Err(e) => (500, json!({"ok": false, "error": e})),
    }
}

/// Put the monitors in the studio layout (big ones side by side, Elgato below the seam).
pub fn fix_layout() -> (u16, Value) {
    let v = screens("fix");
    (if v["ok"].as_bool() == Some(true) { 200 } else { 500 }, v)
}

/// Which USB controller the Scarlett is on, judged by its history on this box.
/// Every mic death logs "Transfer event for disabled endpoint" on the Scarlett's
/// own controller first: 31:00.3 three times (09-23, shared with the Elgato and
/// Z8) and 2a:00.1 three times (10-14, 10-15). 2a:00.3 carries only the RGB
/// controller and an internal hub, and is the one it has never died on.
fn mic_port() -> Value {
    let mut ctrl: Option<String> = None;
    if let Ok(rd) = std::fs::read_dir("/sys/bus/usb/devices") {
        for e in rd.flatten() {
            let p = e.path();
            if std::fs::read_to_string(p.join("idVendor")).map(|v| v.trim() == "1235").unwrap_or(false) {
                let real = std::fs::canonicalize(&p).unwrap_or(p);
                ctrl = real.to_string_lossy().split('/')
                    .filter(|seg| seg.len() == 12 && seg.starts_with("0000:") && seg.contains('.'))
                    .last().map(str::to_string);
                break;
            }
        }
    }
    let (level, why) = match ctrl.as_deref() {
        None => ("bad", "The Scarlett is not connected."),
        Some("0000:2a:00.3") => ("good", "On the quiet USB controller it has never dropped out on."),
        Some("0000:2a:00.1") => ("bad", "On the USB controller where it went silent 3 times (10-14, 10-15). Move its cable to another port."),
        Some("0000:31:00.3") => ("bad", "Shares a USB controller with the Elgato and Z8, where it dropped out 3 times. Move its cable to another port."),
        Some(_) => ("warn", "On a USB controller it has not been tested on."),
    };
    json!({"controller": ctrl, "level": level, "why": why})
}

/// The server half of the recorder's "Studio ready" card: everything the page
/// cannot see from the browser. The page adds camera, mic level, screen share
/// and the prompter window, which only it knows.
pub fn studio_ready() -> Value {
    let mic = crate::tools::run_text(&["wpctl", "status"], 6).contains("Scarlett");
    let dir = rec_dir();
    let _ = std::fs::create_dir_all(&dir);
    let free_gb = crate::tools::run_text(&["df", "-B1", "--output=avail", &dir.to_string_lossy()], 6)
        .lines().nth(1).and_then(|l| l.trim().parse::<f64>().ok()).map(|b| b / 1e9);
    let load = std::fs::read_to_string("/proc/loadavg").ok()
        .and_then(|l| l.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()));
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let top: Vec<Value> = crate::tools::run_text(&["ps", "-eo", "pcpu,comm", "--sort=-pcpu", "--no-headers"], 6)
        .lines().filter_map(|l| {
            let mut it = l.split_whitespace();
            let cpu = it.next()?.parse::<f64>().ok()?;
            let name = it.collect::<Vec<_>>().join(" ");
            // ps measures itself over its own tiny lifetime and always looks busy.
            (name != "ps").then(|| json!({"cpu": cpu, "name": name}))
        }).take(3).collect();
    let s = script();
    let check = check_result()["check"].clone();
    json!({
        "ok": true,
        "screens": screens("status"),
        "micDevice": mic,
        "micPort": mic_port(),
        "diskFreeGb": free_gb,
        "load": load, "cpus": cpus, "topCpu": top,
        "script": {"loaded": s["text"].is_string(), "name": s["name"], "charts": s["charts"].as_array().map(|a| a.len()).unwrap_or(0)},
        "check": check,
    })
}

/// Open every chart of the newest script as browser tabs, in script order.
pub fn open_charts() -> (u16, Value) {
    let urls: Vec<String> = script_charts().iter()
        .filter_map(|c| c["url"].as_str().map(str::to_string)).collect();
    if urls.is_empty() {
        return (404, json!({"ok": false, "error": "this script has no chart list"}));
    }
    if let Err(e) = need("xdg-open") {
        return (500, json!({"ok": false, "error": e}));
    }
    let n = urls.len();
    // One at a time with a gap: launched together, the browser opens tabs in a random order.
    std::thread::spawn(move || for u in urls {
        let _ = std::process::Command::new("xdg-open").arg(&u)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
        std::thread::sleep(Duration::from_millis(700));
    });
    (200, json!({"ok": true, "opened": n}))
}

/// Open the recordings folder in the file manager.
pub fn open_dir() -> (u16, Value) {
    let d = rec_dir();
    let _ = std::fs::create_dir_all(&d);
    if let Err(e) = need("xdg-open") {
        return (500, json!({"ok": false, "error": e}));
    }
    match std::process::Command::new("xdg-open").arg(&d).stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null()).spawn() {
        Ok(_) => (200, json!({"ok": true, "path": d})),
        Err(e) => (500, json!({"ok": false, "error": e.to_string()})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(id_ok("20261015-143000"));
        assert!(!id_ok("20261015-14300"));
        assert!(!id_ok("../../etc/passw"));
        assert!(!id_ok("20261015x143000"));
        assert!(ext_ok("webm") && ext_ok("mp4") && !ext_ok("sh"));
    }
}


// ---- Publishing pipeline (Videos page buttons) ----------------------------
// The work lives in your scripts repo's scripts/youtube/{transcribe,thumbs,publish}.mjs;
// these endpoints only start them and edit the kit's picks. Each script reports
// progress into the kit's `jobs` field, which the page re-reads.

/// Where the optional video scripts live (check, transcribe, thumbs, publish).
fn scripts_repo() -> PathBuf {
    std::env::var_os("Z8_SCRIPTS_REPO").map(PathBuf::from)
        .unwrap_or_else(|| crate::app::home_dir().join("projects/video-scripts"))
}

fn kit_file(slug: &str) -> Option<PathBuf> {
    if slug.is_empty() || !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let f = crate::app::home_dir().join("Pictures/z8-studio/videos").join(format!("{slug}.json"));
    f.is_file().then_some(f)
}

fn edit_kit(slug: &str, change: impl FnOnce(&mut Value)) -> Result<Value, String> {
    let f = kit_file(slug).ok_or("no such video")?;
    let mut kit: Value = serde_json::from_str(&std::fs::read_to_string(&f).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    change(&mut kit);
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(&kit).unwrap_or_default()))
        .and_then(|_| std::fs::rename(&tmp, &f)).map_err(|e| e.to_string())?;
    Ok(kit)
}

/// POST {slug, action: transcribe|thumbs|publish, args?: [..]}: start the script, return at once.
pub fn run_kit_job(b: &Value) -> Value {
    let slug = b.get("slug").and_then(|v| v.as_str()).unwrap_or("");
    let action = b.get("action").and_then(|v| v.as_str()).unwrap_or("");
    if kit_file(slug).is_none() {
        return json!({"ok": false, "error": "no such video"});
    }
    if !matches!(action, "transcribe" | "thumbs" | "publish" | "sync-tascam") {
        return json!({"ok": false, "error": "unknown action"});
    }
    let repo = scripts_repo();
    let script = repo.join(format!("scripts/youtube/{action}.mjs"));
    if !script.is_file() {
        return json!({"ok": false, "error": format!("missing {}", script.display())});
    }
    let args: Vec<String> = b.get("args").and_then(|v| v.as_array()).map(|a| a.iter()
        .filter_map(|x| x.as_str()).filter(|x| x.starts_with("--") || !x.contains('/'))
        .map(String::from).collect()).unwrap_or_default();
    let logs = crate::app::home_dir().join(".cache/z8-jobs");
    let _ = std::fs::create_dir_all(&logs);
    let log = logs.join(format!("{slug}-{action}.log"));
    let Ok(fh) = std::fs::File::create(&log) else { return json!({"ok": false, "error": "cannot write the job log"}) };
    let Ok(fh2) = fh.try_clone() else { return json!({"ok": false, "error": "cannot write the job log"}) };
    let _ = edit_kit(slug, |k| {
        k["jobs"][action] = json!({"state": "running", "message": "Starting", "at": chrono::Utc::now().to_rfc3339()});
    });
    let spawned = std::process::Command::new("nice").args(["-n", "10", "node"]).arg(&script).arg(slug).args(&args)
        .current_dir(&repo).stdin(std::process::Stdio::null()).stdout(fh).stderr(fh2).spawn();
    match spawned {
        Ok(child) => {
            // Reap it so a finished job does not linger as a zombie.
            std::thread::spawn(move || { let mut c = child; let _ = c.wait(); });
            json!({"ok": true, "log": log})
        }
        Err(e) => json!({"ok": false, "error": e.to_string()}),
    }
}

/// POST {slug, thumbnail?: path (toggle, max 3 in A/B/C order), title?: string}.
pub fn pick_kit(b: &Value) -> Value {
    let slug = b.get("slug").and_then(|v| v.as_str()).unwrap_or("");
    let thumb = b.get("thumbnail").and_then(|v| v.as_str()).map(String::from);
    let title = b.get("title").and_then(|v| v.as_str()).map(String::from);
    match edit_kit(slug, |k| {
        if let Some(t) = thumb {
            let mut picks: Vec<String> = k["picked_thumbnails"].as_array().map(|a| a.iter()
                .filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            if let Some(i) = picks.iter().position(|p| *p == t) { picks.remove(i); } else if picks.len() < 3 { picks.push(t); }
            k["picked_thumbnails"] = json!(picks);
        }
        if let Some(t) = title {
            k["picked_title"] = json!(t);
        }
    }) {
        Ok(k) => json!({"ok": true, "picked_thumbnails": k["picked_thumbnails"], "picked_title": k["picked_title"]}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

/// Bytes of a generated thumbnail or a headshot; any other path is refused.
pub fn kit_image(p: &str) -> Option<(Vec<u8>, &'static str)> {
    let root = crate::app::home_dir().join("Pictures/z8-studio");
    let path = std::fs::canonicalize(p).ok()?;
    let allowed = ["thumbnails", "headshots"].iter()
        .any(|d| std::fs::canonicalize(root.join(d)).map(|r| path.starts_with(r)).unwrap_or(false));
    if !allowed {
        return None;
    }
    let ctype = match path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        _ => return None,
    };
    Some((std::fs::read(&path).ok()?, ctype))
}
