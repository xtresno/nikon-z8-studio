#![allow(clippy::needless_range_loop, clippy::chunks_exact_to_as_chunks)]
//! z8-feed: keeps the "Nikon Z8" virtual webcam alive for as long as you are
//! logged in, so OBS, Zoom and anything else never lose it.
//!
//! /dev/video10 is a v4l2loopback device with exclusive_caps: it only offers
//! a CAMERA while something writes to it, so this owns the device for the
//! whole session and never stops writing:
//!
//!   - live camera frames while Z8 Studio wants the webcam (.feed.json live);
//!   - a "Reconnecting..." card over the last frame the moment frames stall,
//!     while it restarts the camera link itself (USB reset if wedged);
//!   - an "in use for photos" card otherwise.
//!
//! Always 1024x576. It only touches the camera while live is wanted, and
//! releases it with SIGINT, never harder.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use serde_json::{json, Value};

use z8::app::{home_dir, now, num, read_json, round, truthy, write_json_atomic};
use z8::frame::{Frame, Grader, Look, H, OUT_LEN, W};
use z8::img;

const STALL_S: f64 = 1.5;
const RESTART_S: f64 = 8.0;

fn log(msg: &str) {
    println!("[z8-feed] {msg}");
    let _ = std::io::stdout().flush();
}

struct Env {
    ctrl: PathBuf,
    status: PathBuf,
    dev: PathBuf,
    null_out: bool,
    /// Z8_FEED_OUTFMT=file: each frame overwrites a plain file, so a test can
    /// look at what the device would have shown.
    file_out: bool,
    mtp: PathBuf,
    port: u16,
    fps: u32,
    usb_root: PathBuf,
    threads: usize,
}

fn env() -> Env {
    let home = std::env::var_os("Z8_HOME").map(PathBuf::from).unwrap_or_else(|| home_dir().join("bin/z8-studio"));
    let e = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let vnr = e("VIDEO_NR").unwrap_or_else(|| "10".into());
    Env {
        ctrl: e("Z8_FEED_CTRL").map(PathBuf::from).unwrap_or_else(|| home.join(".feed.json")),
        status: e("Z8_FEED_STATUS").map(PathBuf::from).unwrap_or_else(|| home.join(".feed-status.json")),
        dev: e("Z8_FEED_DEV").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("/dev/video{vnr}"))),
        null_out: e("Z8_FEED_OUTFMT").as_deref() == Some("null"),
        file_out: e("Z8_FEED_OUTFMT").as_deref() == Some("file"),
        mtp: e("Z8_MTPLVCAP").map(|p| z8::app::expand_user(&p)).unwrap_or_else(|| home_dir().join("bin/mtplvcap")),
        port: e("PORT").and_then(|v| v.parse().ok()).unwrap_or(42839),
        fps: e("FPS").and_then(|v| v.parse().ok()).unwrap_or(30),
        usb_root: e("Z8_USB_ROOT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/sys/bus/usb/devices")),
        threads: e("Z8_FEED_THREADS").and_then(|v| v.parse().ok()).unwrap_or(4).clamp(1, 8),
    }
}

fn camera_present(e: &Env) -> bool {
    std::fs::read_dir(&e.usb_root).map(|it| it.filter_map(|x| x.ok()).any(|x| {
        std::fs::read_to_string(x.path().join("idVendor")).map(|v| v.trim().eq_ignore_ascii_case("04b0"))
            .unwrap_or(false)
    })).unwrap_or(false)
}

/// Running mtplvcap on OUR port, zombies excluded: a keeper under test must
/// never see, let alone SIGINT, the real one feeding a podcast.
fn mtplvcap_pids(port: u16) -> Vec<i32> {
    let want = format!("-port\0{port}\0");
    let mut out = vec![];
    let Ok(it) = std::fs::read_dir("/proc") else { return out };
    for ent in it.filter_map(|x| x.ok()) {
        let name = ent.file_name().to_string_lossy().into_owned();
        let Ok(pid) = name.parse::<i32>() else { continue };
        let p = ent.path();
        if std::fs::read_to_string(p.join("comm")).map(|c| c.trim() != "mtplvcap").unwrap_or(true) {
            continue;
        }
        let mut cmd = std::fs::read(p.join("cmdline")).unwrap_or_default();
        cmd.push(0);
        if !String::from_utf8_lossy(&cmd).contains(&want) {
            continue;
        }
        let st = std::fs::read_to_string(p.join("stat")).unwrap_or_default();
        let state = st.rsplit_once(')').and_then(|(_, r)| r.split_whitespace().next()).unwrap_or("Z");
        if state != "Z" && state != "X" {
            out.push(pid);
        }
    }
    out
}

