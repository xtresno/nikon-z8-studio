//! The behaviours selftest.py checked by importing server.py and calling its
//! functions directly, ported so they test THIS implementation. Everything
//! runs in a throwaway home and photo root; no camera is touched.
//!
//!     cargo test --release
use std::path::PathBuf;
use std::sync::Once;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use z8::{advice, analysis, app, camera, feedctl, grade, img, jobs, load, store};

static INIT: Once = Once::new();

/// The job registry is one process-wide table, and pruning is its whole
/// point, so tests that flood it must not run beside one reading it back.
static JOBS_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("z8-rust-tests-{}", std::process::id())).join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// One app for the whole test binary: its own home (with the real
/// presets.json), its own photo root, an empty USB bus and /proc, and camera
/// ceilings scaled 1/12 so a 12 s ceiling is 1 s.
fn setup() {
    INIT.call_once(|| {
        let home = tmp("home");
        std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/presets.json"), home.join("presets.json")).unwrap();
        for (k, v) in [("Z8_HOME", home.clone()), ("Z8_ROOT", tmp("root")), ("Z8_USB_ROOT", tmp("usb")),
                       ("Z8_PROC_ROOT", tmp("proc"))] {
            unsafe { std::env::set_var(k, v) };
        }
        unsafe {
            std::env::set_var("Z8_NO_AUTOCONNECT", "1");
            std::env::set_var("Z8_LOCK_SCALE", format!("{}", 1.0 / 12.0));
            std::env::set_var("Z8_PORT", "8499");
        }
        app::init();
    });
}

fn which(bin: &str) -> bool {
    load::which(bin).is_some()
}

// ---------------------------------------------------------------- camera lock
/// A stalled USB call must not hang later requests: live view and settings
/// give up at their own limits, every other call at its scaled ceiling, as a
/// "busy" the page can retry, and the lock is never left held.
#[test]
fn camera_calls_give_up_instead_of_hanging() {
    setup();
    let (tx, rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let hog = std::thread::spawn(move || {
        camera::cam().hold_lock_for_test(|| {
            tx.send(()).unwrap();
            let _ = done_rx.recv_timeout(Duration::from_secs(12));
        });
    });
    rx.recv().unwrap();
    let t = Instant::now();
    assert!(camera::cam().preview_jpeg().is_none());
    let dt = t.elapsed().as_secs_f64();
    assert!((2.0..6.0).contains(&dt), "live view gave up after {dt:.1}s, wants about 3");
    let t = Instant::now();
    assert!(camera::cam().settings().is_empty());
    let dt = t.elapsed().as_secs_f64();
    assert!((4.0..8.0).contains(&dt), "settings gave up after {dt:.1}s, wants about 5");
    let t = Instant::now();
    let r = camera::cam().set_setting("iso", "640");
    let dt = t.elapsed().as_secs_f64();
    match r {
        Err(camera::Busy(msg)) => assert!(msg.contains("that setting") && msg.contains("timed out"), "{msg}"),
        Ok(_) => panic!("a held camera returned instead of refusing"),
    }
    assert!((0.7..2.5).contains(&dt), "gave up after {dt:.2}s for a 12 s ceiling scaled to 1 s");
    done_tx.send(()).unwrap();
    hog.join().unwrap();
    // And the lock was given back.
    let t = Instant::now();
    let _ = camera::cam().autofocus();
    assert!(t.elapsed() < Duration::from_millis(500), "the lock was left held");
}

// ---------------------------------------------------------------- advice
#[test]
fn exposure_compensation_fractions_are_read_not_zeroed() {
    assert!((advice::ev_value("+1/3") - 1.0 / 3.0).abs() < 1e-9);
    assert_eq!(advice::ev_value("-0.7"), -0.7);
    assert_eq!(advice::ev_value("junk"), 0.0);
    assert_eq!(advice::ev_value(""), 0.0);
    assert_eq!(advice::ev_value("1/0"), 0.0);
}

#[test]
fn live_turn_reads_both_spellings() {
    assert_eq!(advice::live_turn_of("Rotate 270 CW"), 270);
    assert_eq!(advice::live_turn_of("8"), 270);
    assert_eq!(advice::live_turn_of("Rotate 90 CW"), 90);
    assert_eq!(advice::live_turn_of("Horizontal (normal)"), 0);
}

/// A Nikon portrait frame is stored landscape-sized with an Orientation tag;
/// the advice must read the shape as DISPLAYED.
#[test]
fn a_turned_body_is_portrait() {
    setup();
    if !which("exiftool") {
        eprintln!("skipped: no exiftool");
        return;
    }
    let d = tmp("portrait");
    let fp = d.join("portrait.jpg");
    let im = img::Rgb::new(8256, 5504, [120, 100, 90]);
    std::fs::write(&fp, img::jpeg_encode(&im, &img::JpegOpts::q(40)).unwrap()).unwrap();
    let ok = std::process::Command::new("exiftool").args(["-q", "-overwrite_original", "-Orientation#=8"])
        .arg(&fp).status().unwrap().success();
    assert!(ok, "exiftool could not tag the fixture");
    let a = advice::actual_settings(Some(&fp)).expect("a reading");
    assert_eq!((store::py_str(&a["width"]), store::py_str(&a["height"])), ("5504".into(), "8256".into()),
               "shape must be the displayed one: {a}");
    assert!(advice::control_confirms(Some(&a), "tether").iter().any(|c| c.contains("PORTRAIT")));
    assert!(!advice::control_notices(&Default::default(), Some(&a), "tether").iter().any(|n| n.contains("LANDSCAPE")));
    let step = advice::next_step(&advice::Live::default(), Some(&a), "webcam").expect("a step");
    assert!(step["step"].as_str().unwrap().contains("landscape"), "{step}");
    let flat = d.join("landscape.jpg");
    std::fs::write(&flat, img::jpeg_encode(&im, &img::JpegOpts::q(40)).unwrap()).unwrap();
    let b = advice::actual_settings(Some(&flat)).expect("a reading");
    assert_eq!(store::py_str(&b["width"]), "8256");
    assert_eq!(b["turn"], json!(0));
}

// ---------------------------------------------------------------- call look
/// How you look on a call must not depend on the photo you were editing.
#[test]
fn the_call_look_is_its_own_setting() {
    setup();
    let set = |v: &str| { app::app().transient.lock().insert("webcam_look_id".into(), json!(v)); };
    set("clean-headshot");
    let lk = feedctl::webcam_look(None).expect("a named preset resolves");
    assert!(lk.get("exposure").is_some());
    for k in ["id", "label", "note"] {
        assert!(lk.get(k).is_none(), "preset bookkeeping {k} leaked into the look");
    }
    assert!(feedctl::ffmpeg_look(&lk).contains("eq="));
    set("");
    assert!(feedctl::webcam_look(None).is_none(), "no look means none");
    assert!(feedctl::ffmpeg_look(&json!({})).contains("out_range=limited"),
            "no look must still convert full range to limited, or Zoom crushes the shadows");
    set("sliders");
    assert!(feedctl::webcam_look(None).is_none(), "only the page knows the sliders");
    set("does-not-exist");
    assert!(feedctl::webcam_look(None).is_none());
    assert_eq!(feedctl::webcam_look(Some(&json!({"exposure": 1.3}))), Some(json!({"exposure": 1.3})));
    app::app().transient.lock().shift_remove("webcam_look_id");
}

// ---------------------------------------------------------------- images
fn real_frame() -> Option<PathBuf> {
    let root = app::home_dir().join("Pictures/z8-studio");
    let mut all: Vec<PathBuf> = std::fs::read_dir(root).ok()?.filter_map(|d| d.ok())
        .flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().filter_map(|e| e.ok()).map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e == "jpg").unwrap_or(false))
        .filter(|p| std::fs::metadata(p).map(|m| m.len() > 5_000_000).unwrap_or(false)).collect();
    all.sort();
    all.pop()
}

/// A drafted decode is smaller and, thumbnailed, visually the same.
#[test]
fn draft_decoding_is_smaller_and_the_same() {
    setup();
    let Some(f) = real_frame() else { eprintln!("skipped: no real frame"); return };
    let full = load::load_editable(&f, None).unwrap();
    let small = load::load_editable(&f, Some(420)).unwrap();
    assert!(full.w > 4000 && small.w < full.w, "{}x{} vs {}x{}", full.w, full.h, small.w, small.h);
    let (a, b) = (img::thumbnail(&full, 420, 420), img::thumbnail(&small, 420, 420));
    assert_eq!((a.w, a.h), (b.w, b.h));
    let worst = a.px.iter().zip(&b.px).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap();
    assert!(worst <= 8, "worst channel difference {worst}/255");
}

/// Every JPEG preset stays inside its size ceiling and uses a sensible part
/// of it; no preset below quality 90; the print master is the best.
#[test]
fn presets_use_the_headroom_they_have() {
    setup();
    let Some(f) = real_frame() else { eprintln!("skipped: no real frame"); return };
    let im = load::load_editable(&f, None).unwrap();
    let pres = store::presets().unwrap()["export"].as_array().unwrap().clone();
    let (mut over, mut wasteful, mut n) = (vec![], vec![], 0);
    let mut qs: Vec<(String, i64)> = vec![];
    for p in &pres {
        let fmt = p.get("format").and_then(|v| v.as_str()).unwrap_or("JPEG").to_uppercase();
        let (Some(w), Some(h)) = (p.get("w").and_then(|v| v.as_u64()), p.get("h").and_then(|v| v.as_u64())) else { continue };
        if fmt != "JPEG" {
            continue;
        }
        let q = p.get("quality").and_then(|v| v.as_i64()).unwrap_or(92);
        qs.push((p["id"].as_str().unwrap().to_string(), q));
        let fit = grade::fit_to(im.clone(), w as usize, h as usize,
                                p.get("fit").and_then(|v| v.as_str()).unwrap_or("cover"),
                                p.get("anchor").and_then(|v| v.as_str()).unwrap_or("center"));
        let kb = img::jpeg_encode(&fit, &img::JpegOpts { quality: q as i32, subsampling: 0, optimize: false, dpi: 0 })
            .unwrap().len() as f64 / 1024.0;
        n += 1;
        if let Some(cap) = p.get("max_kb").and_then(|v| v.as_f64()) {
            if kb > cap { over.push(format!("{} {kb:.0}>{cap}", p["id"])); }
            if kb < cap * 0.15 { wasteful.push(format!("{} {kb:.0} of {cap}", p["id"])); }
        }
    }
    assert!(n > 0);
    assert!(over.is_empty(), "over their ceiling: {over:?}");
    assert!(wasteful.is_empty(), "wasting their ceiling: {wasteful:?}");
    assert!(qs.iter().all(|(_, q)| *q >= 90), "{qs:?}");
    let master = qs.iter().find(|(k, _)| k == "headshot-8x10").map(|x| x.1).unwrap_or(0);
    assert!(qs.iter().filter(|(k, _)| k != "full").all(|(_, q)| *q <= master), "{qs:?}");
}

/// Feeding each knob's untouched value in gives back the frame bit for bit,
/// which is what lets the COPY export say which edits it is dropping.
#[test]
fn the_adjustment_defaults_are_the_real_ones() {
    let mut im = img::Rgb::new(64, 48, [0, 0, 0]);
    for (i, p) in im.px.iter_mut().enumerate() {
        *p = ((i * 37) % 251) as u8;
    }
    for (k, v) in grade::ADJ_DEFAULTS {
        if *k == "straighten" {
            continue;
        }
        let out = grade::apply_edits(&im, &json!({ *k: v }), grade::Phase::All);
        assert!(out.px == im.px, "{k} at its default changed the frame");
    }
    let lut = grade::tone_lut(0.0, 0.0);
    assert!(lut.iter().enumerate().all(|(i, v)| *v as usize == i));
    let lift = grade::tone_lut(0.5, 0.0);
    assert!(lift.windows(2).all(|w| w[0] <= w[1]), "a shadow lift must stay monotonic");
}

// ---------------------------------------------------------------- analysis
/// Focus is judged against the set's 90th percentile, never an absolute,
/// and the worst single fault decides the order.
#[test]
fn ranking_is_relative_and_worst_fault_first() {
    let row = |n: &str, focus: f64, face: f64| json!({"name": n, "ok": true, "focus": focus, "clipped": 0.0,
                                                      "faceY": face, "catchlight": 0.01, "skinInBox": 0.8});
    let mut set: Vec<Value> = (0..9).map(|i| row(&format!("f{i}"), 200.0 + i as f64, 138.0)).collect();
    set.push(row("outlier", 900.0, 138.0));   // one unusually detailed frame
    set.push(row("soft", 60.0, 138.0));
    set.push(json!({"name": "noface", "ok": false, "why": "no face"}));
    let ranked = analysis::rank_scored(set);
    let by = |n: &str| ranked.iter().find(|r| r["name"] == n).unwrap().clone();
    assert!(by("f3")["faults"].as_array().unwrap().is_empty(),
            "one outlier must not make the sharp pack soft: {}", by("f3"));
    assert!(by("soft")["faults"].as_array().unwrap().contains(&json!("soft")));
    assert_eq!(ranked.last().unwrap()["name"], "noface", "unscored frames go last");
    let pos = |n: &str| ranked.iter().position(|r| r["name"] == n).unwrap();
    assert!(pos("soft") > pos("f3"), "the soft frame ranks below the clean ones");
}

