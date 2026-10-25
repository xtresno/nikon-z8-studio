//! The microphone: an honest level check, the parallel recorder for skits
//! (state held HERE, not in the page), its runaway guard, and clap detection.
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use parking_lot::Mutex;
use regex::Regex;
use serde_json::{json, Value};

use crate::app::{app, now, round};
use crate::load::which;
use crate::store::{disk_ok, session_dir, wav_seconds};
use crate::tools::{run_text, run_timeout};

pub struct Rec {
    pub proc: Option<Child>,
    pub path: Option<PathBuf>,
    pub name: Option<String>,
    pub at: f64,
    pub stopped_because: Option<String>,
}

pub static REC: Mutex<Rec> = Mutex::new(Rec { proc: None, path: None, name: None, at: 0.0, stopped_because: None });

fn running(r: &mut Rec) -> bool {
    match r.proc.as_mut() {
        Some(p) => matches!(p.try_wait(), Ok(None)),
        None => false,
    }
}

pub fn audio_rec_status() -> Value {
    let mut r = REC.lock();
    if !running(&mut r) {
        return Value::Null;
    }
    let path = r.path.clone();
    let mb = path.as_ref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len() as f64 / 1048576.0).unwrap_or(0.0);
    json!({"name": r.name, "seconds": round(now() - r.at, 1), "mb": round(mb, 1),
           "limit": app().audio_max_seconds})
}