struct Ctrl {
    live: bool,
    look: Option<Value>,
}

fn read_ctrl(e: &Env) -> Ctrl {
    match read_json(&e.ctrl) {
        Some(c) => Ctrl { live: truthy(c.get("live")), look: c.get("look").filter(|l| l.is_object()).cloned() },
        None => Ctrl { live: false, look: None },
    }
}

fn release_gvfs() {
    for pat in ["gvfsd-gphot[o]2", "gvfs-gphoto2-volume-monito[r]"] {
        let _ = Command::new("pkill").args(["-f", pat]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
}

fn reset_usb() -> bool {
    let out = Command::new("lsusb").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let re = regex::Regex::new(r"Bus (\d+) Device (\d+): ID 04b0:").expect("re");
    let Some(m) = re.captures(&out) else { return false };
    let Ok(c) = std::ffi::CString::new(format!("/dev/bus/usb/{}/{}", &m[1], &m[2])) else { return false };
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_WRONLY) };
    if fd < 0 {
        return false;
    }
    const USBDEVFS_RESET: libc::c_ulong = ((b'U' as libc::c_ulong) << 8) | 20;
    unsafe {
        libc::ioctl(fd, USBDEVFS_RESET as _, 0);
        libc::close(fd);
    }
    log("reset the camera's USB port");
    std::thread::sleep(Duration::from_secs(4));
    true
}

fn look_of(v: Option<&Value>) -> Look {
    let n = Look::neutral();
    let Some(v) = v else { return n };
    // Python's `float(x or default)`: a zero falls back to the default too.
    let g = |k: &str, d: f64| { let x = num(v.get(k), d); if x == 0.0 { d } else { x } };
    Look { exposure: g("exposure", 1.0), contrast: g("contrast", 1.0), saturation: g("saturation", 1.0),
           warmth: g("warmth", 0.0), vignette: g("vignette", 0.0), sharpness: g("sharpness", 1.0) }
}

// ------------------------------------------------------------------ writer
/// Writes I420 frames to the device itself: no ffmpeg in the path, which held
/// every frame 80-117 ms.
struct Writer {
    fd: Option<i32>,
    look: Option<Value>,
    grader: Grader,
    frame: Frame,
    rgb: Vec<u8>,
    pool: rayon::ThreadPool,
    restarts: u64,
    /// A card is the same picture written 5-10 times a second: converted once,
    /// and forgotten whenever the look changes.
    memo: Option<(usize, Vec<u8>)>,
}

impl Writer {
    fn new(threads: usize) -> Writer {
        Writer { fd: None, look: None, grader: Grader::new(&Look::neutral()), frame: Frame::default(),
                 rgb: Vec::new(), restarts: 0, memo: None,
                 pool: rayon::ThreadPoolBuilder::new().num_threads(threads)
                     .thread_name(|i| format!("z8f-{i}")).build().expect("pool") }
    }