// ---------------------------------------------------------------- plumbing
/// Twenty-four jobs started at once get twenty-four ids: a millisecond
/// timestamp collided six times.
#[test]
fn job_ids_never_collide() {
    let _g = JOBS_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let ids: Vec<String> = (0..24).map(|_| std::thread::spawn(|| jobs::new("t", 1)))
        .collect::<Vec<_>>().into_iter().map(|h| h.join().unwrap()).collect();
    let uniq: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(uniq.len(), 24, "{ids:?}");
}

/// A wrong-typed field is refused and named, never coerced.
#[test]
fn a_wrong_typed_body_is_refused_by_name() {
    let e = z8::http::body_type_error(&json!({"name": [1, 2]})).unwrap();
    assert!(e.contains("'name'") && e.contains("str") && e.contains("list"), "{e}");
    assert!(z8::http::body_type_error(&json!({"name": "x.jpg", "preset": {}, "names": []})).is_none());
    assert!(z8::http::body_type_error(&json!({"crop": null})).is_none(), "null means absent");
    assert!(z8::http::body_type_error(&json!({"preset": "notadict"})).is_some());
}

/// A focus drive or focus point the camera cannot use is refused before the
/// camera is asked: no steps, zero, a fraction that rounds to zero, a string,
/// past the lens range, or a point that would wrap when sent unsigned.
#[test]
fn focus_moves_the_camera_cannot_use_are_refused() {
    setup();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let post = |uri: &str, body: Value| {
        let req = axum::http::Request::builder().method("POST").uri(uri)
            .header("origin", "http://127.0.0.1:8499").header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string())).unwrap();
        let r = rt.block_on(z8::http::handle(req));
        let code = r.status().as_u16();
        let bytes = rt.block_on(axum::body::to_bytes(r.into_body(), 1 << 20)).unwrap();
        (code, serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null))
    };
    for b in [json!({}), json!({"steps": 0}), json!({"steps": 0.4}), json!({"steps": "3000"}),
              json!({"steps": 32768}), json!({"steps": -40000}), json!({"steps": null})] {
        let (code, v) = post("/api/focus", b.clone());
        assert_eq!(code, 400, "/api/focus {b}");
        assert!(v["ok"] == json!(false) && v["error"].as_str().unwrap().contains("steps"), "{b}: {v}");
    }
    for b in [json!({}), json!({"x": 10}), json!({"y": 10}), json!({"x": -1, "y": 5}),
              json!({"x": "10", "y": "5"}), json!({"x": 40000, "y": 1}), json!({"x": 1, "y": 70000})] {
        let (code, v) = post("/api/af-area", b.clone());
        assert_eq!(code, 400, "/api/af-area {b}");
        assert!(v["ok"] == json!(false) && v["error"].as_str().unwrap().contains("x and y"), "{b}: {v}");
    }
}

/// One naming rule, a broken template never loses the export, and no
/// template can write outside the folder.
#[test]
fn export_names_follow_the_template_and_stay_in_the_folder() {
    setup();
    let pre = json!({"id": "YT Thumb", "w": 1280, "h": 720, "format": "JPEG"});
    assert_eq!(store::export_name("20260921-1.jpg", &pre, None), "20260921-1_yt-thumb.jpg");
    let set = |v: &str| { app::app().transient.lock().insert("name_template".into(), json!(v)); };
    set("{stem}_{w}x{h}");
    assert_eq!(store::export_name("a.jpg", &pre, None), "a_1280x720.jpg");
    set("{stem}_{nope}");
    assert_eq!(store::export_name("a.jpg", &pre, None), "a_yt-thumb.jpg", "a broken template falls back");
    set("../../{stem}");
    assert!(!store::export_name("a.jpg", &pre, None).contains('/'), "a template cannot climb out");
    app::app().transient.lock().shift_remove("name_template");
    assert_eq!(store::export_name("a.heic", &json!({"id": "full", "format": "PNG"}), None), "a_full.png");
}

#[test]
fn the_thumbnail_size_matches_pillow() {
    assert_eq!(img::thumbnail_size(8256, 5504, 240, 240), Some((240, 160)));
    assert_eq!(img::thumbnail_size(5504, 8256, 240, 720), Some((240, 360)));
    assert_eq!(img::thumbnail_size(100, 100, 240, 240), None, "never enlarges");
    assert_eq!(img::draft_denom(8256, 5504, 2200), 2);
    assert_eq!(img::draft_denom(8256, 5504, 420), 8);
}

/// A full-range MJPEG-like source: what the camera actually sends. lavfi's
/// own output is already limited range, so a control case built from it
/// would look fine without the conversion and prove nothing.
fn fullrange_jpeg(name: &str, size: &str) -> PathBuf {
    let p = tmp("ff").join(name);
    let ok = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", &format!("testsrc2=size={size}:rate=1:duration=1"),
               "-vf", "format=yuvj420p", "-frames:v", "1", "-q:v", "2"]).arg(&p)
        .status().map(|s| s.success()).unwrap_or(false);
    assert!(ok, "ffmpeg could not make the source frame");
    p
}

fn y_range(src: &PathBuf, vf: &str) -> (u8, u8) {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"]).arg(src)
        .args(["-vf", vf, "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"]).output().unwrap();
    assert!(out.status.success(), "ffmpeg refused {vf}: {}", String::from_utf8_lossy(&out.stderr));
    let y = &out.stdout[..320 * 180];
    (*y.iter().min().unwrap(), *y.iter().max().unwrap())
}