fn stop(r: &mut Rec) {
    if let Some(mut p) = r.proc.take() {
        unsafe { libc::kill(p.id() as i32, libc::SIGTERM) };
        for _ in 0..100 {
            if matches!(p.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = p.kill();
        let _ = p.wait();
    }
}

/// A take left running overnight is about 4.3 GB: stop it at the limit, or
/// early when the disk gets tight.
pub fn watchdog() {
    loop {
        std::thread::sleep(Duration::from_secs(15));
        let mut r = REC.lock();
        if !running(&mut r) {
            continue;
        }
        let age = now() - r.at;
        let dir = r.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
        let why = if age > app().audio_max_seconds as f64 {
            Some(format!("reached the {}s limit", app().audio_max_seconds))
        } else if !disk_ok(&dir).0 {
            Some("the disk is nearly full".into())
        } else {
            None
        };
        if let Some(w) = why {
            eprintln!("[z8] stopping the audio take: {w}");
            stop(&mut r);
            r.stopped_because = Some(w);
        }
    }
}

pub fn default_source() -> Option<String> {
    let re = Regex::new(r"(alsa_input\.\S+)").expect("re");
    for line in run_text(&["wpctl", "status"], 6).lines() {
        if line.contains('*') && line.contains("alsa_input") {
            if let Some(m) = re.captures(line) {
                return Some(m[1].to_string());
            }
        }
    }
    None
}

/// The hardware node and channel a loopback-split source reads from, from
/// pw-dump: the `<source>.split` stream's `target.object` and its single
/// `audio.position` (e.g. alsa_input.hw_Gen_0, AUX0).
fn hw_channel_for(src: &str) -> Option<(String, String)> {
    let o = run_timeout(Command::new("pw-dump").stdin(Stdio::null()), Duration::from_secs(8)).ok()?;
    let all: Value = serde_json::from_slice(&o.stdout).ok()?;
    let want = format!("{src}.split");
    for n in all.as_array()? {
        let p = &n["info"]["props"];
        if p["node.name"].as_str() != Some(want.as_str()) {
            continue;
        }
        let hw = p["target.object"].as_str()?.to_string();
        let pos = p["audio.position"].as_str()?.trim_matches(|c| c == '[' || c == ']' || c == ' ').to_string();
        if hw.is_empty() || pos.is_empty() || pos.contains(',') {
            return None;
        }
        return Some((hw, pos));
    }
    None
}

/// Record a moment and measure it: (signal, peak %, why). signal is None
/// when it genuinely cannot tell.
pub fn measure_input_level(seconds: f64) -> (Option<bool>, Option<f64>, Option<String>) {
    let re = Regex::new(r"(alsa_input\.\S+)").expect("re");
    let mut src = None;
    for line in run_text(&["wpctl", "status"], 6).lines() {
        if line.contains('*') && line.contains("alsa_input") {
            src = re.captures(line).map(|m| m[1].to_string());
            break;
        }
    }
    let Some(src) = src else { return (None, None, Some("no default input to record from".into())) };
    let tmp = app().cache_tmp.join("miclevel.raw");
    let _ = std::fs::create_dir_all(&app().cache_tmp);
    if which("pw-record").is_none() {
        return (None, None, Some("pw-record is not installed, so the level cannot be measured. \
                                  Install it with: sudo apt install pipewire-bin".into()));
    }
    let r = run_timeout(Command::new("timeout").arg(format!("{}", seconds as i64 + 3))
        .args(["pw-record", "--target", &src, "--rate", "48000", "--channels", "1", "--format", "s16",
               "--latency", "50ms"]).arg(&tmp), Duration::from_secs_f64(seconds + 10.0));
    let r = match r {
        Ok(o) => o,
        Err(e) => return (None, None, Some(format!("could not record from the input: {e}"))),
    };
    let data = std::fs::read(&tmp).unwrap_or_default();
    if data.len() < 2000 {
        let err = String::from_utf8_lossy(&r.stderr).trim().lines().last().map(|s| s.to_string());
        return (None, None, Some(err.unwrap_or_else(|| "could not record from the input".into())));
    }
    let mut peak = 0i32;
    let mut i = 0;
    while i + 1 < data.len() {
        let v = i16::from_le_bytes([data[i], data[i + 1]]) as i32;
        peak = peak.max(v.abs());
        i += 64;
    }
    let _ = std::fs::remove_file(&tmp);
    let pct = peak as f64 / 32768.0 * 100.0;
    (Some(pct > 0.5), Some(round(pct, 3)), None)
}

/// What the microphone chain looks like. The level meter is informational
/// only: it mixes capture and playback, and once read 4095 off a playback
/// channel while the mic was dead.
pub fn audio_status() -> Value {
    let mut out = json!({"source": null, "volume": null, "muted": null, "cards": [], "meter": null,
                         "signal": null, "phantom": null, "hints": []});
    let st = run_text(&["wpctl", "status"], 6);
    let nid_re = Regex::new(r"(\d+)\.\s").expect("re");
    let vol_re = Regex::new(r"Volume:\s*([0-9.]+)").expect("re");
    for line in st.lines() {
        if line.contains('*') && line.contains("alsa_input") {
            out["source"] = json!(line.split_once('.').map(|(_, r)| r.trim()).unwrap_or(line.trim()));
            if let Some(n) = nid_re.captures(line) {
                let v = run_text(&["wpctl", "get-volume", &n[1]], 6);
                if let Some(m) = vol_re.captures(&v) {
                    out["volume"] = json!(m[1].parse::<f64>().unwrap_or(0.0));
                }
                out["muted"] = json!(v.contains("MUTED"));
            }
            break;
        }
    }
    let card_re = Regex::new(r"^\s*(\d+)\s+\[(\S+)\s*\]:\s*(.*)").expect("re");
    let mut cards = Vec::new();
    for line in std::fs::read_to_string("/proc/asound/cards").unwrap_or_default().lines() {
        if let Some(m) = card_re.captures(line) {
            cards.push(json!({"index": &m[1], "id": &m[2], "name": m[3].trim()}));
        }
    }
    let meter_re = Regex::new(r"(?s)name='Level Meter'.*?\n.*?\n\s*:\s*values=([0-9,]+)").expect("re");
    let phantom_re = Regex::new(r"(?s)Phantom Power.*?\n.*?\n\s*:\s*values=(\w+)").expect("re");
    for card in &cards {
        let id = card["id"].as_str().unwrap_or("");
        let conts = run_text(&["amixer", "-c", id, "contents"], 8);
        if let Some(m) = meter_re.captures(&conts) {
            let vals: Vec<i64> = m[1].split(',').filter(|x| !x.is_empty()).filter_map(|x| x.parse().ok()).collect();
            out["meter"] = json!({"card": card["name"], "values": vals, "max": 4095});
        }
        if let Some(m) = phantom_re.captures(&conts) {
            out["phantom"] = json!(m[1].contains("on"));
        }
    }
    out["cards"] = json!(cards);
    let mut hints: Vec<String> = Vec::new();
    if out["muted"] == json!(true) {
        hints.push("the input is muted in the system mixer".into());
    }
    if out["volume"].as_f64().map(|v| v < 0.2).unwrap_or(false) {
        hints.push("the system input volume is very low".into());
    }
    let (signal, level, why) = measure_input_level(1.0);
    out["signal"] = json!(signal);
    out["level"] = json!(level);
    out["why"] = json!(why);
    if signal == Some(false) {
        hints.push("no sound is reaching the computer: turn up the interface's GAIN knob, check the \
                    cable is seated at both ends, and check the microphone's own switch".into());
    }
    if out["phantom"] == json!(false) {
        hints.push("phantom power is off, which a condenser microphone needs".into());
    }
    out["hints"] = json!(hints);
    out
}

/// Start or stop the parallel recording. (http status, body)
pub fn record(want_on: bool) -> (u16, Value) {
    let mut r = REC.lock();
    let is_running = running(&mut r);
    if want_on && is_running {
        return (200, json!({"ok": true, "recording": true, "name": r.name, "note": "already recording"}));
    }
    if !want_on {
        if !is_running {
            return (409, json!({"ok": false, "error": "not recording"}));
        }
        stop(&mut r);
        let path = r.path.clone().unwrap_or_default();
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let secs = wav_seconds(&path);
        return (200, json!({"ok": size > 1000, "recording": false, "name": crate::store::fname(&path),
                            "path": path, "size": size, "seconds": secs,
                            "error": if size > 1000 { Value::Null } else { json!("the recording is empty, check the input") }}));
    }
    if let Err(e) = crate::tools::need("pw-record") {
        return (503, json!({"ok": false, "error": e}));
    }
    let Some(src) = default_source() else {
        return (409, json!({"ok": false, "error": "no audio input to record from"}));
    };
    let d = session_dir(None);
    let (ok, why) = disk_ok(&d);
    if !ok {
        return (507, json!({"ok": false, "error": why, "disk_full": true}));
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut name = format!("{stamp}.wav");
    let mut n = 2;
    while d.join(&name).exists() {
        name = format!("{stamp}-{n}.wav");
        n += 1;
    }
    let path = d.join(&name);
    // stderr to a FILE: an unread pipe fills at 64 KB and blocks the recorder mid-take.
    let _ = std::fs::create_dir_all(&app().cache_tmp);
    let errlog = app().cache_tmp.join("pw-record.err");
    let Ok(efh) = std::fs::File::create(&errlog) else {
        return (500, json!({"ok": false, "error": "cannot write the recorder log"}));
    };
    // Realtime priority: with every CPU busy, a normal-priority pw-record lost
    // 1.57 s of every 45 s (3.5%, heard as crackle); at SCHED_FIFO 20 it kept
    // up (measured 2026-10-19). Falls back to normal priority if RT is refused.
    // Record the HARDWARE channel behind the default source, not the source
    // itself: on the Scarlett the default "Input 1" is a PipeWire loopback
    // split of the 4-channel device, and that loopback drops whole 1024-sample
    // quanta when it resyncs (a 94 s take lost 0.52 s in 4 cuts on
    // 2026-10-19, so the WAV drifted out of sync). Same signal, measured
    // r = 1.000, one quantum earlier. Falls back to the source if no split.
    let (target, map) = match hw_channel_for(&src) {
        Some((hw, ch)) => (hw, Some(ch)),
        None => (src.clone(), None),
    };
    let spawn = |rt: bool, log: std::fs::File| {
        let mut c = if rt { let mut c = Command::new("chrt"); c.args(["-f", "20", "pw-record"]); c }
                    else { Command::new("pw-record") };
        c.args(["--target", &target, "--rate", "48000", "--format", "s24"]);
        match &map {
            // Two identical channels of that one hardware input.
            Some(ch) => { c.args(["--channel-map", ch, "-P", "{ stream.dont-remix = true }"]); }
            None => { c.args(["--channels", "1"]); }
        }
        c.arg(&path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log).spawn()
    };
    let rt = which("chrt").is_some();
    let mut child = match spawn(rt, efh) {
        Ok(c) => c,
        Err(e) => return (500, json!({"ok": false, "error": e.to_string()})),
    };
    std::thread::sleep(Duration::from_millis(600));
    if rt && matches!(child.try_wait(), Ok(Some(_))) {
        let _ = std::fs::remove_file(&path);
        let Ok(efh) = std::fs::File::create(&errlog) else {
            return (500, json!({"ok": false, "error": "cannot write the recorder log"}));
        };
        child = match spawn(false, efh) {
            Ok(c) => c,
            Err(e) => return (500, json!({"ok": false, "error": e.to_string()})),
        };
        std::thread::sleep(Duration::from_millis(600));
    }
    if let Ok(Some(_)) = child.try_wait() {
        let err = std::fs::read_to_string(&errlog).unwrap_or_default();
        let last = err.trim().lines().last().map(|s| s.to_string()).unwrap_or_else(|| "pw-record exited".into());
        return (500, json!({"ok": false, "error": last}));
    }
    r.proc = Some(child);
    r.path = Some(path);
    r.name = Some(name.clone());
    r.at = now();
    (200, json!({"ok": true, "recording": true, "name": name, "source": src}))
}

/// Seconds to the sharpest loud onset in the first `window` seconds, or None.
/// A clap is defined by how abruptly it arrives, and judged RELATIVE to the
/// track, because the same clap reads 525 at one gain and thousands at another.
pub fn loudest_onset(path: &Path, window: f64) -> Option<f64> {
    let o = run_timeout(Command::new("ffmpeg").args(["-v", "error", "-t", &window.to_string(), "-i"])
        .arg(path).args(["-map", "0:a:0", "-ac", "1", "-ar", "8000", "-f", "s16le", "-"]),
        Duration::from_secs(300)).ok()?;
    let d = o.stdout;
    if d.len() < 4000 {
        return None;
    }
    let (rate, block) = (8000.0, 160usize);
    let mut energies = Vec::new();
    let mut i = 0;
    // Python's range(0, len(d) - 2 * block, 2 * block): whole 20 ms blocks only.
    while i < d.len().saturating_sub(block * 2) {
        let mut peak = 0i32;
        let mut j = i;
        while j < i + block * 2 {
            let v = i16::from_le_bytes([d[j], d[j + 1]]) as i32;
            peak = peak.max(v.abs());
            j += 8;
        }
        energies.push(peak);
        i += block * 2;
    }
    if energies.len() < 3 {
        return None;
    }
    let rises: Vec<i32> = (1..energies.len()).map(|k| energies[k] - energies[k - 1]).collect();
    let best_rise = *rises.iter().max()?;
    let best = rises.iter().position(|&r| r == best_rise)? + 1;
    let peak = *energies.iter().max()?;
    let mut ordered = rises.clone();
    ordered.sort_unstable();
    let typical = ordered[ordered.len() / 2];
    if peak < 200 || (best_rise as f64) < peak as f64 * 0.20 || best_rise < (typical * 4).max(40) {
        return None;
    }
    Some(round(best as f64 * block as f64 / rate, 3))
}