    fn open(&mut self, e: &Env) -> Result<(), String> {
        if e.null_out {
            let c = std::ffi::CString::new("/dev/null").expect("path");
            self.fd = Some(unsafe { libc::open(c.as_ptr(), libc::O_WRONLY) });
            return Ok(());
        }
        let c = std::ffi::CString::new(e.dev.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
        if e.file_out {
            self.fd = Some(unsafe { libc::open(c.as_ptr(), libc::O_WRONLY | libc::O_CREAT, 0o644) });
            return Ok(());
        }
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_WRONLY) };
        if fd < 0 {
            return Err(format!("cannot open {}", e.dev.display()));
        }
        // struct v4l2_format for V4L2_BUF_TYPE_VIDEO_OUTPUT, YU12, 1024x576.
        let mut fmt = [0u8; 208];
        let put = |b: &mut [u8], off: usize, v: u32| b[off..off + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut fmt, 0, 2);
        let yu12 = u32::from_le_bytes(*b"YU12");
        for (i, v) in [W as u32, H as u32, yu12, 1, W as u32, OUT_LEN as u32].iter().enumerate() {
            put(&mut fmt, 8 + i * 4, *v);
        }
        const VIDIOC_S_FMT: libc::c_ulong = 0xC0D05605;
        if unsafe { libc::ioctl(fd, VIDIOC_S_FMT as _, fmt.as_mut_ptr()) } < 0 {
            unsafe { libc::close(fd) };
            return Err("could not set the device format".into());
        }
        self.fd = Some(fd);
        log("direct writer up (no ffmpeg in the path), frame engine: rust");
        Ok(())
    }

    fn ensure(&mut self, look: Option<&Value>) {
        if self.look.as_ref() != look {
            self.look = look.cloned();
            self.grader = Grader::new(&look_of(look));
            self.memo = None;
        }
    }

    fn convert_jpeg(&mut self, jpeg: &[u8], graded: bool) -> Result<Vec<u8>, String> {
        let im = img::jpeg_decode(jpeg, 1)?;
        self.rgb = im.px;
        let (g, f, rgb) = (&self.grader, &mut self.frame, &self.rgb);
        self.pool.install(|| f.process_par(g, rgb, im.w, im.h, graded));
        Ok(self.frame.out.clone())
    }

    fn convert_rgb(&mut self, rgb: &img::Rgb, graded: bool) -> Vec<u8> {
        let (g, f) = (&self.grader, &mut self.frame);
        self.pool.install(|| f.process_par(g, &rgb.px, rgb.w, rgb.h, graded));
        self.frame.out.clone()
    }

    fn write_raw(&mut self, raw: &[u8], e: &Env) -> bool {
        if self.fd.is_none() && self.open(e).is_err() {
            return false;
        }
        let fd = self.fd.unwrap_or(-1);
        if e.file_out {
            unsafe { libc::lseek(fd, 0, libc::SEEK_SET) };
        }
        let n = unsafe { libc::write(fd, raw.as_ptr() as *const libc::c_void, raw.len()) };
        if n != raw.len() as isize {
            // A short write would shift every later frame: a torn picture.
            unsafe { libc::close(fd) };
            self.fd = None;
            self.restarts += 1;
            return false;
        }
        true
    }

    fn write_jpeg(&mut self, jpeg: &[u8], e: &Env) -> bool {
        match self.convert_jpeg(jpeg, true) {
            Ok(raw) => self.write_raw(&raw, e),
            Err(_) => false,
        }
    }

    /// A card: converted once per (card, look), then the bytes are reused.
    fn write_card(&mut self, id: usize, card: &img::Rgb, e: &Env) -> bool {
        let raw = match &self.memo {
            Some((mid, r)) if *mid == id => r.clone(),
            _ => {
                let r = self.convert_rgb(card, true);
                self.memo = Some((id, r.clone()));
                r
            }
        };
        self.write_raw(&raw, e)
    }
}

