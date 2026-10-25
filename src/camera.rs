//! The camera. ONE lock for every libgphoto2 call (it is not safe to call
//! concurrently, and a preview mid-capture wedges the MTP session), with a
//! ceiling on every wait, because libgphoto2 has no timeout of its own.
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::{Mutex, ReentrantMutex};
use serde_json::{json, Map, Value};

use crate::app::{app, now};
use crate::gp::{self, Session};
use crate::load::{nef_is_high_efficiency, normalise};
use crate::store;

pub struct Attrs {
    pub error: Option<String>,
    pub rotation: i64,
    pub mirror: bool,
    /// Set by a deliberate Off, so auto-connect does not undo it.
    pub manual_off: bool,
    pub last_try: f64,
    pub connecting: bool,
    /// Recording lives on the CAMERA, not in the page.
    pub recording: bool,
    pub warning: Option<String>,
    pub focus_note: Option<String>,
    /// The USB device the open session belongs to (sysfs dir + devnum). A
    /// camera that sleeps, switches off or re-enumerates (the photo/video
    /// switch does that) leaves the session dead while it still looks open.
    pub device: Option<String>,
    /// Live view frames refused in a row, and when live view was last restarted for it.
    pub preview_fails: u32,
    pub relatched_at: f64,
}

pub struct Camera {
    lock: ReentrantMutex<RefCell<Option<Session>>>,
    open: AtomicBool,
    pub a: Mutex<Attrs>,
}

// The session is only touched under `lock`.
unsafe impl Sync for Camera {}

static CAM: std::sync::OnceLock<Camera> = std::sync::OnceLock::new();

pub fn cam() -> &'static Camera {
    CAM.get_or_init(|| Camera {
        lock: ReentrantMutex::new(RefCell::new(None)),
        open: AtomicBool::new(false),
        a: Mutex::new(Attrs {
            error: None, rotation: 0, mirror: false, manual_off: false, last_try: 0.0,
            connecting: false, recording: false, warning: None, focus_note: None, device: None,
            preview_fails: 0, relatched_at: 0.0,
        }),
    })
}

pub struct Busy(pub String);

pub type CardListing = (Option<Vec<Value>>, Option<String>, bool);

pub const SETTING_MAP: &[(&str, &str)] = &[
    ("iso", "iso"), ("shutter", "shutterspeed"), ("aperture", "f-number"),
    ("whitebalance", "whitebalance"), ("focusmode", "focusmode"),
    ("focallength", "focallength"), ("quality", "imagequality"),
    ("exposurecomp", "exposurecompensation"), ("expprogram", "expprogram"),
];

/// PTP_DPC_NIKON_LiveViewSelector: the photo/video lever.
const LEVER: u16 = 0xD1A6;

/// Restart live view and read the lever as the camera now obeys it (0
/// photo, 1 video). The next preview frame starts live view again.
fn relatch(s: &gp::Session) -> Option<u8> {
    s.end_live_view().ok()?;
    std::thread::sleep(Duration::from_millis(300));
    s.prop_u8(LEVER).ok()
}

/// Restart live view by itself? A flip of the PHOTO/VIDEO lever during live view makes the Z8 refuse every
/// frame with "[-1] Unspecified error" until live view restarts (measured 2026-09-28: the lamp went blank
/// for 9 minutes in VIDEO), which only a tap on the lamp did. Three refusals in a row, the camera's generic
/// error only, never while recording, at most once every 15 s.
pub fn should_relatch(fails: u32, error: &str, recording: bool, since_last: f64) -> bool {
    fails >= 3 && error.contains("[-1]") && !recording && since_last >= 15.0
}

/// A setting as a number: "f/5.6", "1/50", "0.0200s", "+0.7", "1250".
fn setting_number(v: &str) -> Option<f64> {
    let t = v.trim().trim_start_matches("f/").trim_end_matches('s').trim_start_matches('+');
    match t.split_once('/') {
        Some((n, d)) => Some(n.trim().parse::<f64>().ok()? / d.trim().parse::<f64>().ok()?),
        None => t.parse().ok(),
    }
}