fn looks() -> Vec<Value> {
    let p: Value = serde_json::from_str(&std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/presets.json")).unwrap()).unwrap();
    p["look"].as_array().unwrap().clone()
}

/// Every webcam look builds a graph ffmpeg accepts, ends in yuv420p for
/// v4l2loopback, and lands the Zoom feed in broadcast range: the camera sends
/// full range, v4l2loopback carries no range metadata, and OBS and Zoom
/// assume limited, so without the conversion the shadows crush.
#[test]
fn every_webcam_look_lands_in_broadcast_range() {
    if !which("ffmpeg") {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let src = fullrange_jpeg("fullrange.jpg", "320x180");
    let (lo0, hi0) = y_range(&src, "format=yuv420p");
    assert!(lo0 < 16 || hi0 > 236, "control case already in range: Y {lo0}..{hi0}");
    let all = looks();
    assert!(all.len() >= 3, "only {} looks", all.len());
    for l in &all {
        let vf = feedctl::ffmpeg_look(l);
        assert!(vf.ends_with("format=yuv420p"), "{}: {vf}", l["label"]);
        assert!(vf.contains("scale=in_range=full:out_range=limited"), "{}: {vf}", l["label"]);
        let (lo, hi) = y_range(&src, &vf);
        assert!((16..=236).contains(&lo) && hi <= 236, "{} Y {lo}..{hi}", l["label"]);
    }
}

/// The Zoom feed and an export are the same grade. ffmpeg's brightness is
/// additive where the grade's exposure is multiplicative, so they can only be
/// close; hold the measured line so an edit cannot quietly widen it.
#[test]
fn the_zoom_look_matches_the_export_look() {
    setup();
    if !which("ffmpeg") {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let src = fullrange_jpeg("parity.jpg", "640x360");
    let base = load::load_editable(&src, None).unwrap();
    let (mut drift, mut worst) = (0.0f64, String::new());
    for l in looks() {
        let want = img::mean_rgb(&grade::apply_edits(&base, &l, grade::Phase::All));
        let vf = feedctl::ffmpeg_look(&l).replace("scale=in_range=full:out_range=limited,", "");
        let out = tmp("ff").join(format!("pff-{}.jpg", l["id"].as_str().unwrap_or("x")));
        let ok = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-i"]).arg(&src).args(["-vf", &vf, "-frames:v", "1", "-q:v", "2"]).arg(&out)
            .status().unwrap().success();
        assert!(ok, "ffmpeg refused {vf}");
        let got = img::mean_rgb(&load::load_editable(&out, None).unwrap());
        let d = want.iter().zip(got).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        if d > drift {
            drift = d;
            worst = l["id"].to_string();
        }
    }
    assert!(drift < 7.0, "worst {drift:.1}/255 on {worst}");
}

/// A running job is never evicted by later ones, and the finished history is
/// still bounded, so the page never 404s on its own batch.
#[test]
fn running_jobs_survive_and_finished_ones_are_pruned() {
    let _g = JOBS_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let running: Vec<String> = (0..24).map(|_| jobs::new("keep", 3)).collect();
    for _ in 0..60 {
        let j = jobs::new("prune", 1);
        jobs::finish(&j, "done");
    }
    assert!(running.iter().all(|j| jobs::get(j).is_some()), "a running job was evicted");
    let one_more = jobs::new("prune", 1);
    jobs::finish(&one_more, "done");
    // 40 kept, plus the one just finished.
    let kept = jobs::finished_count();
    assert!(kept <= 41, "{kept} finished jobs kept");
    for j in &running {
        jobs::finish(j, "done");
    }
}

/// Whatever a job body does, including panic, the job reaches a terminal
/// state and says why, rather than spinning "running" forever.
#[test]
fn a_panicking_job_still_finishes_and_says_why() {
    let _g = JOBS_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let id = jobs::new("boom", 2);
    jobs::spawn(id.clone(), || panic!("disk vanished"));
    let t = Instant::now();
    let mut state = Value::Null;
    while t.elapsed() < Duration::from_secs(5) {
        state = jobs::get(&id).map(|j| j["state"].clone()).unwrap_or(Value::Null);
        if state != "running" {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(state, "failed");
    let errs = jobs::get(&id).unwrap()["errors"].to_string();
    assert!(errs.contains("disk vanished"), "{errs}");
}

/// A fake card: folder -> (subfolders, files), counting every USB round trip.
struct FakeCard {
    tree: std::collections::HashMap<String, (Vec<String>, Vec<String>)>,
    cyclic: bool,
    bad_files: Option<String>,
    bad_folders: Option<String>,
    calls: std::cell::Cell<usize>,
}

impl FakeCard {
    fn nikon() -> Self {
        let mut tree = std::collections::HashMap::new();
        tree.insert("/".into(), (vec!["DCIM".into()], vec![]));
        tree.insert("/DCIM".into(), ((100..110).map(|i| format!("{i}NZ_8")).collect(), vec![]));
        for i in 100..110 {
            tree.insert(format!("/DCIM/{i}NZ_8"), (vec![], (0..20).map(|j| format!("DSC_{i}{j:02}.NEF")).collect()));
        }
        FakeCard { tree, cyclic: false, bad_files: None, bad_folders: None, calls: 0.into() }
    }

    fn walk(&self, limit: usize) -> camera::CardListing {
        let folders = |d: &str| -> Result<Vec<String>, String> {
            self.calls.set(self.calls.get() + 1);
            if self.bad_folders.as_deref() == Some(d) {
                return Err("[-1] Unspecified error".into());
            }
            if self.cyclic && d != "/" {
                return Ok(vec!["loop".into()]);
            }
            Ok(self.tree.get(d).map(|t| t.0.clone()).unwrap_or_default())
        };
        let files = |d: &str| -> Result<Vec<String>, String> {
            self.calls.set(self.calls.get() + 1);
            if self.bad_files.as_deref() == Some(d) {
                return Err("[-1] Unspecified error".into());
            }
            Ok(self.tree.get(d).map(|t| t.1.clone()).unwrap_or_default())
        };
        camera::walk_card(&mut { folders }, &mut { files }, limit)
    }
}

/// The card walk: every file on a normal card, a limit that bounds the USB
/// work and not just the result, a cyclic tree that terminates, and one
/// unreadable folder that does not lose the rest of the card.
#[test]
fn the_card_walk_is_bounded_and_survives_a_bad_folder() {
    let card = FakeCard::nikon();
    let (files, err, trunc) = card.walk(400);
    assert_eq!((files.as_ref().map(|f| f.len()), err.is_none(), trunc), (Some(200), true, false));
    let full_calls = card.calls.get();
    assert_eq!(files.unwrap()[0]["name"], "DSC_10919.NEF", "newest first");

    let small = FakeCard::nikon();
    let (files, _, trunc) = small.walk(25);
    assert!(files.unwrap().len() == 25 && trunc);
    assert!(small.calls.get() < full_calls, "{} calls vs {full_calls}", small.calls.get());

    let mut cyc = FakeCard::nikon();
    cyc.cyclic = true;
    let (_, _, trunc) = cyc.walk(400);
    assert!(trunc, "a cyclic tree must end truncated");

    for (bad_files, bad_folders) in [(Some("/DCIM/105NZ_8"), None), (None, Some("/DCIM/105NZ_8"))] {
        let mut c = FakeCard::nikon();
        c.bad_files = bad_files.map(String::from);
        c.bad_folders = bad_folders.map(String::from);
        let (files, err, trunc) = c.walk(400);
        assert_eq!(files.map(|f| f.len()), Some(200 - if bad_files.is_some() { 20 } else { 0 }),
                   "one bad folder lost the card ({bad_files:?} {bad_folders:?})");
        assert!(err.is_some() && trunc, "the failure is reported, not hidden");
    }

    let mut none = FakeCard::nikon();
    none.bad_folders = Some("/".into());
    let (files, err, _) = none.walk(400);
    assert!(files.is_none() && err.is_some(), "an unreadable root is no card at all");
}

/// Recording lives on the camera; releasing it clears the flag, so the page
/// never shows a recording badge for a camera that is gone.
#[test]
fn releasing_the_camera_clears_the_recording_flag() {
    setup();
    let c = camera::cam();
    let was_off = c.a.lock().manual_off;
    c.a.lock().recording = true;
    c.disconnect();
    let rec = c.a.lock().recording;
    c.a.lock().manual_off = was_off;
    assert!(!rec);
    assert!(!c.is_open(), "published recording is ANDed with an open camera");
}

/// The extension is a claim, not a fact: the Z8 names HEIF frames .nef, so a
/// download is renamed by its magic bytes.
#[test]
fn a_download_is_named_by_its_bytes() {
    let d = tmp("normalise");
    for (name, magic, want) in [("liar.nef", &b"\xff\xd8\xff\xe0"[..], "jpg"), ("real.nef", b"II*\x00", "nef"),
                                ("moto.nef", b"MM\x00*", "nef"), ("mystery.bin", b"NOTANIMAGE", "bin")] {
        let p = d.join(name);
        let mut bytes = magic.to_vec();
        bytes.extend([0u8; 32]);
        std::fs::write(&p, bytes).unwrap();
        let out = load::normalise(&p);
        assert_eq!(out.extension().unwrap(), want, "{name} -> {out:?}");
        assert!(out.exists(), "{out:?} is on disk");
    }
}

/// The camera probe matches the Nikon vendor id, not any USB device: a bus
/// holding only a Logitech reads as absent, and one with a Nikon as present.
#[test]
fn only_a_nikon_counts_as_the_camera() {
    let root = tmp("usb-probe");
    let dev = |name: &str, vendor: &str| {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("idVendor"), format!("{vendor}\n")).unwrap();
    };
    dev("1-2", "046d");
    assert_eq!(camera::usb_has_nikon(&root), Some(false));
    dev("1-3", "04B0");
    assert_eq!(camera::usb_has_nikon(&root), Some(true));
    assert_eq!(camera::usb_has_nikon(&root.join("missing")), None);
}

fn notices(requested: Value, actual: Value) -> Vec<String> {
    advice::control_notices(requested.as_object().unwrap(), Some(&actual), "tether")
}

/// The camera-notice detector, driven directly with synthetic readings. The
/// live checks only run while the camera happens to be misconfigured, so
/// without this the detector went untested the moment the user fixed it.
#[test]
fn a_dead_control_says_so() {
    let hlg = json!({"transfer": "BT.2100 HLG, ARIB STD-B67", "primaries": "BT.2020, BT.2100",
                     "pictureControl": "Standard(HLG)", "bitDepth": "10", "iso": "1400", "autoISO": "On"});
    let syn = notices(json!({"iso": {"value": "640"}}), hlg.clone());
    assert!(syn.first().is_some_and(|n| n.contains("HLG TONE MODE")), "HLG first: {syn:?}");
    let hn = &syn[0];
    for want in ["Photo Shooting Menu", "Tone mode", "cannot be changed over USB", "gamut", "tone curve"] {
        assert!(hn.contains(want), "HLG notice lacks {want:?}");
    }
    let mut arib = hlg.clone();
    arib["transfer"] = json!("ARIB STD-B67");
    arib["pictureControl"] = json!("Standard");
    assert!(notices(json!({}), arib).iter().any(|n| n.contains("HLG TONE MODE")), "ARIB alone is HLG");

    let sdr = json!({"transfer": "BT.709", "primaries": "BT.709", "pictureControl": "Portrait",
                     "iso": "320", "autoISO": "Off"});
    let quiet = notices(json!({"iso": {"value": "320"}}), sdr);
    assert!(quiet.is_empty(), "a correctly set camera gets no notices: {quiet:?}");

    let ec = |prog: &str, comp: &str, auto: &str| notices(json!({}), json!({"exposureProgram": prog,
        "exposureCompensation": comp, "autoISO": auto, "transfer": "BT.709"}));
    let dead = ec("Manual", "+1", "Off");
    assert!(dead.iter().any(|n| n.contains("doing nothing") && n.contains("aperture, shutter or ISO")), "{dead:?}");
    assert!(!ec("Manual", "+1", "On").iter().any(|n| n.contains("doing nothing")), "it works with Auto ISO");
    assert!(!ec("Manual", "0", "Off").iter().any(|n| n.contains("doing nothing")), "zero is not worth a warning");
    assert!(!ec("Aperture-priority", "+1", "Off").iter().any(|n| n.contains("doing nothing")), "it steers A mode");

    let he = notices(json!({}), json!({"nefCompression": "14", "transfer": "BT.709"}));
    assert!(he.iter().any(|n| n.contains("High Efficiency")));
    assert!(!notices(json!({}), json!({"nefCompression": "4", "transfer": "BT.709"}))
        .iter().any(|n| n.contains("High Efficiency")));

    // Aperture is compared as a number: f/5 and f/5.0 are one aperture, and
    // an unreadable value can never fire a notice.
    let ap = |asked: &str, got: &str| notices(json!({"aperture": {"value": asked}}),
        json!({"fNumber": got, "transfer": "BT.709"})).into_iter().filter(|n| n.starts_with("Aperture")).count();
    assert_eq!(ap("f/5", "5.0"), 0);
    assert_eq!(ap("f/5.6", "8.0"), 1);
    assert_eq!(ap("?", "8.0"), 0);
    assert!(advice::control_notices(&serde_json::Map::new(), None, "tether").is_empty(), "no reading, no claim");
}

/// Confirmations for a setting he fixed by hand: each correct setting earns
/// its own line, and each wrong one withdraws exactly its own line.
#[test]
fn a_setting_he_fixed_by_hand_is_confirmed() {
    let good = json!({"iso": "320", "autoISO": "Off", "exposureProgram": "Manual",
                      "pictureControl": "Portrait", "width": "8256", "height": "5504"});
    let cc = |over: &[(&str, &str)]| {
        let mut a = good.clone();
        for (k, v) in over {
            a[*k] = json!(v);
        }
        advice::control_confirms(Some(&a), "tether")
    };
    assert!(advice::control_confirms(None, "tether").is_empty(), "no frame read, no reassurance");
    assert!(advice::control_confirms(Some(&json!({})), "tether").is_empty(), "an empty reading says nothing");
    let land = cc(&[]);
    let joined = land.join(" ");
    assert!(land.len() >= 2 && joined.contains("SDR") && joined.contains("320"), "{land:?}");
    assert!(!joined.to_uppercase().contains("PORTRAIT"), "landscape is not congratulated");
    assert!(cc(&[("width", "5504"), ("height", "8256")]).iter().any(|x| x.contains("PORTRAIT")));
    let hlg = cc(&[("pictureControl", "Standard(HLG)")]);
    assert!(!hlg.iter().any(|x| x.contains("SDR")) && hlg.iter().any(|x| x.contains("Auto ISO off")), "{hlg:?}");
    for (f, v) in [("transfer", "BT.2100 HLG, ARIB STD-B67"), ("primaries", "BT.2020, BT.2100")] {
        assert!(!cc(&[(f, v)]).iter().any(|x| x.contains("SDR")), "HLG through {f}");
    }
    let auto = cc(&[("autoISO", "On")]);
    assert!(!auto.iter().any(|x| x.contains("Auto ISO off")) && auto.iter().any(|x| x.contains("SDR")), "{auto:?}");
    assert!(!cc(&[("exposureProgram", "Aperture-priority AE")]).iter().any(|x| x.contains("Auto ISO off")));
}

/// A made-up world for the next-step ladder: no camera, no session.
#[derive(Default, Clone)]
struct Fake {
    open: bool,
    off: bool,
    recording: bool,
    webcam: bool,
    shots: Vec<Value>,
    clips: Vec<Value>,
    picks: serde_json::Map<String, Value>,
    stems: Vec<Value>,
}

impl advice::World for Fake {
    fn camera_open(&self) -> bool { self.open }
    fn manual_off(&self) -> bool { self.off }
    fn recording(&self) -> bool { self.recording }
    fn webcam(&self) -> bool { self.webcam }
    fn shots(&self) -> Vec<Value> { self.shots.clone() }
    fn clips(&self) -> Vec<Value> { self.clips.clone() }
    fn picks(&self) -> serde_json::Map<String, Value> { self.picks.clone() }
    fn stems(&self) -> Vec<Value> { self.stems.clone() }
}

fn frames(n: usize) -> Vec<Value> {
    (0..n).map(|i| json!({"name": format!("f{i}.jpg"), "at": i as f64})).collect()
}

fn step(w: &Fake, a: &Value) -> Value {
    advice::next_step(w, Some(a), &advice::current_mode(w)).unwrap_or(Value::Null)
}

/// The ladder names ONE next thing, in order: connect, camera setup (portrait
/// outranks HLG and Auto ISO), shoot, cull, export, done.
#[test]
fn the_app_says_what_to_do_next() {
    let good = json!({"width": "5504", "height": "8256", "autoISO": "Off", "pictureControl": "Portrait"});
    let with = |k: &[(&str, &str)]| {
        let mut a = good.clone();
        for (f, v) in k {
            a[*f] = json!(v);
        }
        a
    };
    let land = [("width", "8256"), ("height", "5504")];
    let mut w = Fake::default();
    assert_eq!(step(&w, &good)["stage"], 0, "a disconnected camera comes first");
    w.off = true;
    assert_ne!(step(&w, &good)["stage"], 0, "but not when HE turned it off");

    let mut w = Fake { open: true, ..Fake::default() };
    let l = step(&w, &with(&land));
    assert!(l["stage"] == 1 && l["step"].as_str().unwrap().to_lowercase().contains("portrait"), "{l}");
    let both = step(&w, &with(&[land[0], land[1], ("autoISO", "On"), ("pictureControl", "Standard(HLG)")]));
    assert!(both["step"].as_str().unwrap().to_lowercase().contains("portrait"), "orientation outranks: {both}");
    assert!(step(&w, &with(&[("pictureControl", "Standard(HLG)")]))["step"].as_str().unwrap().contains("HLG"));
    assert!(step(&w, &with(&[("autoISO", "On")]))["step"].as_str().unwrap().contains("Auto ISO"));
    assert_eq!(step(&w, &good)["stage"], 2, "camera right and no frames: shoot");

    w.shots = frames(6);
    let cull = step(&w, &good);
    assert!(cull["stage"] == 3 && cull["step"].as_str().unwrap().contains('6'), "{cull}");
    w.picks = serde_json::from_value(json!({"f0.jpg": "pick", "f1.jpg": "reject"})).unwrap();
    assert!(step(&w, &good)["step"].as_str().unwrap().contains('4'), "counts what is LEFT");
    w.picks = (0..6).map(|i| (format!("f{i}.jpg"), json!(if i < 4 { "pick" } else { "reject" }))).collect();
    let exp = step(&w, &good);
    assert!(exp["stage"] == 4 && exp["step"].as_str().unwrap().contains('4'), "{exp}");
    w.stems = (0..2).map(|i| json!({"stem": format!("f{i}"), "state": "current"})).collect();
    assert!(step(&w, &good)["step"].as_str().unwrap().contains('2'), "exported keepers are not counted again");
    w.stems = (0..4).map(|i| json!({"stem": format!("f{i}"), "state": "current"})).collect();
    assert!(step(&w, &good)["step"].as_str().unwrap().contains("Done"));
    w.picks = (0..6).map(|i| (format!("f{i}.jpg"), json!("reject"))).collect();
    let none = step(&w, &good);
    assert!(none["stage"] == 3 && none["step"].as_str().unwrap().to_lowercase().contains("rejected"), "{none}");

    // Every rung carries a reason, or it is an order without an argument.
    for (label, w, a) in [("off", Fake::default(), good.clone()),
                          ("landscape", Fake { open: true, ..Fake::default() }, with(&land)),
                          ("shoot", Fake { open: true, ..Fake::default() }, good.clone())] {
        assert!(step(&w, &a)["why"].as_str().unwrap_or("").len() > 30, "the {label} rung explains itself");
    }
}

fn land() -> Value {
    json!({"width": "8256", "height": "5504", "autoISO": "Off", "exposureProgram": "Manual",
           "iso": "320", "pictureControl": "Portrait"})
}

fn port() -> Value {
    let mut p = land();
    p["width"] = json!("5504");
    p["height"] = json!("8256");
    p
}

fn hlg(mut a: Value) -> Value {
    a["pictureControl"] = json!("Standard(HLG)");
    a
}

fn step_in(w: &Fake, a: Option<&Value>, mode: &str) -> String {
    advice::next_step(w, a, mode).map(|r| r["step"].as_str().unwrap_or("").to_string()).unwrap_or_default()
}

/// One invariant across all three ladders: if a mode's own work is available,
/// that mode never asks for the camera. Mid-Zoom the camera handle is closed
/// for the whole call (the feed holds the session), and the panel used to say
/// "Turn the camera on" while he was live on it.
#[test]
fn no_ladder_asks_for_a_camera_that_is_already_working() {
    let asks = |w: &Fake, a: Option<&Value>| step_in(w, a, &advice::current_mode(w)).to_lowercase()
        .contains("turn the camera on");
    let clip = vec![json!({"name": "t.mov", "at": 100.0})];
    assert!(!asks(&Fake { webcam: true, ..Fake::default() }, Some(&land())), "a call is live");
    assert!(!asks(&Fake { clips: clip, ..Fake::default() }, Some(&land())), "a take is on disk");
    assert!(!asks(&Fake { shots: frames(1), ..Fake::default() }, Some(&port())), "frames are waiting");
    assert!(step_in(&Fake::default(), None, "tether").contains("Turn the camera on"), "stills, nothing to do");
    assert!(step_in(&Fake::default(), None, "movie").contains("Turn the camera on"), "movie, no takes");
    assert!(!step_in(&Fake { off: true, ..Fake::default() }, None, "tether").contains("Turn the camera on"),
            "a camera HE turned off is left alone");

    // Exactly three ladders, all covered above, and the webcam branch never
    // looks at the camera handle.
    let src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/advice.rs"));
    let body = &src[src.find("pub fn next_step(").unwrap()..];
    let body = &body[..body.find("\n}\n").unwrap()];
    assert_eq!(body.matches("let l = [\"Connect\"").count(), 3, "a new ladder needs a case above");
    let webcam = &body[body.find("if mode == \"webcam\"").unwrap()..body.find("let offline").unwrap()];
    assert!(!webcam.contains("camera_open") && !webcam.contains("is_open"), "webcam branch checks the handle");
}

/// Stills, a call and a skit are three modes with three right answers about
/// orientation, and a skit stays a skit between takes.
#[test]
fn recording_a_skit_is_its_own_mode() {
    let mut w = Fake { open: true, ..Fake::default() };
    assert_eq!(advice::current_mode(&w), "tether");
    w.webcam = true;
    assert_eq!(advice::current_mode(&w), "webcam");
    w.webcam = false;
    w.recording = true;
    assert_eq!(advice::current_mode(&w), "movie", "rolling");
    w.recording = false;
    w.clips = vec![json!({"name": "a.mov", "at": 100.0})];
    assert_eq!(advice::current_mode(&w), "movie", "it STAYS movie between takes");
    w.shots = vec![json!({"name": "s.jpg", "at": 200.0})];
    assert_eq!(advice::current_mode(&w), "tether", "a still switches back");
    w.clips = vec![json!({"name": "b.mov", "at": 300.0})];
    assert_eq!(advice::current_mode(&w), "movie", "recording again switches back");

    let w = Fake { open: true, ..Fake::default() };
    assert!(!step_in(&w, Some(&land()), "movie").to_lowercase().contains("portrait"));
    assert!(step_in(&w, Some(&port()), "movie").to_lowercase().contains("landscape"));
    assert!(!step_in(&w, Some(&hlg(land())), "movie").contains("HLG"), "HLG is a real video format");
    assert!(step_in(&w, Some(&hlg(port())), "tether").contains("HLG"), "but a stills defect");
    let ladders: std::collections::HashSet<String> = ["movie", "webcam", "tether"].iter()
        .map(|m| advice::next_step(&w, Some(&land()), m).unwrap()["ladder"].to_string()).collect();
    assert_eq!(ladders.len(), 3);
    assert!(step_in(&w, Some(&land()), "movie").contains("Record a take"));
    for (n, word) in [(1, "1 take"), (3, "3 takes")] {
        let w = Fake { open: true, clips: (0..n).map(|i| json!({"name": format!("{i}.mov"), "at": i})).collect(),
                       ..Fake::default() };
        assert!(step_in(&w, Some(&land()), "movie").contains(word), "{word}");
    }
    for (n, frame, keeper) in [(1, "1 frame", "1 keeper"), (2, "2 frames", "2 keepers")] {
        let mut w = Fake { open: true, shots: frames(n), ..Fake::default() };
        assert!(step_in(&w, Some(&port()), "tether").ends_with(frame), "{frame}");
        w.picks = (0..n).map(|i| (format!("f{i}.jpg"), json!("pick"))).collect();
        assert!(step_in(&w, Some(&port()), "tether").contains(keeper), "{keeper}");
    }
}

/// Portrait is right for a headshot and wrong for a call: the step, the
/// landscape warning and the portrait confirmation all flip with the mode.
#[test]
fn orientation_advice_follows_the_mode() {
    let w = Fake { open: true, ..Fake::default() };
    let t_land = advice::next_step(&w, Some(&land()), "tether").unwrap();
    assert!(t_land["step"].as_str().unwrap().to_lowercase().contains("portrait"));
    assert!(advice::next_step(&w, Some(&port()), "tether").unwrap()["stage"].as_i64().unwrap() >= 2);
    let w_land = advice::next_step(&w, Some(&land()), "webcam").unwrap();
    let wl = w_land["step"].as_str().unwrap();
    assert!(!wl.to_lowercase().contains("portrait") && wl.contains("Zoom"), "{wl}");
    assert!(step_in(&w, Some(&port()), "webcam").to_lowercase().contains("landscape"));
    assert_ne!(w_land["ladder"], t_land["ladder"]);
    assert!(step_in(&w, Some(&hlg(port())), "tether").contains("HLG"));
    assert!(step_in(&w, Some(&hlg(land())), "webcam").contains("HLG"));

    let pt = advice::control_confirms(Some(&port()), "tether").join(" ");
    let pw = advice::control_confirms(Some(&port()), "webcam").join(" ");
    assert!(pt.contains("PORTRAIT") && !pw.contains("PORTRAIT"), "{pt} / {pw}");
    assert!(pw.contains("SDR") && pw.contains("Auto ISO off"), "{pw}");
    let none = serde_json::Map::new();
    let warn = |m: &str| advice::control_notices(&none, Some(&land()), m).iter().any(|n| n.contains("LANDSCAPE"));
    assert!(warn("tether") && !warn("webcam"), "the landscape warning is gated on the mode");
}

/// Tidy exports: a frame in the trash still counts as present (it is one
/// button from coming back), a name that follows no template is "unknown"
/// and never an orphan, a dry run moves nothing, and confirming moves an
/// orphan to the trash, recoverable, with collisions renamed.
#[test]
fn old_exports_can_be_tidied_away() {
    setup();
    let day = "2001-01-01";
    let sess = store::session_dir(Some(day));
    let exp = store::export_dir(Some(day));
    std::fs::create_dir_all(sess.join(".trash")).unwrap();
    std::fs::create_dir_all(&exp).unwrap();
    std::fs::write(sess.join("a.jpg"), b"x").unwrap();
    std::fs::write(sess.join(".trash").join("b.jpg"), b"x").unwrap();
    for n in ["a_head.jpg", "b_head.jpg", "c_head.jpg", "loose.jpg"] {
        std::fs::write(exp.join(n), b"\xff\xd8").unwrap();
    }
    let rows = store::export_stems(Some(day));
    let state = |n: &str| rows.iter().find(|r| r["name"] == n).map(|r| r["state"].clone()).unwrap_or(Value::Null);
    assert_eq!(state("a_head.jpg"), "current");
    assert_eq!(state("b_head.jpg"), "current", "a frame in the trash still counts as present");
    assert_eq!(state("c_head.jpg"), "orphan");
    assert_eq!(state("loose.jpg"), "unknown", "an unrecognised name is never an orphan");

    let dry = store::tidy_exports(Some(day), true);
    assert_eq!(dry["dryRun"], true);
    assert!(exp.join("c_head.jpg").exists(), "a dry run moves nothing");
    // A same-named file already in the trash: the move must not clobber it.
    std::fs::write(sess.join(".trash").join("c_head.jpg"), b"older").unwrap();
    let did = store::tidy_exports(Some(day), false);
    assert_eq!(did["moved"], json!(["c_head.jpg"]), "{did}");
    assert!(!exp.join("c_head.jpg").exists());
    assert_eq!(std::fs::read(sess.join(".trash").join("c_head.jpg")).unwrap(), b"older");
    assert!(sess.join(".trash").join("c_head~2.jpg").exists(), "the collision is renamed, not overwritten");
    assert!(exp.join("loose.jpg").exists() && exp.join("a_head.jpg").exists(), "only orphans move");
}

/// The exports folder can live on another drive, and rename(2) cannot cross
/// a filesystem. /tmp is tmpfs here and target/ is not.
#[test]
fn a_move_across_filesystems_falls_back_to_a_copy() {
    let src = tmp("xdev").join("from.jpg");
    std::fs::write(&src, b"payload").unwrap();
    let dest_dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/xdev-test"));
    std::fs::create_dir_all(&dest_dir).unwrap();
    let dest = dest_dir.join("to.jpg");
    let _ = std::fs::remove_file(&dest);
    let cross = std::fs::rename(&src, dest_dir.join("probe")).err().and_then(|e| e.raw_os_error()) == Some(libc::EXDEV);
    if !cross {
        let _ = std::fs::remove_file(dest_dir.join("probe"));
        eprintln!("skipped: /tmp and target/ share a filesystem here");
        return;
    }
    store::atomic_replace(&src, &dest).expect("the copy fallback");
    assert_eq!(std::fs::read(&dest).unwrap(), b"payload");
    assert!(!src.exists(), "the source is removed after the copy");
    let _ = std::fs::remove_dir_all(&dest_dir);
}

/// A tag nothing can read is orientation 1, never an error that costs the frame.
#[test]
fn a_malformed_orientation_tag_is_upright() {
    assert_eq!(load::exif_orientation(b"not an image at all"), 1);
    assert_eq!(load::exif_orientation(b"\xff\xd8\xff\xe1\x00\x10Exif\x00\x00II*\x00\xff\xff"), 1);
}

/// A listing capped below the card's size keeps the NEWEST frames: newest
/// folder first, and a folder bigger than the room left gives up its oldest.
#[test]
fn a_capped_card_listing_keeps_the_newest() {
    let card = FakeCard::nikon();
    let (files, _, trunc) = card.walk(25);
    let names: Vec<String> = files.unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect();
    assert!(trunc);
    assert_eq!(names.len(), 25);
    assert_eq!(names[0], "DSC_10919.NEF", "newest first");
    assert_eq!(names[19], "DSC_10900.NEF", "the whole newest folder");
    assert_eq!(&names[20..], ["DSC_10819.NEF", "DSC_10818.NEF", "DSC_10817.NEF", "DSC_10816.NEF", "DSC_10815.NEF"],
               "the next folder gives up its oldest, not its newest");
}

/// Auto lands skin ON the colour target, not past it: a face of three real
/// skin tones on a cool background comes out at the target red-blue spread
/// and face brightness.
#[test]
fn auto_lands_skin_on_the_colour_target() {
    for skin in [[112u8, 92, 82], [105, 86, 76], [120, 98, 86]] {
        let mut im = img::Rgb::new(600, 800, [60, 70, 78]);
        // The face: an ellipse centred (300, 335), radii 130 x 185.
        for y in 0..800usize {
            for x in 0..600usize {
                let (dx, dy) = ((x as f64 - 300.0) / 130.0, (y as f64 - 335.0) / 185.0);
                if dx * dx + dy * dy <= 1.0 {
                    let i = (y * 600 + x) * 3;
                    im.px[i..i + 3].copy_from_slice(&skin);
                }
            }
        }
        let (adj, _, _) = analysis::auto_look(&im);
        let sm = img::thumbnail(&im, 420, 420);
        let mask = analysis::skin_mask_stats(&sm).expect("the face is skin");
        let st = analysis::stats_at(&grade::apply_edits(&sm, &Value::Object(adj), grade::Phase::All), &mask.px)
            .expect("stats");
        assert!(((st.r - st.b) - analysis::AUTO_SPREAD).abs() <= 2.0 && (st.y - analysis::AUTO_FACE_Y).abs() <= 4.0,
                "skin {skin:?}: red-blue {:+.1}, face {:.0}", st.r - st.b, st.y);
    }
}

/// The live view remembers the newest REAL frame's turn across days, and a
/// frame with no camera EXIF (a fixture, an import) never changes it.
#[test]
fn the_live_turn_is_remembered_from_real_frames_only() {
    setup();
    let real = json!({"lens": "NIKKOR", "orientation": "Rotate 270 CW", "turn": 270, "file": "x.jpg"});
    assert_eq!(advice::live_turn(Some(&real)), 270);
    assert_eq!(advice::live_turn(None), 270, "used on a day with no frame yet");
    let fixture = json!({"orientation": "Horizontal (normal)", "turn": 0});
    assert_eq!(advice::live_turn(Some(&fixture)), 0, "the fixture's own turn is reported");
    assert_eq!(advice::live_turn(None), 270, "but it is not REMEMBERED");
    let mut land = real.clone();
    land["orientation"] = json!("Horizontal (normal)");
    land["turn"] = json!(0);
    assert_eq!(advice::live_turn(Some(&land)), 0);
    assert_eq!(advice::live_turn(None), 0, "a later real landscape frame replaces it");
}

/// Nothing runs after SIGKILL, so a staging file survives a kill and the
/// startup sweep collects it, keyed on the writer's PID being DEAD: another
/// instance may be legitimately mid-write.
#[test]
fn the_startup_sweep_removes_only_dead_writers_staging() {
    let d = tmp("sweep");
    let probes = [(".alive.jpg.part-1-9", true), (".dead.jpg.part-999999-9", false),
                  (".odd.jpg.part-notanumber-9", true), ("ordinary-file.txt", true)];
    for (n, _) in probes {
        std::fs::write(d.join(n), b"").unwrap();
    }
    let removed = store::sweep_staging(std::slice::from_ref(&d));
    for (n, keep) in probes {
        assert_eq!(d.join(n).exists(), keep, "{n} should {}", if keep { "survive" } else { "be swept" });
    }
    assert_eq!(removed, vec![".dead.jpg.part-999999-9".to_string()]);
}

/// The camera's USB identity changes when it re-enumerates, which is how a
/// dead session is told apart from a live one.
#[test]
fn a_reenumerated_camera_is_a_new_device() {
    let root = tmp("usb-ident");
    let put = |name: &str, vendor: &str, devnum: &str| {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("idVendor"), format!("{vendor}\n")).unwrap();
        std::fs::write(root.join(name).join("devnum"), format!("{devnum}\n")).unwrap();
    };
    put("1-2", "046d", "3");
    assert_eq!(camera::nikon_device(&root), None, "a Logitech is not the camera");
    put("1-6", "04b0", "8");
    let first = camera::nikon_device(&root);
    assert_eq!(first.as_deref(), Some("1-6#8"));
    put("1-6", "04b0", "10");
    assert_ne!(camera::nikon_device(&root), first, "re-enumerated means a new device");
    std::fs::remove_dir_all(root.join("1-6")).unwrap();
    assert_eq!(camera::nikon_device(&root), None, "gone means gone");
}

/// Any real NEF on this machine (read-only): the test copies it first.
fn real_nef() -> Option<PathBuf> {
    // Real camera raws only. The tests' own scratch dirs (z8-rust-tests-*)
    // hold copies, including a JPEG renamed .nef, and once a reboot cleared
    // the genuine /tmp raws they were all that was left: the raw tests then
    // failed on a fake instead of skipping (2026-09-27).
    let mut roots = vec![app::home_dir().join("Pictures/z8-studio")];
    roots.extend(std::fs::read_dir("/tmp").ok()?.filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.file_name().map(|n| {
            let n = n.to_string_lossy();
            n.starts_with("z8-") && !n.starts_with("z8-rust-tests-")
        }).unwrap_or(false)));
    let genuine = |p: &PathBuf| {
        let big = std::fs::metadata(p).map(|m| m.len() > 5 << 20).unwrap_or(false);
        let mut head = [0u8; 4];
        let tiff = std::fs::File::open(p).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)).is_ok()
            && (&head == b"II*\0" || &head == b"MM\0*");
        big && tiff
    };
    let mut all: Vec<PathBuf> = roots.iter().flat_map(|r| {
        std::fs::read_dir(r).into_iter().flatten().filter_map(|e| e.ok()).flat_map(|e| {
            let p = e.path();
            if p.is_dir() { std::fs::read_dir(&p).into_iter().flatten().filter_map(|x| x.ok()).map(|x| x.path()).collect() }
            else { vec![p] }
        }).collect::<Vec<_>>()
    }).filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("nef")).unwrap_or(false) && genuine(p)).collect();
    all.sort();
    all.pop()
}

