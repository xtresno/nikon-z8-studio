//! Process-wide state: where things live, and the handful of globals the
//! Python app kept at module level (the active session, transient config,
//! caches). One `App`, reached through `app()`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::{Mutex, RwLock};
use serde_json::{Map, Value};

pub struct App {
    /// Where presets.json, config.json and the state files live (the old HERE).
    pub home: PathBuf,
    /// Where the page's files are served from.
    pub web: PathBuf,
    /// Shots, one folder per day.
    pub root: PathBuf,
    pub cache_tmp: PathBuf,
    pub port: u16,
    /// Exact HTTPS origin served by a private reverse proxy; loopback stays available.
    pub remote_origin: Option<String>,
    pub no_autoconnect: bool,
    pub usb_root: PathBuf,
    pub proc_root: PathBuf,
    pub min_free_mb: f64,
    pub lock_scale: f64,
    pub audio_max_seconds: u64,
    pub started_at: f64,
    pub source_id: String,
    /// None means today. A capture always goes to today regardless.
    pub active_session: RwLock<Option<String>>,
    pub transient: Mutex<Map<String, Value>>,
    /// Serialises every read-modify-write of a sidecar or config file.
    pub sidecar: parking_lot::ReentrantMutex<()>,
    pub export_dir_warned: Mutex<Vec<String>>,
    pub meta_cache: Mutex<HashMap<(PathBuf, u64), Value>>,
}

static APP: std::sync::OnceLock<App> = std::sync::OnceLock::new();

pub fn app() -> &'static App {
    APP.get().expect("app not initialised")
}

fn env_path(key: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(key).map(PathBuf::from).unwrap_or(default)
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn expand_user(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        home_dir().join(rest)
    } else if s == "~" {
        home_dir()
    } else {
        PathBuf::from(s)
    }
}

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn mtime(p: &Path) -> Option<f64> {
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(m.duration_since(UNIX_EPOCH).ok()?.as_secs_f64())
}

pub fn init() -> &'static App {
    let home = env_path("Z8_HOME", home_dir().join("bin/z8-studio"));
    let web = env_path("Z8_WEB", home.join("web"));
    let port: u16 = std::env::var("Z8_PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8420);
    let a = App {
        source_id: source_fingerprint(&home, &web),
        home,
        web,
        root: env_path("Z8_ROOT", home_dir().join("Pictures/z8-studio")),
        cache_tmp: home_dir().join(".cache/z8-studio"),
        port,
        remote_origin: std::env::var("Z8_REMOTE_ORIGIN").ok().map(|v| {
            parse_remote_origin(&v).expect("Z8_REMOTE_ORIGIN must be an HTTPS origin, without a path")
        }),
        no_autoconnect: std::env::var("Z8_NO_AUTOCONNECT").as_deref() == Ok("1"),
        usb_root: env_path("Z8_USB_ROOT", PathBuf::from("/sys/bus/usb/devices")),
        proc_root: env_path("Z8_PROC_ROOT", PathBuf::from("/proc")),
        min_free_mb: std::env::var("Z8_MIN_FREE_MB").ok().and_then(|v| v.parse().ok())
            .unwrap_or(300.0),
        lock_scale: std::env::var("Z8_LOCK_SCALE").ok().and_then(|v| v.parse().ok())
            .filter(|v: &f64| *v > 0.0).unwrap_or(1.0),
        audio_max_seconds: std::env::var("Z8_AUDIO_MAX_SECONDS").ok()
            .and_then(|v| v.parse().ok()).unwrap_or(3600),
        started_at: now(),
        active_session: RwLock::new(None),
        transient: Mutex::new(Map::new()),
        sidecar: parking_lot::ReentrantMutex::new(()),
        export_dir_warned: Mutex::new(Vec::new()),
        meta_cache: Mutex::new(HashMap::new()),
    };
    let _ = APP.set(a);
    app()
}

/// A single explicit origin, never a wildcard or a value supplied by a request.
pub fn parse_remote_origin(value: &str) -> Result<String, &'static str> {
    let value = value.trim().trim_end_matches('/').to_ascii_lowercase();
    let uri: axum::http::Uri = value.parse().map_err(|_| "invalid origin")?;
    let host = uri.host().ok_or("missing host")?;
    let authority = uri.authority().ok_or("missing authority")?;
    if uri.scheme_str() != Some("https") || uri.query().is_some()
        || !matches!(uri.path(), "" | "/") || authority.as_str().contains('@')
        || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        || host.starts_with('.') || host.ends_with('.')
    {
        return Err("expected one HTTPS origin");
    }
    Ok(format!("https://{authority}"))
}

/// The fingerprint of what this PROCESS serves, so a test can refuse to grade
/// a stale server. The Python app hashed its five source files; here it is
/// the binary plus the page files and presets.
fn source_fingerprint(home: &Path, web: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    let mut files: Vec<PathBuf> = vec![
        web.join("index.html"), web.join("app.css"), web.join("dist/app.js"),
        home.join("presets.json"),
    ];
    if let Ok(exe) = std::env::current_exe() {
        files.push(exe);
    }
    for f in files {
        h.update(f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        match std::fs::read(&f) {
            Ok(b) => h.update(&b),
            Err(_) => h.update(b"\0"),
        }
    }
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

// ------------------------------------------------------------------ values
/// Python's float(x) with a fallback: numbers, numeric strings and bools.
pub fn num(v: Option<&Value>, default: f64) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(default),
        Some(Value::String(s)) => s.trim().parse::<f64>().unwrap_or(default),
        Some(Value::Bool(b)) => if *b { 1.0 } else { 0.0 },
        _ => default,
    }
}

/// Python truthiness of a JSON value.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

pub fn get_str<'a>(b: &'a Value, k: &str) -> &'a str {
    b.get(k).and_then(|v| v.as_str()).unwrap_or("")
}

/// Round like Python's round(x, n) closely enough for reporting.
pub fn round(x: f64, n: i32) -> f64 {
    let p = 10f64.powi(n);
    let v = x * p;
    // banker's rounding at exact halves, as Python does
    let r = v.round();
    let adj = if (v - v.trunc()).abs() == 0.5 && (r as i64) % 2 != 0 { r - v.signum() } else { r };
    adj / p
}

/// Atomic JSON write: temp file beside, then rename.
pub fn write_json_atomic(path: &Path, v: &Value, indent: usize) -> std::io::Result<()> {
    // Unique per writer: two saves of one file must not share a temp name,
    // or one renames the other's half-written temp into place.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_file_name(format!(
        "{}.{}-{n}.tmp", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        std::process::id()));
    let text = if indent == 0 { serde_json::to_string(v)? } else { py_json_indent(v, indent) };
    let result = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path));
    if result.is_err() { let _ = std::fs::remove_file(&tmp); }
    result
}

pub fn read_json(path: &Path) -> Option<Value> {
    let s = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&s).ok()
}

/// json.dump(..., indent=n) output, so files stay diffable against the old app.
pub fn py_json_indent(v: &Value, indent: usize) -> String {
    let mut buf = Vec::new();
    let pad = vec![b' '; indent];
    let fmt = serde_json::ser::PrettyFormatter::with_indent(&pad);
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    serde::Serialize::serialize(v, &mut ser).ok();
    String::from_utf8(buf).unwrap_or_default()
}

/// Hand freed arenas back to the OS after a big batch: glibc otherwise keeps
/// hundreds of MB resident after many large short-lived allocations.
pub fn release_memory() {
    unsafe {
        libc::malloc_trim(0);
    }
}