/// Did the camera take the value asked for? The camera writes values in its
/// own form: exposure compensation in thirds ("0.666" for +0.7, which read
/// as "declined" on 2026-09-27 although it had been set), a shutter speed as
/// seconds. Same text, or the same number within 6 %, or for exposure
/// compensation within 0.05 EV (a third of a stop is 0.333).
pub fn setting_matches(key: &str, asked: &str, got: &str) -> bool {
    if asked.trim() == got.trim() {
        return true;
    }
    match (setting_number(asked), setting_number(got)) {
        (Some(a), Some(g)) if key == "exposurecomp" => (a - g).abs() <= 0.05,
        (Some(a), Some(g)) => (a - g).abs() <= 0.06 * a.abs().max(g.abs()),
        _ => false,
    }
}

fn gp_name(key: &str) -> Option<&'static str> {
    SETTING_MAP.iter().find(|(k, _)| *k == key).map(|(_, n)| *n)
}

impl Camera {
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }
    /// Test hook: hold the camera lock while `f` runs, as a stalled USB call
    /// would. Only the tests use it.
    #[doc(hidden)]
    pub fn hold_lock_for_test<T>(&self, f: impl FnOnce() -> T) -> T {
        let _g = self.lock.lock();
        f()
    }

    pub fn mirror(&self) -> bool {
        self.a.lock().mirror
    }

    /// Take the lock with a ceiling, scaled for the test suite.
    fn held<T>(&self, secs: f64, what: &str, f: impl FnOnce(&RefCell<Option<Session>>) -> T)
        -> Result<T, Busy> {
        let secs = secs * app().lock_scale;
        match self.lock.try_lock_for(Duration::from_secs_f64(secs)) {
            Some(g) => Ok(f(&g)),
            None => Err(Busy(format!("the camera is busy, {what} timed out after {secs}s"))),
        }
    }

    fn release_gvfs(&self) {
        for name in ["gvfsd-gphoto2", "gvfs-gphoto2-vo"] {
            let _ = crate::tools::run_timeout(std::process::Command::new("pkill").args(["-x", name]),
                                              Duration::from_secs(5));
        }
    }

    /// Recover a wedged MTP session without unplugging anything.
    pub fn usb_reset(&self) -> bool {
        let out = crate::tools::run_text(&["lsusb"], 5);
        let re = regex::Regex::new(r"Bus (\d+) Device (\d+): ID 04b0:").expect("re");
        let Some(m) = re.captures(&out) else { return false };
        let path = format!("/dev/bus/usb/{}/{}", &m[1], &m[2]);
        let Ok(c) = std::ffi::CString::new(path) else { return false };
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_WRONLY) };
        if fd < 0 {
            return false;
        }
        const USBDEVFS_RESET: libc::c_ulong = ((b'U' as libc::c_ulong) << 8) | 20;
        let r = unsafe { libc::ioctl(fd, USBDEVFS_RESET as _, 0) };
        unsafe { libc::close(fd) };
        if r < 0 {
            return false;
        }
        std::thread::sleep(Duration::from_secs(4));
        self.release_gvfs();
        std::thread::sleep(Duration::from_secs(1));
        true
    }

    pub fn connect(&self) -> bool {
        let g = self.lock.lock();
        if g.borrow().is_some() {
            return true;
        }
        for attempt in 1..=2 {
            self.release_gvfs();
            std::thread::sleep(Duration::from_millis(600));
            match Session::open() {
                Ok(s) => {
                    *g.borrow_mut() = Some(s);
                    self.open.store(true, Ordering::SeqCst);
                    let mut a = self.a.lock();
                    a.error = None;
                    a.device = nikon_device(&app().usb_root);
                    return true;
                }
                Err(e) => {
                    let wedged = e.contains("-10") || e.contains("-53");
                    self.a.lock().error = Some(e);
                    // -10 timeout, -53 could not claim: a stale session or
                    // another holder. One reset, then give up.
                    if attempt == 1 && wedged {
                        self.usb_reset();
                        continue;
                    }
                    return false;
                }
            }
        }
        false
    }

    /// Drop a session whose camera has left the bus or come back as a new
    /// device, so the page stops saying "connected" and auto-connect opens
    /// the camera again when it returns. Not a deliberate Off: manual_off is
    /// left alone. Returns false when it dropped the session.
    pub fn check_still_attached(&self) -> bool {
        let opened = self.a.lock().device.clone();
        if !self.is_open() || opened.is_none() {
            return true;
        }
        let now_dev = nikon_device(&app().usb_root);
        if now_dev == opened {
            return true;
        }
        let Some(g) = self.lock.try_lock_for(Duration::from_secs(10)) else { return true };
        let s = g.borrow_mut().take();
        self.open.store(false, Ordering::SeqCst);
        drop(s);
        let mut a = self.a.lock();
        a.device = None;
        a.recording = false;
        a.error = Some(if now_dev.is_none() {
            "the camera left USB (asleep or switched off); it reconnects when it is back".into()
        } else {
            "the camera reconnected on USB (the photo/video switch does that); opening it again".into()
        });
        eprintln!("[z8] {}", a.error.as_deref().unwrap_or(""));
        false
    }

    pub fn disconnect(&self) {
        {
            let mut a = self.a.lock();
            a.manual_off = true;
            a.recording = false;
        }
        let g = self.lock.lock();
        let s = g.borrow_mut().take();
        self.open.store(false, Ordering::SeqCst);
        drop(s);
    }

    /// A live view frame, or None when busy or not connected: the next poll
    /// simply asks again.
    pub fn preview_jpeg(&self) -> Option<Vec<u8>> {
        let g = self.lock.try_lock_for(Duration::from_secs(3))?;
        let b = g.borrow();
        let s = b.as_ref()?;
        match s.preview() {
            Ok(d) => {
                // A transient preview error (for example while the battery is
                // changed) is no longer current once frames are arriving again.
                let mut a = self.a.lock();
                a.error = None;
                a.preview_fails = 0;
                Some(d)
            }
            Err(e) => {
                let go = {
                    let mut a = self.a.lock();
                    a.preview_fails += 1;
                    let go = should_relatch(a.preview_fails, &e, a.recording, now() - a.relatched_at);
                    if go {
                        a.preview_fails = 0;
                        a.relatched_at = now();
                    }
                    a.error = Some(e);
                    go
                };
                if go {
                    // the same restart a tap on the lamp does; the next frame starts live view again
                    let side = relatch(s);
                    eprintln!("[z8] live view refused frames: restarted it; the lever reads {side:?}");
                }
                None
            }
        }
    }

    pub fn autofocus(&self) -> Result<(bool, String), Busy> {
        self.held(12.0, "autofocus", |c| {
            let b = c.borrow();
            let Some(s) = b.as_ref() else { return (false, "not connected".into()) };
            let r = (|| -> gp::GpResult<()> {
                let cfg = s.config()?;
                cfg.child("autofocusdrive")?.set_int(1)?;
                s.set_config(&cfg)
            })();
            match r {
                Ok(()) => (true, "ok".into()),
                Err(e) => (false, e),
            }
        })
    }

    /// Turn the focus ring by `steps` lens steps (libgphoto2 manualfocusdrive; the sign picks the way).
    /// Works with live view on, in AF-S/AF-C too. Shoot with {"focus": false} after it, or the
    /// autofocus before the shot undoes it.
    pub fn focus_drive(&self, steps: i32) -> Result<(bool, String), Busy> {
        self.action("focus drive", "manualfocusdrive", &format!("{steps}"))
    }

    /// Move the focus point to (x, y) in the camera's live view pixels (libgphoto2 changeafarea).
    pub fn af_area(&self, x: i32, y: i32) -> Result<(bool, String), Busy> {
        self.action("focus point", "changeafarea", &format!("{x}x{y}"))
    }

    fn action(&self, what: &'static str, node: &str, value: &str) -> Result<(bool, String), Busy> {
        self.held(12.0, what, |c| {
            let b = c.borrow();
            let Some(s) = b.as_ref() else { return (false, "not connected".into()) };
            let r = (|| -> gp::GpResult<()> {
                let cfg = s.config()?;
                cfg.child(node)?.set_text(value)?;
                s.set_config(&cfg)
            })();
            match r {
                Ok(()) => (true, "ok".into()),
                Err(e) => (false, e),
            }
        })
    }

    /// Fire the shutter and pull the file down. (ok, name or error).
    pub fn shoot(&self, dest: &Path, focus: Option<bool>) -> Result<(bool, String), Busy> {
        let raw_path = self.held(45.0, "the capture", |c| -> Result<PathBuf, String> {
            if c.borrow().is_none() {
                return Err("not connected".into());
            }
            let cfg = store::load_config();
            let want_af = focus.unwrap_or_else(|| cfg.get("autofocus_before_shoot")
                .map(|v| crate::app::truthy(Some(v))).unwrap_or(true));
            self.a.lock().focus_note = None;
            if want_af {
                // Reentrant: the camera cannot be taken away between focusing
                // and firing.
                match self.autofocus() {
                    Ok((true, _)) => {
                        let settle = crate::app::num(cfg.get("autofocus_settle_ms"), 600.0);
                        std::thread::sleep(Duration::from_secs_f64(settle.clamp(0.0, 3000.0) / 1000.0));
                    }
                    Ok((false, msg)) => {
                        self.a.lock().focus_note = Some(format!("autofocus did not confirm: {msg}"));
                    }
                    Err(Busy(msg)) => {
                        self.a.lock().focus_note = Some(format!("autofocus did not confirm: {msg}"));
                    }
                }
            }
            let b = c.borrow();
            let s = b.as_ref().ok_or("not connected")?;
            // Refused with the lever on VIDEO, as -7 or a bare -1. A lever
            // moved to PHOTO during live view is not obeyed until live view
            // restarts, so restart it and look: on PHOTO now, fire once more;
            // still on VIDEO, say so (focus mode is no guide, it read AF-F on
            // the photo side).
            let (folder, name) = match s.capture() {
                Ok(r) => r,
                Err(first) => match relatch(s) {
                    Some(0) => s.capture().map_err(|e| format!("{e} (after restarting live view)"))?,
                    Some(1) => return Err(format!("{first} | the camera is on VIDEO: move its lever to PHOTO for stills")),
                    _ => return Err(first),
                },
            };
            // Second-resolution names: a burst must not overwrite itself.
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
            let ext = store::splitext(&name).1.to_lowercase();
            let ext = if ext.is_empty() { ".jpg".to_string() } else { ext };
            let listing = store::list_dir(dest);
            let taken = |stem: &str| listing.iter().any(|f| f.starts_with(&format!("{stem}."))
                || *f == format!("{stem}{ext}"));
            let (mut base, mut n) = (stamp.clone(), 2);
            while taken(&base) {
                base = format!("{stamp}-{n}");
                n += 1;
            }
            let raw_path = dest.join(format!("{base}{ext}"));
            let data = s.file_get(&folder, &name)?;
            // Staged, then renamed: the strip polls this folder, and a frame
            // listed half-written fails to decode under its real name.
            store::write_atomic(&raw_path, &data).map_err(|e| e.to_string())?;
            Ok(raw_path)
        })?;
        let raw_path = match raw_path {
            Ok(p) => p,
            Err(e) => return Ok((false, e)),
        };
        // The camera reports a .nef name even when it emits HEIF.
        let real = normalise(&raw_path);
        let he = real.to_string_lossy().to_lowercase().ends_with(".nef") && nef_is_high_efficiency(&real);
        // Develop a raw in the background now, so opening it is instant; the
        // strip shows its embedded JPEG until then. HE raws cannot develop.
        if crate::load::is_raw(&real) && !he {
            let p = real.clone();
            std::thread::spawn(move || {
                if let Err(e) = crate::load::develop_raw(&p) {
                    eprintln!("[z8] background develop of {}: {e}", p.display());
                }
            });
        }
        self.a.lock().warning = he.then(|| "This raw is High Efficiency, which cannot be developed. \
            On the camera: PHOTO SHOOTING MENU > RAW recording > RAW compression > Lossless \
            compressed, then re-shoot.".to_string());
        Ok((true, store::fname(&real)))
    }

    /// Everything on the card, newest first, bounded by `limit`:
    /// (files, a folder that could not be read, truncated).
    pub fn card_list(&self, limit: usize) -> Result<CardListing, Busy> {
        self.held(30.0, "reading the card", |c| {
            let b = c.borrow();
            let Some(s) = b.as_ref() else { return (None, Some("not connected".into()), false) };
            // Every call is a USB round trip; say what a listing cost.
            let (t0, mut nf, mut nd) = (std::time::Instant::now(), 0, 0);
            let r = walk_card(&mut |d| { nd += 1; s.list_folders(d) },
                              &mut |d| { nf += 1; s.list_files(d) }, limit);
            eprintln!("[z8] card listing: {nd} folder + {nf} file calls, {} ms",
                      t0.elapsed().as_millis());
            r
        })
    }

    pub fn card_get(&self, folder: &str, name: &str, dest: &Path) -> Result<(bool, String), Busy> {
        let r = self.held(60.0, "importing from the card", |c| -> Result<PathBuf, String> {
            let b = c.borrow();
            let s = b.as_ref().ok_or("not connected")?;
            let data = s.file_get(folder, name)?;
            let target = dest.join(name);
            if target.exists() {
                return Ok(target);
            }
            store::write_atomic(&target, &data).map_err(|e| e.to_string())?;
            Ok(target)
        })?;
        Ok(match r {
            Ok(t) => {
                if store::ends_with_any(name, store::VIDEO_EXT) {
                    (true, store::fname(&t))
                } else {
                    (true, store::fname(&normalise(&t)))
                }
            }
            Err(e) => (false, e),
        })
    }

    pub fn movie(&self, on: bool) -> Result<(bool, String), Busy> {
        self.held(12.0, "the record command", |c| {
            let b = c.borrow();
            let Some(s) = b.as_ref() else { return (false, "not connected".into()) };
            let start = || -> gp::GpResult<()> {
                let cfg = s.config()?;
                cfg.child("movie")?.set_int(on as i32)?;
                s.set_config(&cfg)
            };
            // A lever moved to VIDEO during live view is obeyed only once live
            // view restarts (see relatch): restart and try once more.
            let r = match start() {
                Err(_) if on && relatch(s) == Some(1) => start(),
                r => r,
            };
            match r {
                Ok(()) => {
                    self.a.lock().recording = on;
                    (true, if on { "recording" } else { "stopped" }.into())
                }
                // Refused as -7 or, measured 2026-09-25, a bare -1. The lever
                // itself says which side it is on (focus mode does not: it
                // read AF-F on the photo side on 2026-09-27).
                Err(e) if on && (e.contains("-7") || e.contains("Access") || s.prop_u8(LEVER) == Ok(0)) =>
                    (false, format!("{e} | the camera has to be in VIDEO mode to record: \
                                     move the photo/video switch to video")),
                Err(e) => (false, e),
            }
        })
    }

    /// The mapped settings, informational: a busy camera just gives {}.
    pub fn settings(&self) -> Map<String, Value> {
        let Some(g) = self.lock.try_lock_for(Duration::from_secs(5)) else { return Map::new() };
        let b = g.borrow();
        let Some(s) = b.as_ref() else { return Map::new() };
        let Ok(cfg) = s.config() else { return Map::new() };
        let mut out = Map::new();
        for (key, name) in SETTING_MAP {
            if let Ok(node) = cfg.child(name) {
                let Ok(v) = node.value() else { continue };
                out.insert(key.to_string(), json!({"value": v.py_str(), "choices": node.choices(),
                                                   "readonly": node.readonly().unwrap_or(false)}));
            }
        }
        if let Ok(node) = cfg.child("batterylevel") {
            if let Ok(v) = node.value() {
                out.insert("battery".into(), json!({"value": v.py_str(), "choices": [], "readonly": true}));
            }
        }
        // The PHOTO/VIDEO lever: Nikon's LiveViewSelector, read-only over USB
        // (measured 2026-09-27, also in application mode), so it is shown,
        // never set. 0 photo, 1 video, measured with the lever in each.
        if let Ok(v) = s.prop_u8(LEVER) {
            let side = match v { 0 => "photo", 1 => "video", _ => "unknown" };
            out.insert("lever".into(), json!({"value": side, "choices": [], "readonly": true}));
        }
        out
    }

    /// The lever as the camera obeys it NOW: live view restarted first, as
    /// a flip made during live view is only picked up by a new one. For a
    /// tap on the lamp, never for polling (the live view blinks).
    pub fn recheck_lever(&self) -> Result<Option<&'static str>, Busy> {
        self.held(8.0, "checking the lever", |c| {
            let b = c.borrow();
            match b.as_ref().and_then(relatch) {
                Some(0) => Some("photo"),
                Some(1) => Some("video"),
                _ => None,
            }
        })
    }

    /// The PHOTO/VIDEO lever, None when it cannot be read.
    /// Never waits on a busy camera: a shot or a settings read wins.
    pub fn lever(&self) -> Option<&'static str> {
        let g = self.lock.try_lock_for(Duration::from_millis(200))?;
        let b = g.borrow();
        match b.as_ref()?.prop_u8(LEVER).ok()? {
            0 => Some("photo"),
            1 => Some("video"),
            _ => None,
        }
    }

    /// Every config node the camera exposes, flat. Read-only.
    pub fn all_settings(&self) -> Result<Value, Busy> {
        self.held(15.0, "reading every setting", |c| {
            let b = c.borrow();
            let Some(s) = b.as_ref() else { return json!({"ok": false, "error": "not connected"}) };
            let cfg = match s.config() {
                Ok(c) => c,
                Err(e) => return json!({"ok": false, "error": e}),
            };
            let mut out = Map::new();
            fn walk(w: gp::Widget<'_>, path: &str, out: &mut Map<String, Value>) {
                for ch in w.children() {
                    let nm = ch.name();
                    if !ch.children().is_empty() {
                        walk(ch, &format!("{path}/{nm}"), out);
                    } else {
                        let v = ch.value().map(|v| v.json()).unwrap_or(Value::Null);
                        let ro = ch.readonly().map(Value::from).unwrap_or(Value::Null);
                        out.insert(nm.clone(), json!({"value": v, "readonly": ro, "path": format!("{path}/{nm}")}));
                    }
                }
            }
            walk(cfg.root(), "", &mut out);
            json!({"ok": true, "count": out.len(), "nodes": out})
        })
    }

    /// Write, then READ IT BACK: libgphoto2 returns success for a write the
    /// camera then declines.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(bool, String), Busy> {
        let Some(name) = gp_name(key) else { return Ok((false, "unknown setting".into())) };
        self.held(12.0, "that setting", |c| {
            let b = c.borrow();
            let Some(s) = b.as_ref() else { return (false, "not connected".into()) };
            let r = (|| -> gp::GpResult<()> {
                let cfg = s.config()?;
                cfg.child(name)?.set_text(value)?;
                s.set_config(&cfg)
            })();
            if let Err(e) = r {
                return (false, e);
            }
            let back = s.config().and_then(|cfg| cfg.child(name).and_then(|n| n.value()));
            match back {
                Err(_) => (true, "ok (could not read back)".into()),
                Ok(v) if !setting_matches(key, value, &v.py_str()) => (false, format!(
                    "the camera declined it and stayed on {}. Some settings are locked by the mode \
                     dial or by a menu option and cannot be set over USB", v.py_str())),
                Ok(_) => (true, "ok".into()),
            }
        })
    }
}