/// A raw nobody has developed tiles from the camera's own embedded JPEG:
/// full size, turned by the RAW's orientation tag, in well under a second.
#[test]
fn an_undeveloped_raw_tiles_from_its_embedded_jpeg() {
    setup();
    if !which("exiftool") {
        eprintln!("skipped: no exiftool");
        return;
    }
    let Some(src) = real_nef() else { eprintln!("skipped: no NEF on this machine"); return };
    let d = tmp("rawtile");
    let nef = d.join("frame.nef");
    std::fs::copy(&src, &nef).unwrap();
    assert!(load::is_raw(&nef) && !load::raw_is_developed(&nef));
    let t = Instant::now();
    let im = load::raw_embedded(&nef, None).expect("a Z8 NEF carries a full-size JPEG");
    let dt = t.elapsed();
    assert!(im.w > 4000 && im.w > im.h, "{}x{}", im.w, im.h);
    // The claim is "no develop", so assert that directly; the clock only
    // guards against a gross regression (a develop is ~4.7 s, and this ran
    // 0.29 s alone but once took over 2 s beside a parallel darktable run).
    assert!(!load::raw_is_developed(&nef), "reading the embedded JPEG developed the raw");
    assert!(dt < Duration::from_secs(10), "took {dt:?}");
    // Tagged as a turned body, the tile must come out portrait.
    let ok = std::process::Command::new("exiftool").args(["-q", "-overwrite_original", "-Orientation#=8"])
        .arg(&nef).status().unwrap().success();
    assert!(ok);
    let up = load::raw_embedded(&nef, Some(720)).unwrap();
    assert!(up.h > up.w, "portrait raw tiled as {}x{}", up.w, up.h);
    assert!(up.w < 2000, "the draft decode was used: {}x{}", up.w, up.h);
}

