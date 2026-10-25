//! Everything the app shells out to, and a subprocess runner that can never
//! hang a request: every call has a ceiling, output is drained on threads so a
//! full pipe cannot block the child, and a child past its ceiling is killed.
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::load::which;

pub const TOOLS: &[(&str, &str, &str)] = &[
    ("ffmpeg", "recording, clip trimming and the Zoom feed", "ffmpeg"),
    ("ffprobe", "reading clip length and size", "ffmpeg"),
    ("heif-convert", "opening the HEIF files the Z8 actually emits", "libheif-examples"),
    ("darktable-cli", "developing raw NEF frames", "darktable"),
    ("xdg-open", "the Open shots / Open exports buttons", "xdg-utils"),
];

pub fn missing_tools() -> Value {
    Value::Array(TOOLS.iter().filter(|(b, _, _)| which(b).is_none())
        .map(|(b, why, pkg)| json!({"bin": b, "why": why, "install": pkg})).collect())
}

/// Raise something a human can act on, before shelling out.
pub fn need(bin: &str) -> Result<(), String> {
    if which(bin).is_some() {
        return Ok(());
    }
    let (why, pkg) = TOOLS.iter().find(|(b, _, _)| *b == bin)
        .map(|(_, w, p)| (*w, *p)).unwrap_or(("", ""));
    Err(format!("{bin} is not installed, so {why} cannot work. Install it with: sudo apt install {pkg}"))
}

/// Run to completion or kill at `limit`. Err only when it could not start or
/// was killed; a non-zero exit is still Ok with its status.
pub fn run_timeout(cmd: &mut Command, limit: Duration) -> Result<Output, String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| e.to_string())?;
    let mut so = child.stdout.take();
    let mut se = child.stderr.take();
    let t1 = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = so.as_mut() {
            let _ = s.read_to_end(&mut v);
        }
        v
    });
    let t2 = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = se.as_mut() {
            let _ = s.read_to_end(&mut v);
        }
        v
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = t1.join();
                let _ = t2.join();
                return Err("timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(e) => return Err(e.to_string()),
        }
    };
    Ok(Output { status, stdout: t1.join().unwrap_or_default(), stderr: t2.join().unwrap_or_default() })
}

/// stdout as text, "" on any failure: the Python app's _run.
pub fn run_text(args: &[&str], secs: u64) -> String {
    if args.is_empty() {
        return String::new();
    }
    let mut c = Command::new(args[0]);
    c.args(&args[1..]);
    run_timeout(&mut c, Duration::from_secs(secs))
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

pub fn last_line(b: &[u8]) -> Option<String> {
    String::from_utf8_lossy(b).trim().lines().last().map(|s| s.to_string())
}