// ------------------------------------------------------------------ presence
static PRESENT: Mutex<(f64, bool)> = Mutex::new((0.0, false));

/// Is a Nikon on the USB bus at all? sysfs, cached two seconds; lsusb only
/// where there is no sysfs.
/// Which Nikon is on the bus, as "<sysfs dir>#<devnum>": the devnum changes
/// every time the camera re-enumerates. None when there is none or the tree
/// cannot be read.
pub fn nikon_device(root: &std::path::Path) -> Option<String> {
    std::fs::read_dir(root).ok()?.filter_map(|e| e.ok()).find_map(|e| {
        let vendor = std::fs::read_to_string(e.path().join("idVendor")).ok()?;
        if !vendor.trim().eq_ignore_ascii_case("04b0") {
            return None;
        }
        let dev = std::fs::read_to_string(e.path().join("devnum")).unwrap_or_default();
        Some(format!("{}#{}", e.file_name().to_string_lossy(), dev.trim()))
    })
}

/// A Nikon (vendor 04b0) in a sysfs-shaped USB tree; None when the tree
/// cannot be read. Matches the VENDOR, not "any USB device".
pub fn usb_has_nikon(root: &std::path::Path) -> Option<bool> {
    let it = std::fs::read_dir(root).ok()?;
    Some(it.filter_map(|e| e.ok()).any(|e| {
        std::fs::read_to_string(e.path().join("idVendor"))
            .map(|v| v.trim().eq_ignore_ascii_case("04b0")).unwrap_or(false)
    }))
}