/// A provisional raw tile survives the cache pruner while its frame is live,
/// is replaced by the developed tile once the raw is developed, and goes when
/// the frame does.
#[test]
fn a_provisional_raw_tile_is_replaced_once_developed() {
    setup();
    if !which("exiftool") {
        eprintln!("skipped: no exiftool");
        return;
    }
    let Some(src) = real_nef() else { eprintln!("skipped: no NEF on this machine"); return };
    let sess = store::session_dir(None);
    std::fs::create_dir_all(&sess).unwrap();
    let nef = sess.join("provisional-test.nef");
    std::fs::copy(&src, &nef).unwrap();
    let row = store::shots().into_iter().find(|s| s["name"] == "provisional-test.nef").expect("listed");
    assert_eq!(row["provisional"], true);
    let tile = store::thumb_path("provisional-test.nef", 240).unwrap();
    assert!(tile.to_string_lossy().ends_with(".e.jpg"), "{tile:?}");
    store::shots();          // runs the pruner
    assert!(tile.exists(), "the pruner deleted a live provisional tile");
    // Developed: stand in the camera's own JPEG as the .dev.jpg, as a develop would write it.
    let emb = std::process::Command::new("exiftool").args(["-b", "-JpgFromRaw"]).arg(&nef).output().unwrap().stdout;
    std::fs::write(sess.join("provisional-test.dev.jpg"), emb).unwrap();
    let row = store::shots().into_iter().find(|s| s["name"] == "provisional-test.nef").unwrap();
    assert_eq!(row["provisional"], false);
    let fresh = store::thumb_path("provisional-test.nef", 240).unwrap();
    assert!(!fresh.to_string_lossy().ends_with(".e.jpg") && fresh.exists());
    assert!(!tile.exists(), "the provisional tile is replaced, not kept beside it");
    for f in ["provisional-test.nef", "provisional-test.dev.jpg"] {
        let _ = std::fs::remove_file(sess.join(f));
    }
    store::shots();
    assert!(!fresh.exists(), "a tile outlives its frame");
}

/// The batch runner: every item done exactly once, in parallel, results
/// handled on one thread, and a stop means no NEW item starts.
#[test]
fn a_batch_runs_in_parallel_and_stops_cleanly() {
    let items: Vec<usize> = (0..24).collect();
    let t = Instant::now();
    let mut seen = vec![];
    let finished = jobs::par_each(&items, 3, |&i| { std::thread::sleep(Duration::from_millis(50)); i * 2 },
                                  |&i, r| { assert_eq!(r, i * 2); seen.push(i); true });
    seen.sort();
    assert!(finished && seen == items, "{seen:?}");
    assert!(t.elapsed() < Duration::from_millis(24 * 50 * 2 / 3), "not parallel: {:?}", t.elapsed());
    // Stop after the fifth result: at most the ones already running finish.
    let started = std::sync::atomic::AtomicUsize::new(0);
    let mut handled = 0;
    let finished = jobs::par_each(&items, 3, |_| {
        started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30));
    }, |_, _| { handled += 1; handled < 5 });
    let n = started.load(std::sync::atomic::Ordering::SeqCst);
    assert!(!finished && handled == 5 && n <= 5 + 3, "{n} started after a stop at 5");
    assert!(jobs::batch_workers() >= 1 && jobs::batch_workers() <= 3);
}

/// Two requests for one raw run ONE develop, and the develop only appears
/// under its real name once it is complete: a reader polling for it the
/// whole time never catches a partial file, and no staging file is left.
#[test]
fn a_raw_develops_once_and_never_half_written() {
    setup();
    if !which("darktable-cli") {
        eprintln!("skipped: no darktable-cli");
        return;
    }
    let Some(src) = real_nef() else { eprintln!("skipped: no NEF on this machine"); return };
    let d = tmp("develop");
    let nef = d.join("once.nef");
    std::fs::copy(&src, &nef).unwrap();
    let cache = d.join("once.dev.jpg");
    let _ = std::fs::remove_file(&cache);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = {
        let (cache, done) = (cache.clone(), done.clone());
        std::thread::spawn(move || {
            let mut partial = 0;
            while !done.load(std::sync::atomic::Ordering::SeqCst) {
                if let Ok(b) = std::fs::read(&cache) {
                    if img::jpeg_decode(&b, 8).is_err() {
                        partial += 1;
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            partial
        })
    };
    let t = Instant::now();
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| load::develop_raw(&nef));
        let b = s.spawn(|| load::develop_raw(&nef));
        (a.join().unwrap(), b.join().unwrap())
    });
    let dt = t.elapsed();
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(watcher.join().unwrap(), 0, "a reader saw a half-written develop");
    assert_eq!(a.as_deref().ok(), Some(cache.as_path()));
    assert_eq!(b.as_deref().ok(), Some(cache.as_path()));
    let leftovers: Vec<String> = store::list_dir(&d).into_iter().filter(|n| n.contains(".part-")).collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    let t2 = Instant::now();
    load::develop_raw(&nef).unwrap();
    assert!(t2.elapsed() < Duration::from_millis(200), "a fresh develop is reused");
    eprintln!("two concurrent requests, one develop: {dt:?}");
}

/// Where Auto and Light spend their time on a real frame, once decoded.
/// `cargo test --release -- --ignored --nocapture where_analysis_time_goes`
#[test]
#[ignore]
fn where_analysis_time_goes() {
    setup();
    let Some(f) = real_frame() else { return };
    let t = Instant::now();
    let full = load::load_editable(&f, None).unwrap();
    eprintln!("decode full      {:>7.1} ms ({}x{})", t.elapsed().as_secs_f64() * 1e3, full.w, full.h);
    let t = Instant::now();
    let copy = full.clone();
    eprintln!("clone            {:>7.1} ms", t.elapsed().as_secs_f64() * 1e3);
    drop(copy);
    for n in [420, 640] {
        let t = Instant::now();
        let s = img::thumbnail(&full, n, n);
        eprintln!("thumbnail {n:>4}   {:>7.1} ms ({}x{})", t.elapsed().as_secs_f64() * 1e3, s.w, s.h);
    }
    let t = Instant::now();
    let _ = analysis::auto_look(&full);
    eprintln!("auto_look        {:>7.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let _ = analysis::lighting_report(&full);
    eprintln!("lighting_report  {:>7.1} ms", t.elapsed().as_secs_f64() * 1e3);
}

/// What each knob of the grade costs on a preview-sized real frame.
/// `cargo test --release -- --ignored --nocapture where_grade_time_goes`
#[test]
#[ignore]
fn where_grade_time_goes() {
    setup();
    let Some(f) = real_frame() else { return };
    let im = img::thumbnail(&load::load_editable(&f, Some(2200)).unwrap(), 1100, 1100);
    let time = |label: &str, adj: Value| {
        let _ = grade::apply_edits(&im, &adj, grade::Phase::All);
        let t = Instant::now();
        for _ in 0..10 {
            let _ = grade::apply_edits(&im, &adj, grade::Phase::All);
        }
        eprintln!("{label:12} {:>6.2} ms", t.elapsed().as_secs_f64() * 100.0);
    };
    eprintln!("frame {}x{}", im.w, im.h);
    time("none", json!({}));
    for (k, v) in [("exposure", 1.15), ("contrast", 1.1), ("saturation", 1.1), ("warmth", 0.2), ("tint", 0.1),
                   ("shadows", 0.2), ("highlights", -0.2), ("vignette", 0.2), ("sharpness", 1.3)] {
        time(k, json!({k: v}));
    }
    time("all nine", json!({"exposure": 1.15, "contrast": 1.1, "saturation": 1.1, "warmth": 0.2, "tint": 0.1,
                            "shadows": 0.2, "highlights": -0.2, "vignette": 0.2, "sharpness": 1.3}));
}

/// A detailed synthetic frame: gradients, fine texture and hard edges, so
/// every grade step (and sharpening especially) has something to change.
fn grade_fixture() -> img::Rgb {
    let (w, h) = (733usize, 1100usize);
    let mut im = img::Rgb::new(w, h, [0, 0, 0]);
    let mut seed = 12345u32;
    for y in 0..h {
        for x in 0..w {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let n = (seed >> 16) as i32 % 23 - 11;
            let i = (y * w + x) * 3;
            let edge = if (x / 37 + y / 41) % 2 == 0 { 40 } else { 0 };
            let c = |v: i32| v.clamp(0, 255) as u8;
            im.px[i] = c((x * 255 / w) as i32 + n + edge);
            im.px[i + 1] = c((y * 255 / h) as i32 + n);
            im.px[i + 2] = c(((x + y) * 255 / (w + h)) as i32 - n + edge / 2);
        }
    }
    im
}

fn digest(px: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(px))[..16].to_string()
}

