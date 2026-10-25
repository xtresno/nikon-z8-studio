//! Handing the camera between tethered stills and the OBS/Zoom feed. The
//! keeper (z8-feed) owns /dev/video10 for the whole session; this only says
//! whether the camera should be LIVE on it.
use std::path::PathBuf;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::app::{app, now, num, read_json, write_json_atomic};
use crate::camera::{cam, camera_present};
use crate::store;

static WEBCAM: Mutex<(f64, bool)> = Mutex::new((0.0, false));
// A look update must not turn a just-stopped feed back on during a handover.
static MODE_CHANGE: Mutex<()> = Mutex::new(());

/// Is mtplvcap running? /proc scan, cached two seconds; not knowing must be
/// survivable, so any error reads as "no".
pub fn webcam_running() -> bool {
    let t = now();
    {
        let w = WEBCAM.lock();
        if t - w.0 < 2.0 {
            return w.1;
        }
    }
    let found = std::fs::read_dir(&app().proc_root).map(|it| it.filter_map(|e| e.ok()).any(|e| {
        let n = e.file_name();
        n.to_string_lossy().chars().all(|c| c.is_ascii_digit())
            && std::fs::read_to_string(e.path().join("comm")).map(|c| c.trim() == "mtplvcap").unwrap_or(false)
    })).unwrap_or(false);
    *WEBCAM.lock() = (t, found);
    found
}

pub fn webcam_forget() {
    WEBCAM.lock().0 = 0.0;
}

fn mode_marker() -> PathBuf {
    app().home.join(".last-mode.json")
}

/// What the user last asked for, so a restart can put it back.
pub fn remember_mode(mode: &str, look: Option<&Value>) {
    let _ = crate::app::write_json_atomic(&mode_marker(),
        &json!({"mode": mode, "look": look.cloned().unwrap_or(Value::Null), "at": now()}), 0);
}

pub fn marker_look() -> Option<Value> {
    read_json(&mode_marker()).and_then(|m| m.get("look").cloned()).filter(|v| !v.is_null())
}

pub fn feed_ctrl() -> PathBuf {
    if let Some(p) = std::env::var_os("Z8_FEED_CTRL") {
        return PathBuf::from(p);
    }
    let a = app();
    if a.port == 8420 { a.home.join(".feed.json") } else { a.home.join(format!(".feed-{}.json", a.port)) }
}

pub fn feed_status_path() -> PathBuf {
    std::env::var_os("Z8_FEED_STATUS").map(PathBuf::from)
        .unwrap_or_else(|| app().home.join(".feed-status.json"))
}

/// The keeper decides only from this file; `look` is the call look's numbers
/// (it grades frames itself), `vf` its ffmpeg fallback.
pub fn feed_set(live: bool, vf: Option<String>, look: Option<&Value>) {
    let look = look.filter(|l| l.is_object()).cloned().unwrap_or(Value::Null);
    let _ = write_json_atomic(&feed_ctrl(), &json!({"live": live, "vf": vf, "look": look, "at": now()}), 0);
}

pub fn feed_wanted() -> bool {
    read_json(&feed_ctrl()).map(|v| crate::app::truthy(v.get("live"))).unwrap_or(false)
}

/// The keeper's own report, or None when it is not running (stale over 5 s).
pub fn feed_status() -> Option<Value> {
    let st = read_json(&feed_status_path())?;
    if now() - num(st.get("at"), 0.0) > 5.0 {
        return None;
    }
    Some(st)
}

/// Restart within five minutes of a webcam session and the feed comes back.
/// Stands down while the keeper runs: it owns the wanted flag.
pub fn restore_mode_if_interrupted() {
    let Some(m) = read_json(&mode_marker()) else { return };
    if m.get("mode").and_then(|v| v.as_str()) != Some("webcam") {
        return;
    }
    if feed_status().is_some() {
        return;
    }
    let age = now() - num(m.get("at"), 0.0);
    if age > 300.0 {
        eprintln!("[z8] last mode was webcam but {age:.0}s ago, not restoring");
        return;
    }
    if webcam_running() {
        return;
    }
    eprintln!("[z8] restarted {age:.0}s into a webcam session, restoring the feed");
    let look = m.get("look").cloned().filter(|v| !v.is_null());
    std::thread::spawn(move || {
        set_mode("webcam", look.as_ref());
    });
}