pub fn camera_present() -> bool {
    let t = now();
    {
        let p = PRESENT.lock();
        if t - p.0 < 2.0 {
            return p.1;
        }
    }
    let found = usb_has_nikon(&app().usb_root)
        .unwrap_or_else(|| crate::tools::run_text(&["lsusb"], 3).contains("ID 04b0:"));
    *PRESENT.lock() = (t, found);
    found
}

// ------------------------------------------------------------------ presets
/// The settable values only: a stored read-only value can never be restored.
pub fn capture_cam_preset(settings: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in settings {
        if k.starts_with('_') || k == "battery" || !v.is_object() {
            continue;
        }
        if v.get("readonly").and_then(|r| r.as_bool()).unwrap_or(false) {
            continue;
        }
        if let Some(val) = v.get("value").filter(|x| !x.is_null()) {
            out.insert(k.clone(), json!(store::py_str(val)));
        }
    }
    out
}

/// Write a preset back, mode first, one retry for refusals, and report every
/// setting individually.
pub fn apply_cam_preset(values: &Map<String, Value>) -> Value {
    let order = |k: &str| match k { "expprogram" => 0, "quality" => 1, _ => 5 };
    let mut items: Vec<(&String, &Value)> = values.iter().collect();
    items.sort_by_key(|(k, _)| order(k));
    let (mut done, mut refused) = (Vec::new(), Vec::new());
    for (k, want) in items {
        let want = store::py_str(want);
        let (ok, msg) = cam().set_setting(k, &want).unwrap_or_else(|Busy(m)| (false, m));
        if ok && k == "expprogram" {
            std::thread::sleep(Duration::from_millis(600));
        }
        if ok {
            done.push(json!({"key": k, "value": want}));
        } else {
            refused.push(json!({"key": k, "value": want, "error": msg}));
        }
    }
    if !refused.is_empty() {
        std::thread::sleep(Duration::from_millis(600));
        let mut still = Vec::new();
        for item in refused {
            let (k, v) = (item["key"].as_str().unwrap_or(""), item["value"].as_str().unwrap_or(""));
            match cam().set_setting(k, v) {
                Ok((true, _)) => done.push(json!({"key": k, "value": v, "retried": true})),
                Ok((false, msg)) | Err(Busy(msg)) => {
                    let mut it = item.clone();
                    it["error"] = json!(msg);
                    still.push(it);
                }
            }
        }
        refused = still;
    }
    json!({"ok": refused.is_empty(), "applied": done, "refused": refused})
}