const GRADE_CASES: &[&str] = &[
    r#"{}"#,
    r#"{"sharpness": 1.3}"#,
    r#"{"sharpness": 0.6}"#,
    r#"{"saturation": 1.25}"#,
    r#"{"saturation": 0.7, "warmth": 0.2}"#,
    r#"{"vignette": 0.25}"#,
    r#"{"exposure": 1.15, "contrast": 1.1, "saturation": 1.1, "warmth": 0.2, "tint": 0.1, "shadows": 0.2, "highlights": -0.2, "vignette": 0.2, "sharpness": 1.3}"#,
];

/// Print the grade's digests (run once before an optimisation to pin them).
#[test]
#[ignore]
fn print_grade_digests() {
    let im = grade_fixture();
    for c in GRADE_CASES {
        let out = grade::apply_edits(&im, &serde_json::from_str(c).unwrap(), grade::Phase::All);
        eprintln!("\"{}\",", digest(&out.px));
    }
}

/// The grade's output, pinned to the byte. Optimising the grade (lookup
/// tables, fused passes, threads) must not move a single pixel: these were
/// recorded from the straightforward implementation on 2026-09-26.
#[test]
fn the_grade_is_byte_identical_to_its_pinned_output() {
    const WANT: &[&str] = &["41ad083b4f5a2d80", "c7ce41b7faa62aad", "c01f535b3e43b886", "239d6f8c772bb749",
                            "dfd7d2684d9f8a4d", "ff9165d5628700ac", "4da2582689edd01a"];
    let im = grade_fixture();
    for (c, want) in GRADE_CASES.iter().zip(WANT) {
        let out = grade::apply_edits(&im, &serde_json::from_str(c).unwrap(), grade::Phase::All);
        assert_eq!(&digest(&out.px), want, "{c}");
        let owned = grade::apply_edits_owned(im.clone(), &serde_json::from_str(c).unwrap(), grade::Phase::All);
        assert_eq!(&digest(&owned.px), want, "owned path: {c}");
    }
}

/// A large synthetic frame for the resize paths (reduce, thumbnail, fit).
fn resize_fixture() -> img::Rgb {
    let (w, h) = (2403usize, 3601usize);      // odd sizes: partial boxes at the edges
    let mut im = img::Rgb::new(w, h, [0, 0, 0]);
    let mut seed = 777u32;
    for y in 0..h {
        for x in 0..w {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let n = (seed >> 16) as i32 % 31 - 15;
            let i = (y * w + x) * 3;
            let edge = if (x / 53 + y / 47) % 2 == 0 { 60 } else { 0 };
            let c = |v: i32| v.clamp(0, 255) as u8;
            im.px[i] = c((x * 255 / w) as i32 + n + edge);
            im.px[i + 1] = c((y * 255 / h) as i32 - n);
            im.px[i + 2] = c(((x * y) % 256) as i32 + edge / 3);
        }
    }
    im
}

fn resize_digests() -> Vec<String> {
    let im = resize_fixture();
    vec![
        digest(&img::reduce(&im, 3, 3).px),
        digest(&img::thumbnail(&im, 420, 420).px),
        digest(&img::thumbnail(&im, 1100, 1100).px),
        digest(&grade::fit_to(im.clone(), 800, 1000, "cover", "face").px),
        digest(&grade::fit_to(im.clone(), 1200, 800, "contain", "center").px),
    ]
}

#[test]
#[ignore]
fn print_resize_digests() {
    for d in resize_digests() {
        eprintln!("\"{d}\",");
    }
}

/// The resize paths, pinned to the byte (recorded 2026-09-27 before they
/// were threaded): reduce, two thumbnails, and fit cover/contain.
#[test]
fn the_resize_paths_are_byte_identical_to_their_pinned_output() {
    let want = ["3aec9fc90db46de4", "ef4139dffc0a1b20", "bf4a19d3b84c5e37", "4e4c59faefe5084d", "5cb4f89598406fff"];
    assert_eq!(resize_digests(), want);
}

/// Where a thumbnail of a 45 MP frame spends its time.
#[test]
#[ignore]
fn where_thumbnail_time_goes() {
    let (w, h) = (5504usize, 8256usize);
    let im = img::Rgb::new(w, h, [120, 90, 80]);
    let t = Instant::now();
    let r = img::reduce(&im, 13, 13);
    eprintln!("reduce 13x13     {:>7.1} ms ({}x{})", t.elapsed().as_secs_f64() * 1e3, r.w, r.h);
    let t = Instant::now();
    let s = img::thumbnail(&im, 420, 420);
    eprintln!("thumbnail 420    {:>7.1} ms ({}x{})", t.elapsed().as_secs_f64() * 1e3, s.w, s.h);
    let t = Instant::now();
    let f = grade::fit_to(im.clone(), 2400, 3000, "cover", "face");
    eprintln!("fit 2400x3000    {:>7.1} ms ({}x{})", t.elapsed().as_secs_f64() * 1e3, f.w, f.h);
}

/// Where one headshot export spends its time, step by step, on a real frame.
#[test]
#[ignore]
fn where_export_time_goes() {
    setup();
    let Some(f) = real_frame() else { return };
    let adj = json!({"exposure": 1.1, "contrast": 1.08, "saturation": 1.05, "warmth": 0.1,
                     "shadows": 0.1, "vignette": 0.1, "sharpness": 1.15});
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
    for round in 0..2 {
        let t0 = Instant::now();
        let t = Instant::now();
        let im = load::load_editable(&f, None).unwrap();
        let decode = ms(t);
        let t = Instant::now();
        let im = grade::apply_edits_owned(load::oriented(im), &adj, grade::Phase::Point);
        let point = ms(t);
        let t = Instant::now();
        let im = grade::fit_to(im, 2400, 3000, "cover", "face");
        let fit = ms(t);
        let t = Instant::now();
        let im = grade::apply_edits_owned(im, &adj, grade::Phase::Spatial);
        let spatial = ms(t);
        let t = Instant::now();
        let bytes = img::jpeg_encode(&im, &img::JpegOpts::q(95)).unwrap();
        let enc = ms(t);
        eprintln!("round {round}: decode {decode:.0} | point grade {point:.0} | fit {fit:.0} | spatial {spatial:.0} | \
                   encode {enc:.0} ({} KB) | total {:.0} ms", bytes.len() / 1024, ms(t0));
    }
}

/// The pieces of loading one real JPEG: read, decode, EXIF.
#[test]
#[ignore]
fn where_load_time_goes() {
    setup();
    let Some(f) = real_frame() else { return };
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
    for _ in 0..3 {
        let t = Instant::now();
        let data = std::fs::read(&f).unwrap();
        let read = ms(t);
        let t = Instant::now();
        let im = img::jpeg_decode(&data, 1).unwrap();
        let dec = ms(t);
        let t = Instant::now();
        let o = load::exif_orientation(&data);
        let exif = ms(t);
        eprintln!("read {read:.0} ms ({} MB) | decode {dec:.0} ms ({}x{}) | exif {exif:.1} ms (orientation {o})",
                  data.len() / 1_000_000, im.w, im.h);
    }
}

fn orientation_digests() -> Vec<String> {
    let im = grade_fixture();             // 733x1100, odd width
    (1..=8).map(|o| {
        let r = img::apply_orientation(im.clone(), o);
        format!("{}x{}:{}", r.w, r.h, digest(&r.px))
    }).collect()
}

#[test]
#[ignore]
fn print_orientation_digests() {
    for d in orientation_digests() {
        eprintln!("\"{d}\",");
    }
}

/// Every EXIF orientation, pinned to the byte (recorded 2026-09-27 from the
/// per-pixel implementation, before the tiled, threaded one).
#[test]
fn every_orientation_is_byte_identical_to_its_pinned_output() {
    let want = ["733x1100:41ad083b4f5a2d80", "733x1100:fe2657746ebde324", "733x1100:731774525aa4e215",
                "733x1100:4075701e1e7075d8", "1100x733:9d3506e8e133d112", "1100x733:0e95f48fb208b5b4",
                "1100x733:45c2b55f97aaa33d", "1100x733:2ccce1b25dc756b7"];
    assert_eq!(orientation_digests(), want);
}

/// What turning a 45 MP frame costs.
#[test]
#[ignore]
fn where_orientation_time_goes() {
    let im = img::Rgb::new(8256, 5504, [1, 2, 3]);
    for o in [3u32, 6, 8] {
        let c = im.clone();
        let t = Instant::now();
        let r = img::apply_orientation(c, o);
        eprintln!("orientation {o}: {:.1} ms ({}x{})", t.elapsed().as_secs_f64() * 1e3, r.w, r.h);
    }
}

/// Exports reuse the frame just inspected, but only PEEK: a frame nobody
/// inspected is never decoded into the shared slot by an export, and the
/// inspected frame stays put.
/// Tests that own the one shared full-size frame take turns with it.
static FULL_SLOT: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn an_export_only_peeks_at_the_inspected_frame() {
    setup();
    let _slot = FULL_SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let d = tmp("peek");
    let (a, b) = (d.join("a.jpg"), d.join("b.jpg"));
    let im = grade_fixture();
    std::fs::write(&a, img::jpeg_encode(&im, &img::JpegOpts::q(90)).unwrap()).unwrap();
    std::fs::write(&b, img::jpeg_encode(&im, &img::JpegOpts::q(80)).unwrap()).unwrap();
    assert!(z8::export::full_base_peek(&b).is_none(), "nothing decoded yet");
    let full = z8::export::full_base(&a).unwrap();
    let peeked = z8::export::full_base_peek(&a).expect("the inspected frame is shared");
    assert!(std::sync::Arc::ptr_eq(&full, &peeked), "the same decode, not a second one");
    assert!(z8::export::full_base_peek(&b).is_none(), "peeking another frame decodes nothing");
    assert!(z8::export::full_base_peek(&a).is_some(), "and does not evict the inspected one");
}

/// The float SMOOTH-then-blend that enhance_sharpness replaced, verbatim.
fn sharpen_reference(im: &img::Rgb, f: f32) -> img::Rgb {
    let (w, h) = (im.w, im.h);
    let mut smooth = im.clone();
    let k: Vec<f32> = [1., 1., 1., 1., 5., 1., 1., 1., 1.].iter().map(|v| v / 13.0).collect();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            for c in 0..3 {
                let mut s = 0.0f32;
                for dy in 0..3 {
                    for dx in 0..3 {
                        s += im.px[((y + dy - 1) * w + (x + dx - 1)) * 3 + c] as f32 * k[dy * 3 + dx];
                    }
                }
                smooth.px[(y * w + x) * 3 + c] = if s <= 0.0 { 0 } else if s >= 255.0 { 255 } else { (s + 0.5) as u8 };
            }
        }
    }
    let t = img::blend_table(f);          // blend1 for every (a, b)
    let px = smooth.px.iter().zip(&im.px).map(|(&a, &b)| t[(a as usize) << 8 | b as usize]).collect();
    img::Rgb { w, h, px }
}

/// The integer sharpen equals the float original on a real 45 MP frame.
#[test]
fn integer_sharpening_equals_the_float_original_on_a_real_frame() {
    setup();
    let Some(f) = real_frame() else { eprintln!("skipped: no real frame"); return };
    let im = load::load_editable(&f, None).unwrap();
    for s in [0.5f32, 1.15, 1.6] {
        assert!(img::enhance_sharpness(&im, s).px == sharpen_reference(&im, s).px, "strength {s}");
    }
}

/// The library's resize done whole, as resize_box did before it was split
/// into strips.
fn resize_whole(im: &img::Rgb, nw: usize, nh: usize, bx: (f64, f64, f64, f64)) -> Vec<u8> {
    use fast_image_resize as fr;
    let src = fr::images::ImageRef::new(im.w as u32, im.h as u32, &im.px, fr::PixelType::U8x3).unwrap();
    let mut dst = fr::images::Image::new(nw as u32, nh as u32, fr::PixelType::U8x3);
    let opts = fr::ResizeOptions::new()
        .resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3))
        .crop(bx.0, bx.1, bx.2 - bx.0, bx.3 - bx.1);
    fr::Resizer::new().resize(&src, &mut dst, &opts).unwrap();
    dst.into_vec()
}