// ------------------------------------------------------------------ cards
fn font() -> Option<img::Font> {
    img::Font::load(&["/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                      "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"])
}

/// The last live frame dimmed, or a dark background, with two lines of text.
fn card(title: &str, sub: &str, base: Option<&[u8]>, f: Option<&img::Font>) -> img::Rgb {
    let bg = base.and_then(|b| img::jpeg_decode(b, 1).ok()).map(|im| {
        let im = img::enhance_brightness(&im, 0.35);
        let s = (W as f64 / im.w as f64).max(H as f64 / im.h as f64);
        let im = img::resize(&im, (im.w as f64 * s + 0.5) as usize, (im.h as f64 * s + 0.5) as usize,
                             img::Filter::Bicubic);
        let (x, y) = ((im.w - W) / 2, (im.h - H) / 2);
        img::crop(&im, x as i64, y as i64, (x + W) as i64, (y + H) as i64)
    });
    let mut im = bg.unwrap_or_else(|| img::Rgb::new(W, H, [14, 16, 20]));
    if let Some(f) = f {
        let tw = f.text_width(title, 34.0);
        f.draw(&mut im, title, (W as f32 - tw) / 2.0, H as f32 / 2.0 - 36.0, 34.0, [236, 240, 245]);
        let sw = f.text_width(sub, 20.0);
        f.draw(&mut im, sub, (W as f32 - sw) / 2.0, H as f32 / 2.0 + 14.0, 20.0, [160, 170, 182]);
    }
    im
}

// ------------------------------------------------------------------ reader
/// The newest camera frame, when it arrived, and a sequence number.
type Newest = (Option<Arc<Vec<u8>>>, f64, u64);

struct Latest {
    frame: Mutex<Newest>,
    fresh: Condvar,
    enabled: AtomicBool,
    frames: AtomicU64,
}

/// Chunked transfer decoding: mtplvcap answers with it, and the chunk markers
/// sit in the middle of every frame otherwise.
struct Chunked<R: BufRead> {
    inner: R,
    left: usize,
    done: bool,
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done {
            return Ok(0);
        }
        if self.left == 0 {
            let mut line = String::new();
            if self.inner.read_line(&mut line)? == 0 {
                self.done = true;
                return Ok(0);
            }
            if line.trim().is_empty() {
                line.clear();
                if self.inner.read_line(&mut line)? == 0 {
                    self.done = true;
                    return Ok(0);
                }
            }
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(|_| std::io::Error::other("bad chunk"))?;
            if size == 0 {
                self.done = true;
                return Ok(0);
            }
            self.left = size;
        }
        let n = buf.len().min(self.left);
        let got = self.inner.read(&mut buf[..n])?;
        self.left -= got;
        if got == 0 {
            self.done = true;
        }
        Ok(got)
    }
}

fn stream_once(port: u16, latest: &Latest) -> std::io::Result<()> {
    let mut s = TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    s.write_all(b"GET /mjpeg HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: keep-alive\r\n\r\n")?;
    let mut r = BufReader::with_capacity(1 << 20, s);
    let mut status = String::new();
    r.read_line(&mut status)?;
    if !status.contains(" 200") {
        return Ok(());
    }
    let mut chunked = false;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 {
            return Ok(());
        }
        if h == "\r\n" || h == "\n" {
            break;
        }
        let l = h.to_lowercase();
        if l.starts_with("transfer-encoding:") && l.contains("chunked") {
            chunked = true;
        }
    }
    let body: Box<dyn Read> = if chunked { Box::new(Chunked { inner: r, left: 0, done: false }) } else { Box::new(r) };
    let mut body = BufReader::with_capacity(1 << 20, body);
    while latest.enabled.load(Ordering::SeqCst) {
        let mut line = Vec::new();
        if body.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if !line.starts_with(b"--") {
            continue;
        }
        let mut length = 0usize;
        loop {
            let mut h = Vec::new();
            if body.read_until(b'\n', &mut h)? == 0 {
                return Ok(());
            }
            if h == b"\r\n" || h == b"\n" {
                break;
            }
            let hs = String::from_utf8_lossy(&h).to_lowercase();
            if let Some(v) = hs.strip_prefix("content-length:") {
                length = v.trim().parse().unwrap_or(0);
            }
        }
        if length == 0 {
            continue;
        }
        let mut jpeg = vec![0u8; length];
        body.read_exact(&mut jpeg)?;
        let t = now();
        let mut g = latest.frame.lock();
        g.0 = Some(Arc::new(jpeg));
        g.1 = t;
        g.2 += 1;
        latest.frames.fetch_add(1, Ordering::SeqCst);
        latest.fresh.notify_all();
    }
    Ok(())
}

// ------------------------------------------------------------------ link
/// How often an idle keeper looks for a stray link on its port.
const IDLE_SCAN_S: f64 = 2.0;

struct Link {
    proc: Option<Child>,
    started_at: f64,
    fails: u32,
    restarts: u64,
    next_try: f64,
}

impl Link {
    fn alive(&mut self, port: u16) -> bool {
        if let Some(p) = self.proc.as_mut() {
            let _ = p.try_wait();
        }
        !mtplvcap_pids(port).is_empty()
    }
    fn exited(&mut self) -> bool {
        match self.proc.as_mut() {
            Some(p) => matches!(p.try_wait(), Ok(Some(_))),
            None => false,
        }
    }
    fn start(&mut self, e: &Env) {
        if now() < self.next_try {
            return;
        }
        release_gvfs();
        log("opening the camera");
        use std::os::unix::process::CommandExt;
        let mut cmd = Command::new(&e.mtp);
        cmd.args(["-max-resolution", "-fps", &e.fps.to_string(), "-port", &e.port.to_string()])
            .stdout(Stdio::null()).stderr(Stdio::null());
        // Its own session, as z8-feed.py's start_new_session gave it.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        match cmd.spawn() {
            Ok(c) => self.proc = Some(c),
            Err(err) => {
                log(&format!("cannot start {}: {err}", e.mtp.display()));
                self.next_try = now() + 5.0;
            }
        }
        self.started_at = now();
    }
    /// SIGINT, never harder: it closes the MTP session cleanly.
    fn stop(&mut self, port: u16, wait: f64) {
        for pid in mtplvcap_pids(port) {
            unsafe { libc::kill(pid, libc::SIGINT) };
        }
        let t = now() + wait;
        while now() < t && !mtplvcap_pids(port).is_empty() {
            std::thread::sleep(Duration::from_millis(200));
        }
        if let Some(mut p) = self.proc.take() {
            let _ = p.try_wait();
        }
    }
    fn failed_start(&mut self) {
        self.fails += 1;
        self.next_try = now() + (2.0 * self.fails as f64).min(20.0);
        if self.fails.is_multiple_of(2) {
            reset_usb();
        }
    }
}