/// The card walk, apart from the camera so a fake card can drive it. Folders
/// first (bounded, cycle-proof), then files newest folder first, stopping at
/// `limit` so the limit bounds the USB round trips, not just the result.
pub fn walk_card(list_folders: &mut dyn FnMut(&str) -> Result<Vec<String>, String>,
                 list_files: &mut dyn FnMut(&str) -> Result<Vec<String>, String>,
                 limit: usize) -> CardListing {
    const MAX_FOLDERS: usize = 600;
    let (mut out, mut truncated, mut err) = (Vec::new(), false, None);
    let mut seen = std::collections::HashSet::from(["/".to_string()]);
    let (mut all_dirs, mut stack, mut folders) = (Vec::new(), vec!["/".to_string()], 0);
    while let Some(cur) = stack.pop() {
        folders += 1;
        if folders > MAX_FOLDERS {
            truncated = true;
            break;
        }
        all_dirs.push(cur.clone());
        match list_folders(&cur) {
            Ok(names) => {
                for n in names {
                    let child = if cur.ends_with('/') { format!("{cur}{n}") } else { format!("{cur}/{n}") };
                    if seen.insert(child.clone()) {
                        stack.push(child);
                    }
                }
            }
            // The root failing means there is no card to read. A subfolder
            // failing must not lose the rest of the card: note it and go on.
            Err(e) if cur == "/" => return (None, Some(e), false),
            Err(e) => {
                err = Some(e);
                truncated = true;
            }
        }
    }
    all_dirs.sort();
    all_dirs.reverse();
    for cur in all_dirs {
        let mut names = match list_files(&cur) {
            Ok(n) => n,
            Err(e) => {
                err = Some(e);
                truncated = true;
                continue;
            }
        };
        let room = limit.saturating_sub(out.len());
        names.sort();
        if names.len() > room {
            names = names[names.len() - room..].to_vec();
            truncated = true;
        }
        for n in names.iter().rev() {
            out.push(json!({"folder": cur, "name": n}));
        }
        if out.len() >= limit {
            return (Some(out), None, true);
        }
    }
    (Some(out), err, truncated)
}