/// resize_box equals the library's whole-image resize on a real 45 MP frame,
/// at the sizes the export presets use. Tried 2026-09-27: splitting the
/// resize into four strips on four cores matched the synthetic fixture but
/// NOT this real frame (float positions at the strip seams), so it was
/// dropped. Any future attempt has to pass this first.
#[test]
fn strip_resizing_equals_the_whole_resize_on_a_real_frame() {
    setup();
    let Some(f) = real_frame() else { eprintln!("skipped: no real frame"); return };
    let im = load::load_editable(&f, None).unwrap();
    let (w, h) = (im.w as f64, im.h as f64);
    for (nw, nh, bx) in [(2400usize, 3000usize, (0.0, (h - w * 1.25) * 0.35, w, (h - w * 1.25) * 0.35 + w * 1.25)),
                         (1200, 1600, (w * 0.05, h * 0.02, w * 0.95, h * 0.02 + w * 0.9 * 4.0 / 3.0)),
                         (1920, 1080, (0.0, h * 0.3, w, h * 0.3 + w * 9.0 / 16.0)),
                         (3000, 2000, (0.0, 0.0, w, h))] {
        let got = img::resize_box(&im, nw, nh, img::Filter::Lanczos, Some(bx));
        assert!(got.px == resize_whole(&im, nw, nh, bx), "{nw}x{nh} from {bx:?}");
    }
}

/// A strip tile from the camera's own embedded preview: upright like the
/// frame, and within a few levels of a tile made from the full decode.
#[test]
fn a_strip_tile_can_come_from_the_embedded_preview() {
    setup();
    let Some(f) = real_frame() else { eprintln!("skipped: no real frame"); return };
    let t = Instant::now();
    let Some((pre, mw, mh)) = load::mpf_preview(&f, 240, 240) else { eprintln!("skipped: no MPF preview"); return };
    let dt = t.elapsed();
    let full = load::load_editable(&f, Some(720)).unwrap();
    assert_eq!(pre.w > pre.h, full.w > full.h, "the preview is turned like the frame");
    assert_eq!(mw > mh, full.w > full.h, "and the frame size it reports is upright");
    let b = img::thumbnail(&full, 240, 720);
    let (tw, th) = img::thumbnail_size(mw, mh, 240, 720).unwrap();
    let a = img::resize(&pre, tw, th, img::Filter::Lanczos);
    assert_eq!((a.w, a.h), (b.w, b.h), "same tile size");
    let diffs: Vec<i32> = a.px.iter().zip(&b.px).map(|(x, y)| (*x as i32 - *y as i32).abs()).collect();
    let mean = diffs.iter().sum::<i32>() as f64 / diffs.len() as f64;
    let p99 = { let mut d = diffs.clone(); d.sort(); d[d.len() * 99 / 100] };
    eprintln!("preview {}x{} in {:.1} ms; tile {}x{}: mean diff {mean:.2}, p99 {p99}, max {}",
              pre.w, pre.h, dt.as_secs_f64() * 1e3, a.w, a.h, diffs.iter().max().unwrap());
    assert!(mean < 3.0 && p99 <= 16, "mean {mean:.2}, p99 {p99}");
}

/// What a fresh 136 MB buffer costs to fill versus one already touched.
#[test]
#[ignore]
fn where_page_fault_time_goes() {
    let n = 8256 * 5504 * 3;
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
    for _ in 0..3 {
        let t = Instant::now();
        let mut v = vec![0u8; n];
        for i in (0..n).step_by(4096) { v[i] = 1; }
        let fresh = ms(t);
        let t = Instant::now();
        for i in (0..n).step_by(4096) { v[i] = 2; }
        let warm = ms(t);
        let t = Instant::now();
        let c = v.clone();
        let clone = ms(t);
        eprintln!("fresh 136 MB touched: {fresh:.1} ms | same buffer again: {warm:.2} ms | clone: {clone:.1} ms ({})", c[0]);
    }
}

/// The same fresh 136 MB buffer with transparent huge pages requested.
#[test]
#[ignore]
fn where_huge_page_time_goes() {
    let n = 8256 * 5504 * 3;
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
    for _ in 0..3 {
        let t = Instant::now();
        let mut v: Vec<u8> = Vec::with_capacity(n);
        unsafe {
            let p = v.as_mut_ptr() as usize;
            let a = (p + (1 << 21) - 1) & !((1 << 21) - 1);
            libc::madvise(a as *mut libc::c_void, (p + n).saturating_sub(a) & !((1 << 21) - 1), libc::MADV_HUGEPAGE);
            v.set_len(n);
        }
        for i in (0..n).step_by(4096) { v[i] = 1; }
        eprintln!("fresh 136 MB with MADV_HUGEPAGE touched: {:.1} ms", ms(t));
    }
}

/// The webcam keeper's per-frame cost on a live-view-sized real frame.
#[test]
#[ignore]
fn where_keeper_frame_time_goes() {
    setup();
    let Some(f) = real_frame() else { return };
    let big = load::load_editable(&f, Some(2200)).unwrap();
    let lv = img::resize(&big, 1024, 680, img::Filter::Lanczos);
    let jpeg = img::jpeg_encode(&lv, &img::JpegOpts::q(90)).unwrap();
    let g = z8::frame::Grader::new(&z8::frame::Look { exposure: 1.1, contrast: 1.05, saturation: 1.05,
                                                      warmth: 0.1, vignette: 0.1, sharpness: 1.2 });
    let mut fr = z8::frame::Frame::default();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
    let (mut dec, mut proc_) = (vec![], vec![]);
    for _ in 0..200 {
        let t = Instant::now();
        let im = img::jpeg_decode(&jpeg, 1).unwrap();
        dec.push(ms(t));
        let t = Instant::now();
        pool.install(|| fr.process_par(&g, &im.px, im.w, im.h, true));
        proc_.push(ms(t));
    }
    let med = |v: &mut Vec<f64>| { v.sort_by(|a, b| a.total_cmp(b)); v[v.len() / 2] };
    eprintln!("live frame {} KB: decode {:.2} ms, grade+sharpen+I420 {:.2} ms (medians of 200)",
              jpeg.len() / 1024, med(&mut dec), med(&mut proc_));
}

/// The Export tab asks for the selected frame in full before Export is
/// pressed. The route must fill the frame an export peeks at (not the small
/// preview base), refuse a bad name, and the export must be byte-identical to
/// a cold one of the same photo.
#[test]
fn the_export_tab_prefetch_fills_the_frame_an_export_reuses() {
    setup();
    let _slot = FULL_SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let sess = store::session_dir(None);
    std::fs::create_dir_all(&sess).unwrap();
    let bytes = img::jpeg_encode(&grade_fixture(), &img::JpegOpts::q(90)).unwrap();
    let (warm, cold) = (sess.join("prefetch-warm.jpg"), sess.join("prefetch-cold.jpg"));
    std::fs::write(&warm, &bytes).unwrap();
    std::fs::write(&cold, &bytes).unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let get = |uri: &str| {
        let req = axum::http::Request::builder().uri(uri).body(axum::body::Body::empty()).unwrap();
        rt.block_on(z8::http::handle(req)).status().as_u16()
    };
    assert_eq!(get("/api/prefetch?full=1&name=..%2Fx.jpg"), 400, "a path is not a frame");
    assert_eq!(get("/api/prefetch?full=1&name=prefetch-warm.jpg"), 202);
    let t = Instant::now();
    while z8::export::full_base_peek(&warm).is_none() {
        assert!(t.elapsed() < Duration::from_secs(20), "full=1 never decoded the frame an export peeks at");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(z8::export::full_base_peek(&cold).is_none(), "the twin is exported cold");
    let pre = json!({"id": "t", "format": "JPEG", "w": 400, "h": 500, "fit": "cover", "quality": 90});
    let adj = json!({"exposure": 1.1, "contrast": 1.05, "sharpness": 1.15, "vignette": 0.1});
    let out = std::env::temp_dir().join(format!("z8-rust-tests-{}", std::process::id())).join("prefetch-out");
    let (ok_w, w) = z8::export::render("prefetch-warm.jpg", &pre, &adj, None, None, Some(&out));
    let (ok_c, c) = z8::export::render("prefetch-cold.jpg", &pre, &adj, None, None, Some(&out));
    assert!(ok_w && ok_c, "{w} {c}");
    let read = |v: &Value| std::fs::read(v["path"].as_str().unwrap()).unwrap();
    assert!(read(&w) == read(&c), "an export from the prefetched frame differs from a cold one");
}

/// The page reaches a phone over the tailnet. Its files go gzipped when the
/// browser takes gzip, an unchanged file is a 304 with no body, a changed one
/// is never answered 304 with the old tag, and every load still revalidates.
#[test]
fn the_page_files_are_compressed_and_revalidated() {
    setup();
    let web = app::app().web.clone();
    std::fs::create_dir_all(web.join("dist")).unwrap();
    let js: Vec<u8> = (0..40_000).flat_map(|i: u32| format!("let v{i} = {};\n", i % 97).into_bytes()).collect();
    std::fs::write(web.join("dist/app.js"), &js).unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let get = |hdr: &[(&str, &str)]| {
        let mut b = axum::http::Request::builder().uri("/app.js");
        for (k, v) in hdr {
            b = b.header(*k, *v);
        }
        let r = rt.block_on(z8::http::handle(b.body(axum::body::Body::empty()).unwrap()));
        let (parts, body) = r.into_parts();
        let bytes = rt.block_on(axum::body::to_bytes(body, 1 << 26)).unwrap().to_vec();
        let h = |k: &str| parts.headers.get(k).map(|v| v.to_str().unwrap().to_string());
        (parts.status.as_u16(), h("content-encoding"), h("etag"), h("cache-control"), bytes)
    };
    let (code, enc, _, cache, plain) = get(&[]);
    assert_eq!((code, enc.as_deref(), cache.as_deref()), (200, None, Some("no-cache")));
    assert!(plain == js, "the plain body is the file");
    let (code, enc, tag, _, gz) = get(&[("accept-encoding", "br, gzip")]);
    assert_eq!((code, enc.as_deref()), (200, Some("gzip")));
    let mut unzipped = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&gz[..]), &mut unzipped).unwrap();
    assert!(unzipped == js && gz.len() * 3 < js.len(), "gzip is the same bytes, smaller: {} of {}", gz.len(), js.len());
    let tag = tag.expect("an etag");
    let (code, _, _, _, body) = get(&[("accept-encoding", "gzip"), ("if-none-match", &tag)]);
    assert_eq!((code, body.len()), (304, 0), "an unchanged file is not sent again");
    assert_eq!(get(&[("if-none-match", &tag)]).0, 200, "the gzip tag does not validate the plain file");
    assert_eq!(get(&[("accept-encoding", "gzip;q=0")]).1, None, "q=0 refuses gzip");
    std::thread::sleep(Duration::from_millis(20));
    let mut changed = js.clone();
    changed.extend_from_slice(b"// deployed\n");
    std::fs::write(web.join("dist/app.js"), &changed).unwrap();
    let (code, _, _, _, body) = get(&[("if-none-match", &tag)]);
    assert!(code == 200 && body == changed, "a deploy is served at once, never a 304 on the old tag");
}

/// A raw's provisional tile reads its embedded JPEG from the file itself, not
/// through exiftool (Perl, 0.15 s a frame). The bytes must be exactly what
/// `exiftool -b -JpgFromRaw` prints, on every real NEF this machine has.
#[test]
fn the_raw_preview_reader_matches_exiftool_byte_for_byte() {
    setup();
    if !which("exiftool") {
        eprintln!("skipped: no exiftool");
        return;
    }
    let mut nefs: Vec<PathBuf> = std::fs::read_dir("/tmp").unwrap().filter_map(|e| e.ok()).map(|e| e.path())
        .chain(std::fs::read_dir(app::home_dir().join("Pictures/z8-studio")).into_iter().flatten()
               .filter_map(|e| e.ok()).map(|e| e.path()))
        .filter(|d| d.is_dir())
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().filter_map(|e| e.ok()).map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("nef")).unwrap_or(false)
                && std::fs::metadata(p).map(|m| m.len() > 1 << 20).unwrap_or(false))
        .collect();
    nefs.sort();
    nefs.dedup();
    nefs.truncate(6);
    if nefs.is_empty() {
        eprintln!("skipped: no NEF on this machine");
        return;
    }
    for nef in &nefs {
        let want = std::process::Command::new("exiftool").args(["-b", "-JpgFromRaw"]).arg(nef).output().unwrap().stdout;
        let t = Instant::now();
        let got = load::tiff_jpg_from_raw(nef).unwrap_or_default();
        eprintln!("{}: native read {:.2} ms", nef.display(), t.elapsed().as_secs_f64() * 1e3);
        assert!(!want.is_empty() && got == want, "{nef:?}: {} bytes, exiftool {}", got.len(), want.len());
        // Orientation: the exif crate's answer, which reads the whole file.
        let t = Instant::now();
        let slow = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(std::fs::File::open(nef).unwrap()))
            .ok().and_then(|ex| ex.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)))
            .unwrap_or(1);
        let slow_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        assert_eq!(load::tiff_orientation(nef), Some(slow), "{nef:?} orientation");
        eprintln!("  orientation {slow}: exif crate {slow_ms:.1} ms, native {:.2} ms", t.elapsed().as_secs_f64() * 1e3);
        // NEFCompression: exiftool's number (13/14 are HE, which cannot develop).
        let want: u32 = String::from_utf8_lossy(&std::process::Command::new("exiftool")
            .args(["-n", "-s3", "-NEFCompression"]).arg(nef).output().unwrap().stdout).trim().parse().unwrap();
        assert_eq!(load::nef_compression(nef), Some(want), "{nef:?} NEFCompression");
    }
    // A portrait frame: the same raw with IFD0's Orientation rewritten to 6.
    let turned = tmp("turned-raw").join("portrait.nef");
    std::fs::copy(&nefs[0], &turned).unwrap();
    let w = std::process::Command::new("exiftool").args(["-q", "-n", "-Orientation=6", "-overwrite_original"])
        .arg(&turned).status().unwrap();
    assert!(w.success());
    assert_eq!(load::tiff_orientation(&turned), Some(6), "a portrait raw reads as turned");
    let d = tmp("not-a-raw");
    let jpg = d.join("x.nef");
    std::fs::write(&jpg, img::jpeg_encode(&grade_fixture(), &img::JpegOpts::q(80)).unwrap()).unwrap();
    assert!(load::tiff_jpg_from_raw(&jpg).is_none(), "a JPEG named .nef is not a TIFF");
    assert!(load::tiff_orientation(&jpg).is_none() && load::nef_compression(&jpg).is_none());
    std::fs::write(&jpg, b"II*\0\xff\xff\xff\x7f").unwrap();
    assert!(load::tiff_jpg_from_raw(&jpg).is_none(), "an IFD past the end is refused, not read");
    assert!(load::tiff_orientation(&jpg).is_none() && load::nef_compression(&jpg).is_none());
    eprintln!("{} NEFs byte-identical to exiftool", nefs.len());
}