// ------------------------------------------------------------------ main
fn main() {
    let e = Arc::new(env());
    if !e.dev.exists() && !e.null_out && !e.file_out {
        log(&format!("{} is missing; load v4l2loopback. Waiting for it.", e.dev.display()));
    }
    for sig in [libc::SIGTERM, libc::SIGINT] {
        unsafe { libc::signal(sig, on_signal as *const () as libc::sighandler_t) };
    }
    let writer = Arc::new(Mutex::new(Writer::new(e.threads)));
    let latest = Arc::new(Latest { frame: Mutex::new((None, 0.0, 0)), fresh: Condvar::new(),
                                   enabled: AtomicBool::new(false), frames: AtomicU64::new(0) });
    let sent = Arc::new(AtomicU64::new(0));
    let holds: Arc<Mutex<std::collections::VecDeque<f64>>> = Arc::new(Mutex::new(Default::default()));

    // Reader: reconnects forever while enabled.
    {
        let (latest, e) = (latest.clone(), e.clone());
        std::thread::spawn(move || loop {
            if latest.enabled.load(Ordering::SeqCst) {
                let _ = stream_once(e.port, &latest);
            }
            std::thread::sleep(Duration::from_millis(if latest.enabled.load(Ordering::SeqCst) { 300 } else { 50 }));
        });
    }
    // Pump: sends the NEWEST frame, dropping any it missed, so a slow moment
    // costs frames rather than adding latency.
    {
        let (latest, writer, sent, holds, e) = (latest.clone(), writer.clone(), sent.clone(), holds.clone(), e.clone());
        std::thread::spawn(move || {
            let mut seen = 0u64;
            loop {
                let (jpeg, arrived, seq) = {
                    let mut g = latest.frame.lock();
                    if g.2 == seen {
                        latest.fresh.wait_for(&mut g, Duration::from_millis(500));
                    }
                    if g.2 == seen {
                        continue;
                    }
                    (g.0.clone(), g.1, g.2)
                };
                seen = seq;
                let Some(jpeg) = jpeg else { continue };
                if !latest.enabled.load(Ordering::SeqCst) {
                    continue;
                }
                // One bad frame must never end the pump: the status would go
                // on saying live while OBS froze on the last picture.
                let wrote = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    writer.lock().write_jpeg(&jpeg, &e)
                })).unwrap_or_else(|_| {
                    log("a frame failed to convert; dropped");
                    false
                });
                if wrote {
                    sent.fetch_add(1, Ordering::SeqCst);
                    let mut h = holds.lock();
                    h.push_back((now() - arrived) * 1000.0);
                    while h.len() > 90 {
                        h.pop_front();
                    }
                }
            }
        });
    }

    let f = font();
    let idle_card = card("Nikon Z8", "Camera is in use for photos in Z8 Studio", None, f.as_ref());
    let nocam_card = card("Camera not connected", "Switch the Z8 on or plug it in. This reconnects by itself.",
                          None, f.as_ref());
    let mut recon: Option<(u64, img::Rgb)> = None;
    let mut link = Link { proc: None, started_at: 0.0, fails: 0, restarts: 0, next_try: 0.0 };
    let (mut state, mut since) = ("idle".to_string(), now());
    let (mut last_status, mut last_slate) = (0.0f64, 0.0f64);
    // Idle only needs to notice a stray link now and then; walking /proc
    // every pass was 20 full process scans a second with nothing to do.
    let mut last_idle_scan = 0.0f64;
    log("frame engine: rust");
    while !STOP.load(Ordering::SeqCst) {
        // Nothing in one pass may end the keeper: a crash here is a dark
        // webcam mid-podcast. Log it and go round again.
        let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if !e.dev.exists() && !e.null_out && !e.file_out {
                std::thread::sleep(Duration::from_secs(1));
                return;
            }
            let ctrl = read_ctrl(&e);
            writer.lock().ensure(if ctrl.live { ctrl.look.as_ref() } else { None });
            let t = now();
            let new: &str;
            if ctrl.live {
                latest.enabled.store(true, Ordering::SeqCst);
                let (last_at, frames_seen) = { let g = latest.frame.lock(); (g.1, g.2) };
                let age = if last_at > 0.0 { t - last_at } else { 1e9 };
                let fresh = age < STALL_S && last_at >= link.started_at;
                if fresh {
                    new = "live";
                    link.fails = 0;
                } else if !camera_present(&e) {
                    new = "no-camera";
                } else {
                    new = "reconnecting";
                    if !link.alive(e.port) {
                        // Count a failed start ONCE per attempt.
                        if link.exited() {
                            if t - link.started_at < 8.0 {
                                link.failed_start();
                            }
                            link.proc = None;
                        }
                        if link.proc.is_none() {
                            link.start(&e);
                        }
                    } else if age > RESTART_S && t - link.started_at > RESTART_S {
                        log(&format!("no frame for {age:.0}s, restarting the link"));
                        link.restarts += 1;
                        link.stop(e.port, 6.0);
                        link.start(&e);
                    }
                }
                if new != "live" && t - last_slate > 0.1 {
                    let mut w = writer.lock();
                    if new == "no-camera" {
                        w.write_card(1, &nocam_card, &e);
                    } else {
                        // Built on first need, and rebuilt when a newer frame exists.
                        if recon.as_ref().map(|r| r.0) != Some(frames_seen) {
                            let base = latest.frame.lock().0.clone();
                            let c = card("Reconnecting the camera...", "This takes a few seconds. Stay put.",
                                         base.as_deref().map(|v| v.as_slice()), f.as_ref());
                            recon = Some((frames_seen, c));
                            w.memo = None;
                        }
                        if let Some((_, c)) = recon.as_ref() {
                            w.write_card(3, c, &e);
                        }
                    }
                    last_slate = t;
                }
            } else {
                latest.enabled.store(false, Ordering::SeqCst);
                if (link.proc.is_some() || t - last_idle_scan > IDLE_SCAN_S) && {
                    last_idle_scan = t;
                    link.alive(e.port)
                } {
                    log("live not wanted, releasing the camera");
                    link.stop(e.port, 6.0);
                }
                new = "idle";
                if t - last_slate > 0.2 {
                    writer.lock().write_card(2, &idle_card, &e);
                    last_slate = t;
                }
            }
            if new != state {
                log(&format!("{state} -> {new}"));
                state = new.to_string();
                since = t;
            }
            if t - last_status > 1.0 {
                let last_at = latest.frame.lock().1;
                let hold = {
                    let mut h: Vec<f64> = holds.lock().iter().copied().collect();
                    h.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    if state == "live" && !h.is_empty() {
                        json!({"p50": round(h[h.len() / 2], 1), "p90": round(h[((h.len() - 1) as f64 * 0.9) as usize], 1),
                               "max": round(h[h.len() - 1], 1), "n": h.len()})
                    } else { Value::Null }
                };
                let w = writer.lock();
                let _ = write_json_atomic(&e.status, &json!({
                    "state": state, "since": since, "pid": std::process::id(),
                    "frames": latest.frames.load(Ordering::SeqCst), "sent": sent.load(Ordering::SeqCst),
                    "lastFrameAge": if last_at > 0.0 { json!(round(t - last_at, 2)) } else { Value::Null },
                    "linkRestarts": link.restarts, "writerRestarts": w.restarts,
                    "engine": format!("rust x{}", e.threads), "rustErrors": 0, "holdMs": hold, "at": t}), 0);
                last_status = t;
            }
            std::thread::sleep(Duration::from_millis(if state == "live" { 30 } else { 50 }));
        }));
        if pass.is_err() {
            log("a pass of the main loop failed; continuing");
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    latest.enabled.store(false, Ordering::SeqCst);
    link.stop(e.port, 6.0);
    let _ = write_json_atomic(&e.status, &json!({"state": "stopped", "at": now()}), 0);
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}