/// Take the camera off the feed; return once it has really gone. SIGINT
/// only: anything harsher wedges the camera until a USB reset.
pub fn stop_feed(timeout: f64) -> bool {
    if app().port == 8420 {
        feed_set(false, None, None);
    }
    let soft = now() + if feed_status().is_some() { 6.0 } else { 0.0 };
    while now() < soft {
        webcam_forget();
        if !webcam_running() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = crate::tools::run_timeout(std::process::Command::new("pkill").args(["-INT", "-x", "mtplvcap"]),
                                      Duration::from_secs(5));
    let deadline = now() + timeout;
    while now() < deadline {
        webcam_forget();
        if !webcam_running() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    webcam_forget();
    !webcam_running()
}

pub fn set_mode(mode: &str, look: Option<&Value>) -> Value {
    let _change = MODE_CHANGE.lock();
    let a = app();
    if mode == "webcam" {
        if !webcam_running() && !camera_present() {
            remember_mode("off", None);
            return json!({"ok": false, "mode": "off", "webcam": false, "connected": cam().is_open(),
                          "error": "could not start the webcam feed: no camera on USB. Switch the Z8 \
                                    on (photo or video mode)."});
        }
        cam().disconnect();
        std::thread::sleep(Duration::from_secs(1));
        if feed_status().is_some() && a.port == 8420 {
            feed_set(true, look.map(ffmpeg_look), look);
            for _ in 0..50 {
                std::thread::sleep(Duration::from_millis(500));
                webcam_forget();
                if webcam_running() {
                    break;
                }
            }
        } else if !webcam_running() {
            let mut c = std::process::Command::new(crate::app::home_dir().join("bin/z8-webcam.sh"));
            if let Some(l) = look {
                c.env("Z8_VF", ffmpeg_look(l));
            }
            use std::os::unix::process::CommandExt;
            // A new SESSION, as start_new_session did: the feed outlives this
            // server and must not share its controlling terminal.
            unsafe {
                c.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
            let _ = c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
            for _ in 0..20 {
                std::thread::sleep(Duration::from_secs(1));
                webcam_forget();
                if webcam_running() {
                    break;
                }
            }
        }
        webcam_forget();
        if !webcam_running() {
            // The user asked to USE the camera: re-arm auto-connect.
            cam().a.lock().manual_off = false;
            remember_mode("off", None);
            if a.port == 8420 {
                feed_set(false, None, None);
            }
            let why = if !camera_present() { ": no camera on USB" } else { ", the camera may be busy or asleep" };
            return json!({"ok": false, "mode": "off", "webcam": false, "connected": false,
                          "error": format!("could not start the webcam feed{why}")});
        }
        remember_mode("webcam", look);
        let running = webcam_running();
        return json!({"ok": running, "mode": "webcam", "webcam": running, "connected": false,
                      "filter": look.map(ffmpeg_look)});
    }
    if mode == "tether" {
        remember_mode("tether", None);
        if a.port == 8420 {
            feed_set(false, None, None);
        }
        if webcam_running() {
            // A feed started elsewhere left a dead handle here: drop it.
            cam().disconnect();
            cam().a.lock().manual_off = false;
            stop_feed(11.0);
            std::thread::sleep(Duration::from_millis(1500));
        }
        let ok = cam().connect();
        // Tether is asking to USE the camera: a previous Off no longer
        // holds, or a camera that sleeps and wakes stays closed for good.
        cam().a.lock().manual_off = false;
        return json!({"ok": ok, "mode": "tether", "webcam": webcam_running(), "connected": ok,
                      "error": cam().a.lock().error.clone()});
    }
    cam().disconnect();
    if a.port == 8420 {
        feed_set(false, None, None);
    }
    let stopped = if webcam_running() { stop_feed(11.0) } else { true };
    remember_mode("off", None);
    json!({"ok": stopped, "mode": "off", "webcam": !stopped, "connected": false,
           "error": if stopped { Value::Null } else { json!("the feed did not stop within 11 s; press Off again") }})
}

/// The look the Zoom/OBS feed should use: explicit wins, else the configured one.
pub fn webcam_look(explicit: Option<&Value>) -> Option<Value> {
    if let Some(e) = explicit {
        return Some(e.clone());
    }
    let want = store::cfg_str("webcam_look_id");
    if want.is_empty() || want == "sliders" {
        return None;
    }
    let looks = store::presets().ok()?.get("look")?.as_array()?.clone();
    looks.into_iter().find(|l| l.get("id").and_then(|v| v.as_str()) == Some(&want)).map(|l| {
        let mut m = l.as_object().cloned().unwrap_or_default();
        for k in ["id", "label", "note"] {
            m.shift_remove(k);
        }
        Value::Object(m)
    })
}

/// Change the running grader in place. No camera commands, USB handover, or
/// writer restart: the keeper picks up the control file on its next pass.
pub fn change_call_look(id: &str, sliders: Option<&Value>, follow: bool) -> Result<(bool, String), String> {
    let _change = MODE_CHANGE.lock();
    if follow {
        if id != "sliders" { return Err("only sliders can follow edits".into()); }
        let current = store::cfg_str("webcam_look_id");
        // Another phone/page may have chosen a named look since this page's
        // last poll. A background slider update is not a new selection.
        if current != "sliders" { return Ok((feed_wanted(), current)); }
    }
    let look = if id.is_empty() {
        None
    } else if id == "sliders" {
        Some(sliders.filter(|v| v.is_object()).ok_or("the page must send the edit sliders")?.clone())
    } else {
        let presets = store::presets()?;
        let mut look = presets.get("look").and_then(Value::as_array)
            .and_then(|ls| ls.iter().find(|l| l.get("id").and_then(Value::as_str) == Some(id)))
            .and_then(Value::as_object).cloned().ok_or("unknown call look")?;
        for k in ["id", "label", "note"] { look.shift_remove(k); }
        Some(Value::Object(look))
    };
    let live = feed_wanted();
    if live && feed_status().is_none() {
        return Err("the webcam keeper is not responding; try Webcam again".into());
    }
    if !live && webcam_running() {
        return Err("this webcam was started outside the keeper; switch to Tether, then Webcam".into());
    }
    if !follow {
        store::save_config(&json!({"webcam_look_id": id}).as_object().unwrap().clone())?;
    }
    if live {
        write_json_atomic(&feed_ctrl(), &json!({"live": true, "vf": look.as_ref().map(ffmpeg_look),
            "look": look, "at": now()}), 0).map_err(|e| format!("could not update the feed: {e}"))?;
        // A restart must restore the newly selected look, including no look.
        write_json_atomic(&mode_marker(), &json!({"mode": "webcam", "look": look, "at": now()}), 0)
            .map_err(|e| format!("look updated, but could not save restart state: {e}"))?;
    }
    Ok((live, id.into()))
}

/// The ffmpeg filter chain for a look (the keeper's fallback writer).
pub fn ffmpeg_look(adj: &Value) -> String {
    let f = |k: &str, d: f64| num(adj.get(k), d);
    let (exposure, contrast, sat) = (f("exposure", 1.0), f("contrast", 1.0), f("saturation", 1.0));
    let (warmth, vig, sharp) = (f("warmth", 0.0), f("vignette", 0.0), f("sharpness", 1.0));
    let mut parts = Vec::new();
    let mut eq = Vec::new();
    if exposure != 1.0 {
        eq.push(format!("brightness={:.3}", ((exposure - 1.0) * 0.45).clamp(-1.0, 1.0)));
    }
    if contrast != 1.0 {
        eq.push(format!("contrast={:.3}", contrast.clamp(0.0, 3.0)));
    }
    if sat != 1.0 {
        eq.push(format!("saturation={:.3}", sat.clamp(0.0, 3.0)));
    }
    if !eq.is_empty() {
        parts.push(format!("eq={}", eq.join(":")));
    }
    if warmth != 0.0 {
        let w = warmth.clamp(-1.0, 1.0);
        parts.push(format!("colorbalance=rm={:.3}:bm={:.3}", w * 0.25, -w * 0.25));
    }
    if vig != 0.0 {
        parts.push(format!("vignette=a={:.3}", (vig * 1.1).min(1.2)));
    }
    if sharp > 1.0 {
        parts.push(format!("unsharp=5:5:{:.2}", ((sharp - 1.0) * 1.2).min(2.0)));
    }
    parts.push("scale=in_range=full:out_range=limited".into());
    parts.push("format=yuv420p".into());
    parts.join(",")
}

/// Plugging the camera in should be the whole ritual.
pub fn maybe_autoconnect(present: bool) {
    let c = cam();
    {
        let a = c.a.lock();
        if !(present && !c.is_open() && !a.manual_off && !app().no_autoconnect && now() - a.last_try > 6.0) {
            return;
        }
    }
    // Never while the feed has the camera, or is WANTED (mid-reconnect it
    // looks free for a few seconds).
    if webcam_running() || feed_wanted() {
        return;
    }
    {
        let mut a = c.a.lock();
        a.last_try = now();
        if a.connecting {
            return;
        }
        a.connecting = true;
    }
    std::thread::spawn(|| {
        cam().connect();
        cam().a.lock().connecting = false;
    });
}