/// The PHOTO/VIDEO lever is read from libgphoto2's data log (the raw opcode
/// action throws the reply away). The parser takes only the fixed hex
/// columns, never the ASCII column, and only a whole data container that
/// answers GetDevicePropValue.
#[test]
fn the_lever_reply_is_read_from_the_hex_columns_only() {
    use z8::gp::{data_reply, dump_bytes};
    // Real layout from the Z8 (a DevicePropDesc reply for 0xD1A6).
    let desc = "Read    24 = 0x18 out of 1024 bytes from port: (hexdump of 24 bytes)\n\
                0000  18 00 00 00 02 00 14 10-05 00 00 00 a6 d1 02 00  ................\n\
                0010  00 00 01 02 02 00 00 01-                         ........        \n";
    assert_eq!(dump_bytes(desc).len(), 24);
    // The REQUEST is logged too: a command container for the same operation,
    // whose byte 12 is the property code's low byte (0xa6), not a value.
    let request = "Wrote   16 = 0x10 bytes to port: (hexdump of 16 bytes)\n\
                   0000  10 00 00 00 01 00 15 10-07 00 00 00 a6 d1 00 00  ................\n".to_string();
    let value = |v: u8| format!("Read    13 = 0xd out of 1024 bytes from port: (hexdump of 13 bytes)\n\
                                 0000  0d 00 00 00 02 00 15 10-07 00 00 00 {v:02x}           .............\n");
    assert_eq!(data_reply(&[desc.to_string(), request.clone(), value(1)], 0x1015), Some(1), "video");
    assert_eq!(data_reply(&[request.clone(), value(0)], 0x1015), Some(0), "photo");
    assert_eq!(data_reply(&[request, desc.to_string()], 0x1015), None, "no data container for the op");
    // A reply over two lines whose ASCII column happens to read as hex ("12").
    let two = "0000  12 00 00 00 02 00 15 10-07 00 00 00 01 00 00 00  ................\n\
               0010  31 32                                            12\n".to_string();
    assert_eq!(data_reply(&[two], 0x1015), Some(1), "the ASCII column is not bytes");
    let cut = "0000  20 00 00 00 02 00 15 10-07 00 00 00 01           .............\n".to_string();
    assert_eq!(data_reply(&[cut], 0x1015), None, "a container cut short of its length is refused");
}

/// The status poll must not wait on exiftool (Perl, ~0.2 s) for a new frame's
/// settings: it gets nothing at once, the reading lands in the background,
/// and it is exactly the blocking reading.
#[test]
fn the_status_poll_never_waits_on_exiftool() {
    setup();
    if !which("exiftool") {
        eprintln!("skipped: no exiftool");
        return;
    }
    let p = tmp("actual-now").join("fresh.jpg");
    std::fs::write(&p, img::jpeg_encode(&grade_fixture(), &img::JpegOpts::q(85)).unwrap()).unwrap();
    let t = Instant::now();
    assert!(advice::actual_settings_now(Some(&p)).is_none(), "an unread frame answers nothing yet");
    assert!(advice::actual_settings_now(Some(&p)).is_none());
    assert!(t.elapsed() < Duration::from_millis(50), "the poll waited {:?}", t.elapsed());
    let got = loop {
        if let Some(v) = advice::actual_settings_now(Some(&p)) {
            break v;
        }
        assert!(t.elapsed() < Duration::from_secs(15), "the background read never landed");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(Some(got), advice::actual_settings(Some(&p)), "the same reading the blocking call gives");
}

/// JSON replies go gzipped to a browser that takes gzip and are the same
/// bytes once decoded; small replies and clients without gzip get plain.
#[test]
fn json_replies_are_gzipped_only_when_asked_and_decode_to_the_same() {
    setup();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let get = |uri: &str, enc: Option<&str>| {
        let mut b = axum::http::Request::builder().uri(uri);
        if let Some(e) = enc {
            b = b.header("accept-encoding", e);
        }
        let r = rt.block_on(z8::http::handle(b.body(axum::body::Body::empty()).unwrap()));
        let (parts, body) = r.into_parts();
        let bytes = rt.block_on(axum::body::to_bytes(body, 1 << 26)).unwrap().to_vec();
        (parts.headers.get("content-encoding").map(|v| v.to_str().unwrap().to_string()), bytes)
    };
    let (enc, plain) = get("/api/presets", None);
    assert!(enc.is_none() && plain.len() >= 1400, "presets are big enough to compress: {}", plain.len());
    let (enc, gz) = get("/api/presets", Some("gzip, br"));
    assert_eq!(enc.as_deref(), Some("gzip"));
    let mut back = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&gz[..]), &mut back).unwrap();
    assert!(back == plain && gz.len() < plain.len(), "the same JSON, smaller");
    assert_eq!(get("/api/presets", Some("gzip;q=0")).0, None, "q=0 refuses gzip");
    let (enc, small) = get("/api/lever", Some("gzip"));
    assert!(enc.is_none() && small.len() < 1400, "a small reply is left alone");
}

/// The tile and derived-file prunes run when the folder changes, not on
/// every poll, and still remove exactly what the per-poll prune removed:
/// a deleted frame's tiles and develop, and a HEIF conversion once a real
/// .jpg of that frame arrives even while a raw stays the frame's best file.
#[test]
fn the_prunes_follow_every_change_in_the_session_folder() {
    setup();
    let day = "2031-02-03";
    let d = store::session_dir(Some(day));
    std::fs::create_dir_all(d.join(".cache")).unwrap();
    let jpg = img::jpeg_encode(&grade_fixture(), &img::JpegOpts::q(70)).unwrap();
    for n in ["a.jpg", "b.jpg", "x.nef", "x.heic"] {
        std::fs::write(d.join(n), &jpg).unwrap();
    }
    std::fs::write(d.join(".cache/b.jpg.240.1.jpg"), b"tile").unwrap();
    std::fs::write(d.join("b.dev.jpg"), b"dev").unwrap();
    std::fs::write(d.join("x.heic.conv.jpg"), b"conv").unwrap();
    let names = |v: Vec<Value>| { let mut n: Vec<String> = v.iter().map(|s| s["name"].as_str().unwrap().to_string()).collect(); n.sort(); n };
    assert_eq!(names(store::shots_in(Some(day))), ["a.jpg", "b.jpg", "x.nef"]);
    assert!(d.join(".cache/b.jpg.240.1.jpg").exists() && d.join("x.heic.conv.jpg").exists());
    std::fs::remove_file(d.join("b.jpg")).unwrap();
    store::shots_in(Some(day));
    assert!(!d.join(".cache/b.jpg.240.1.jpg").exists(), "a deleted frame's tile goes");
    assert!(!d.join("b.dev.jpg").exists(), "and its develop");
    std::fs::write(d.join("x.jpg"), &jpg).unwrap();
    assert_eq!(names(store::shots_in(Some(day))), ["a.jpg", "x.nef"], "the raw is still x's frame");
    assert!(!d.join("x.heic.conv.jpg").exists(), "a real .jpg retires the HEIF conversion");
    let one = store::shots_in(Some(day));
    let a = one.iter().find(|s| s["name"] == "a.jpg").unwrap();
    let m = std::fs::metadata(d.join("a.jpg")).unwrap();
    assert_eq!(a["size"].as_u64(), Some(m.len()));
    assert_eq!(a["at"].as_f64(), app::mtime(&d.join("a.jpg")), "at is the file's mtime, as before");
}

/// A setting the camera took must not be reported as declined because the
/// camera writes it in its own form (+0.7 is stored as 0.666), and one it
/// really refused must still be.
#[test]
fn a_setting_the_camera_took_is_not_called_declined() {
    use z8::camera::setting_matches as m;
    assert!(m("exposurecomp", "0.7", "0.666"), "the report of 2026-09-27");
    assert!(m("exposurecomp", "-0.3", "-0.333") && m("exposurecomp", "1", "1"));
    assert!(!m("exposurecomp", "0.5", "0.666"), "a different step is a refusal");
    assert!(!m("exposurecomp", "0.7", "0"), "the camera staying put is a refusal");
    assert!(m("shutter", "1/50", "0.0200s") && !m("shutter", "1/50", "0.0167s"), "1/50 is not 1/60");
    assert!(m("aperture", "f/5.6", "f/5.6") && !m("aperture", "f/5.6", "f/4.5"));
    assert!(m("iso", "1250", "1250") && !m("iso", "1250", "1000"));
    assert!(m("whitebalance", "Automatic", "Automatic") && !m("whitebalance", "Daylight", "Automatic"));
}

/// Every status poll lists the day's audio takes with their length. Reading
/// each whole file for a 44-byte header cost 2 % of a core with nobody
/// shooting (63 takes, 3.4 MB each, reread on every poll).
#[test]
fn a_take_length_comes_from_its_header_not_the_whole_file() {
    let d = std::env::temp_dir().join(format!("z8-wavhead-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let wav = |name: &str, chunks: &[(&[u8; 4], Vec<u8>)], data_size: u32, pad_to: u64| {
        let mut b = b"RIFF\0\0\0\0WAVE".to_vec();
        for (id, body) in chunks {
            b.extend_from_slice(*id);
            b.extend_from_slice(&(body.len() as u32).to_le_bytes());
            b.extend_from_slice(body);
            if body.len() & 1 == 1 {
                b.push(0);
            }
        }
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_size.to_le_bytes());
        let p = d.join(name);
        std::fs::write(&p, &b).unwrap();
        if pad_to > 0 {
            std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_len(pad_to).unwrap();
        }
        p
    };
    let fmt = |rate: u32, block: u16| {
        let mut f = vec![1, 0, 2, 0];
        f.extend_from_slice(&rate.to_le_bytes());
        f.extend_from_slice(&(rate * block as u32).to_le_bytes());
        f.extend_from_slice(&block.to_le_bytes());
        f.extend_from_slice(&[32, 0]);
        f
    };
    // An odd-sized LIST chunk before fmt: the pad byte must be skipped.
    let big = wav("big.wav", &[(b"LIST", vec![7; 5]), (b"fmt ", fmt(48000, 8))], 48000 * 8 * 2500, 1 << 30);
    let t = Instant::now();
    let secs = store::wav_seconds(&big);
    let took = t.elapsed();
    assert_eq!(secs, Some(2500.0), "a 1 GB take reads 2500 s from its header");
    assert!(took < Duration::from_millis(50), "the header alone is read: {took:?}");
    // A take still being written reports from its header, file cut short.
    let partial = wav("partial.wav", &[(b"fmt ", fmt(44100, 4))], 44100 * 4 * 3, 0);
    assert_eq!(store::wav_seconds(&partial), Some(3.0));
    let nofmt = wav("nofmt.wav", &[], 1000, 0);
    assert_eq!(store::wav_seconds(&nofmt), None, "no fmt chunk, no length");
    std::fs::write(d.join("short.wav"), b"RIFF\0\0").unwrap();
    assert_eq!(store::wav_seconds(&d.join("short.wav")), None);
    std::fs::write(d.join("not.wav"), b"RIFX\0\0\0\0WAVEdata\0\0\0\0").unwrap();
    assert_eq!(store::wav_seconds(&d.join("not.wav")), None);
    std::fs::remove_dir_all(&d).unwrap();
}

/// A flip of the PHOTO/VIDEO lever during live view makes the Z8 refuse every frame until live view
/// restarts; the app now restarts it by itself, but only on the camera's generic error, after three
/// refusals in a row, never while recording, and at most once every 15 s.
#[test]
fn live_view_restarts_itself_after_a_lever_flip_but_never_while_recording() {
    let e = "[-1] Unspecified error";
    assert!(camera::should_relatch(3, e, false, 60.0), "three refusals of the generic error restart live view");
    assert!(!camera::should_relatch(2, e, false, 60.0), "two are not enough");
    assert!(!camera::should_relatch(9, e, true, 60.0), "NEVER while recording");
    assert!(!camera::should_relatch(9, e, false, 10.0), "at most once every 15 s");
    assert!(!camera::should_relatch(9, "[-7] I/O problem", false, 60.0), "other errors are left alone");
}
