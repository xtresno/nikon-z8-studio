//! The HTTP surface: the same 54 routes, bodies and status codes as the
//! Python server, so the page and the 1000-check suite drive it unchanged.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, Method, Request, Response, StatusCode};
use serde_json::{json, Map, Value};

use crate::advice::{self, actual_settings, control_confirms, control_notices, live_turn, next_step_cached,
                    next_step_changed};
use crate::analysis;
use crate::app::{app, get_str, now, num, truthy};
use crate::audio;
use crate::camera::{apply_cam_preset, cam, camera_present, capture_cam_preset, Busy};
use crate::export::{self, base_cache_clear, preview_base, render};
use crate::feedctl::{self, feed_status, feed_wanted, maybe_autoconnect, set_mode, webcam_look, webcam_running};
use crate::grade::{apply_crop, apply_edits, straighten, Phase, ANCHORS};
use crate::img;
use crate::jobs;
use crate::load::{self, load_editable, oriented};
use crate::store::{self, ends_with_any, export_dir, fname, py_str, session_dir, shots, splitext,
                   today, today_dir, AUDIO_EXT, VIDEO_EXT};
use crate::tools::{last_line, need, run_timeout};

// ------------------------------------------------------------------ plumbing
pub enum Out {
    Full { code: u16, ctype: String, body: Vec<u8>, extra: Vec<(String, String)> },
    Stream { code: u16, headers: Vec<(String, String)>, rx: tokio::sync::mpsc::Receiver<Bytes> },
}

fn out(code: u16, ctype: &str, body: Vec<u8>) -> Out {
    Out::Full { code, ctype: ctype.into(), body, extra: vec![] }
}

fn js(code: u16, v: Value) -> Out {
    out(code, "application/json", serde_json::to_vec(&v).unwrap_or_default())
}

fn ok(v: Value) -> Out {
    js(200, v)
}

fn text(code: u16, s: &str) -> Out {
    out(code, "text/plain", s.as_bytes().to_vec())
}

fn jpeg(b: Vec<u8>) -> Out {
    out(200, "image/jpeg", b)
}

fn busy(b: Busy) -> Out {
    js(503, json!({"ok": false, "error": b.0, "busy": true}))
}

fn bad(name: &str) -> bool {
    name.is_empty() || name.contains('/') || name.contains("..")
}

fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// parse_qs: first value per key, blanks dropped.
fn query(q: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for (k, v) in form_urlencoded::parse(q.as_bytes()) {
        if !v.is_empty() {
            m.entry(k.into_owned()).or_insert_with(|| v.into_owned());
        }
    }
    m
}

/// A request this app did not make, from a page it did not serve: Host must
/// name this server, and a POST carrying an Origin must be this app's.
fn foreign(method: &Method, h: &HeaderMap) -> Option<String> {
    request_origin_error(method, h, app().port, app().remote_origin.as_deref())
}

fn request_origin_error(method: &Method, h: &HeaderMap, p: u16, remote: Option<&str>) -> Option<String> {
    let local = [format!("127.0.0.1:{p}"), format!("localhost:{p}"), format!("[::1]:{p}")];
    let remote_host = remote.and_then(|v| v.strip_prefix("https://"));
    let host = h.get("host").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    if !host.is_empty() && !local.contains(&host) && remote_host != Some(host.as_str()) {
        return Some(format!("host {host} is not this server"));
    }
    let origin = h.get("origin").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    if method == Method::POST && !origin.is_empty()
        && !local.iter().any(|o| origin == format!("http://{o}")) && remote != Some(origin.as_str()) {
        return Some(format!("origin {origin} is not this app"));
    }
    None
}

const BODY_TYPES: &[(&str, &str)] = &[("name", "str"), ("id", "str"), ("date", "str"), ("preset", "dict"),
                                      ("adjust", "dict"), ("crop", "dict"), ("names", "list")];


fn type_name(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "str",
        Value::Object(_) => "dict",
        Value::Array(_) => "list",
        Value::Bool(_) => "bool",
        Value::Number(n) => if n.is_f64() { "float" } else { "int" },
        Value::Null => "NoneType",
    }
}

/// The first field whose type no handler can use, or None. Rejected, never
/// coerced: coercion invents an intent the caller did not express.
pub fn body_type_error(b: &Value) -> Option<String> {
    for (k, want) in BODY_TYPES {
        match b.get(*k) {
            None | Some(Value::Null) => continue,
            Some(v) if type_name(v) == *want => continue,
            Some(v) => return Some(format!("'{k}' must be {want}, got {}", type_name(v))),
        }
    }
    None
}

pub async fn handle(req: Request<Body>) -> Response<Body> {
    let (parts, body) = req.into_parts();
    if let Some(why) = foreign(&parts.method, &parts.headers) {
        return to_response(js(403, json!({"ok": false, "error": format!("refused: {why}")})));
    }
    let bytes = axum::body::to_bytes(body, 256 * 1024 * 1024).await.unwrap_or_default();
    let method = parts.method.clone();
    let path = parts.uri.path().to_string();
    let q = parts.uri.query().unwrap_or("").to_string();
    let headers = parts.headers.clone();
    let accept = parts.headers.clone();
    let res = tokio::task::spawn_blocking(move || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // The recorder's video chunks are raw bytes, not JSON.
            if method == Method::POST && path == "/api/rec-chunk" {
                let q = query(&q);
                let s = |k: &str| q.get(k).cloned().unwrap_or_default();
                let Ok(off) = s("off").parse::<u64>() else { return js(400, json!({"ok": false, "error": "bad offset"})) };
                let (code, v) = crate::recorder::chunk(&s("id"), &s("ext"), off, &bytes);
                return js(code, v);
            }
            if method == Method::POST {
                let b: Value = if bytes.is_empty() { json!({}) } else {
                    match serde_json::from_slice::<Value>(&bytes) {
                        Ok(v @ Value::Object(_)) => v,
                        _ => json!({}),
                    }
                };
                if let Some(e) = body_type_error(&b) {
                    return js(400, json!({"ok": false, "error": e}));
                }
                post(&path, &b)
            } else {
                get(&path, &query(&q), &headers)
            }
        }))
    }).await;
    let o = match res {
        Ok(Ok(o)) => o,
        Ok(Err(p)) => {
            let msg = p.downcast_ref::<String>().cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into());
            eprintln!("request panicked: {msg}");
            js(500, json!({"ok": false, "error": format!("Panic: {msg}")}))
        }
        Err(e) => js(500, json!({"ok": false, "error": e.to_string()})),
    };
    to_response(gzip_json(o, &accept))
}

fn to_response(o: Out) -> Response<Body> {
    match o {
        Out::Full { code, ctype, body, extra } => {
            let mut r = Response::new(Body::from(body));
            *r.status_mut() = StatusCode::from_u16(code).unwrap_or(StatusCode::OK);
            let h = r.headers_mut();
            h.insert("content-type", HeaderValue::from_str(&ctype).unwrap_or(HeaderValue::from_static("text/plain")));
            h.insert("cache-control", HeaderValue::from_static("no-store"));
            for (k, v) in extra {
                if let (Ok(k), Ok(v)) = (axum::http::HeaderName::try_from(k), HeaderValue::from_str(&v)) {
                    h.insert(k, v);
                }
            }
            r
        }
        Out::Stream { code, headers, rx } => {
            let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
            let stream = futures_util::StreamExt::map(stream, Ok::<Bytes, std::io::Error>);
            let mut r = Response::new(Body::from_stream(stream));
            *r.status_mut() = StatusCode::from_u16(code).unwrap_or(StatusCode::OK);
            for (k, v) in headers {
                if let (Ok(k), Ok(v)) = (axum::http::HeaderName::try_from(k), HeaderValue::from_str(&v)) {
                    r.headers_mut().insert(k, v);
                }
            }
            r
        }
    }
}

fn file_bytes(p: &Path) -> Option<Vec<u8>> {
    std::fs::read(p).ok()
}

/// The browser takes gzip (and has not refused it with q=0).
fn accepts_gzip(h: &HeaderMap) -> bool {
    h.get_all("accept-encoding").iter().filter_map(|x| x.to_str().ok())
        .flat_map(|x| x.split(',')).any(|t| {
            let mut it = t.split(';');
            it.next().map(|e| e.trim().eq_ignore_ascii_case("gzip")).unwrap_or(false)
                && !it.any(|p| p.replace(' ', "") == "q=0")
        })
}

/// JSON replies go gzipped to a browser that takes it: /api/state carries
/// the whole frame list every 3 s, 106 KB at 1000 frames and 8 KB gzipped,
/// which is what a phone on the tailnet pays. Small replies are left alone.
fn gzip_json(o: Out, h: &HeaderMap) -> Out {
    match o {
        Out::Full { code, ctype, body, mut extra }
            if ctype.starts_with("application/json") && body.len() >= 1400 && accepts_gzip(h)
                && !extra.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-encoding")) => {
            let mut enc = flate2::write::GzEncoder::new(Vec::with_capacity(body.len() / 8), flate2::Compression::fast());
            match std::io::Write::write_all(&mut enc, &body).and_then(|_| enc.finish()) {
                Ok(gz) => {
                    extra.push(("content-encoding".into(), "gzip".into()));
                    extra.push(("vary".into(), "Accept-Encoding".into()));
                    Out::Full { code, ctype, body: gz, extra }
                }
                Err(_) => Out::Full { code, ctype, body, extra },
            }
        }
        o => o,
    }
}

/// One page file as served: its validator, its bytes and their gzip.
struct Asset {
    stamp: (PathBuf, u64, std::time::SystemTime),
    etag: String,
    raw: Vec<u8>,
    gz: Vec<u8>,
}

static ASSETS: parking_lot::Mutex<Vec<std::sync::Arc<Asset>>> = parking_lot::Mutex::new(Vec::new());

/// The page's own files. Every load still asks (no-cache, so a deploy shows
/// at once), but an unchanged file is answered 304 with no body, and a
/// changed one goes gzipped when the browser takes it: a phone over the
/// tailnet fetched 223 KB uncompressed on every load.
fn page_asset(file: &Path, ctype: &str, h: &HeaderMap) -> Out {
    let Ok(meta) = std::fs::metadata(file) else { return text(404, "no") };
    let stamp = (file.to_path_buf(), meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH));
    let cached = ASSETS.lock().iter().find(|a| a.stamp == stamp).cloned();
    let asset = match cached {
        Some(a) => a,
        None => {
            let Some(raw) = file_bytes(file) else { return text(404, "no") };
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(&raw);
            let etag: String = digest[..12].iter().map(|b| format!("{b:02x}")).collect();
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            let gz = std::io::Write::write_all(&mut enc, &raw).and_then(|_| enc.finish()).unwrap_or_default();
            let a = std::sync::Arc::new(Asset { stamp, etag, raw, gz });
            let mut all = ASSETS.lock();
            all.retain(|x| x.stamp.0 != a.stamp.0);
            all.push(a.clone());
            a
        }
    };
    let gz = !asset.gz.is_empty() && accepts_gzip(h);
    let etag = format!("\"{}{}\"", asset.etag, if gz { "-gz" } else { "" });
    let mut extra = vec![("cache-control".into(), "no-cache".into()), ("etag".into(), etag.clone()),
                         ("vary".into(), "Accept-Encoding".into())];
    let fresh = h.get_all("if-none-match").iter().filter_map(|x| x.to_str().ok())
        .flat_map(|x| x.split(',')).any(|t| t.trim().trim_start_matches("W/") == etag);
    if fresh {
        return Out::Full { code: 304, ctype: ctype.into(), body: vec![], extra };
    }
    let body = if gz {
        extra.push(("content-encoding".into(), "gzip".into()));
        asset.gz.clone()
    } else {
        asset.raw.clone()
    };
    Out::Full { code: 200, ctype: ctype.into(), body, extra }
}

// ------------------------------------------------------------------ GET
fn get(p: &str, q: &HashMap<String, String>, h: &HeaderMap) -> Out {
    let a = app();
    match p {
        "/" => return page_asset(&a.web.join("index.html"), "text/html; charset=utf-8", h),
        "/app.js" => return page_asset(&a.web.join("dist/app.js"), "application/javascript; charset=utf-8", h),
        "/app.css" => return page_asset(&a.web.join("app.css"), "text/css; charset=utf-8", h),
        "/record" => return page_asset(&a.web.join("record.html"), "text/html; charset=utf-8", h),
        "/record.js" => return page_asset(&a.web.join("dist/record.js"), "application/javascript; charset=utf-8", h),
        "/prompter" => return page_asset(&a.web.join("prompter.html"), "text/html; charset=utf-8", h),
        "/videos" => return page_asset(&a.web.join("videos.html"), "text/html; charset=utf-8", h),
        "/prompter.js" => return page_asset(&a.web.join("dist/prompter.js"), "application/javascript; charset=utf-8", h),
        "/manifest.webmanifest" | "/apple-touch-icon.png" | "/icon-192.png" | "/icon-512.png" => {
            let ctype = if p.ends_with(".webmanifest") { "application/manifest+json" } else { "image/png" };
            return match file_bytes(&a.web.join(&p[1..])) {
                Some(b) => out(200, ctype, b),
                None => text(404, "no"),
            };
        },
        "/preview.mjpg" => return stream_preview(),
        "/api/state" => return ok(state()),
        "/api/card" => {
            let (files, err, truncated) = match cam().card_list(400) {
                Ok(r) => r,
                Err(b) => return busy(b),
            };
            let Some(mut files) = files.filter(|_| true) else {
                return js(409, json!({"ok": false, "error": err}));
            };
            if err.is_some() && files.is_empty() {
                return js(409, json!({"ok": false, "error": err}));
            }
            let here: std::collections::HashSet<String> = store::list_dir(&today_dir()).into_iter().collect();
            for f in files.iter_mut() {
                let n = f["name"].as_str().unwrap_or("").to_string();
                f["have"] = json!(here.contains(&n));
            }
            return ok(json!({"ok": true, "files": files, "truncated": truncated, "warning": err}));
        }
        "/api/all-settings" => return match cam().all_settings() {
            Ok(v) => ok(v),
            Err(b) => busy(b),
        },
        "/api/settings" => {
            let req = cam().settings();
            let act = actual_settings(None);
            let mode = if webcam_running() { "webcam" } else { "tether" };
            let notices = control_notices(&req, act.as_ref(), mode);
            let confirms = control_confirms(act.as_ref(), mode);
            let mut o = req.clone();
            o.insert("_actual".into(), act.unwrap_or(Value::Null));
            o.insert("_notices".into(), json!(notices));
            o.insert("_confirms".into(), json!(confirms));
            return ok(Value::Object(o));
        }
        "/api/job" => {
            let id = q.get("id").cloned().unwrap_or_default();
            return match jobs::get(&id) {
                Some(j) => {
                    let mut m = Map::new();
                    m.insert("ok".into(), json!(true));
                    m.extend(j);
                    ok(Value::Object(m))
                }
                None => {
                    let re = regex::Regex::new(r"^[a-z]+-(\d{13})").expect("re");
                    let born = re.captures(&id).and_then(|c| c[1].parse::<f64>().ok()).map(|v| v / 1000.0);
                    if born.map(|b| b < a.started_at).unwrap_or(false) {
                        return js(410, json!({"ok": false, "restarted": true,
                            "error": "the server restarted while this was running, so it stopped partway. \
                                      Anything already exported is on disk."}));
                    }
                    js(404, json!({"ok": false, "error": "no such job"}))
                }
            };
        }
        "/api/audio" => return ok(audio::audio_status()),
        "/api/rec-info" => return ok(crate::recorder::info()),
        "/api/rec-script" => return ok(crate::recorder::script()),
        "/api/videos" => return ok(crate::recorder::videos()),
        "/api/videos/img" => {
            // Generated thumbnails and headshots only, nothing else on disk.
            let p = q.get("path").cloned().unwrap_or_default();
            return match crate::recorder::kit_image(&p) {
                Some((bytes, ctype)) => out(200, ctype, bytes),
                None => text(404, "no"),
            };
        }
        "/api/rec-check" => return ok(crate::recorder::check_result()),
        "/api/studio-ready" => return ok(crate::recorder::studio_ready()),
        "/api/presets" => return match store::presets() {
            Ok(mut v) => {
                let anchors: Map<String, Value> = ANCHORS.iter()
                    .map(|(k, (x, y))| (k.to_string(), json!([x, y]))).collect();
                v["_anchors"] = Value::Object(anchors);
                ok(v)
            }
            Err(e) => js(500, json!({"ok": false, "error": e})),
        },
        "/api/preview" | "/api/histogram" => return preview_like(p, q),
        "/api/prefetch" => return prefetch(q),
        // The PHOTO/VIDEO lever, read only; null when not connected or busy.
        "/api/lever" => return ok(json!({"lever": if cam().is_open() { cam().lever() } else { None }})),
        "/api/loupe" => return loupe(q),
        "/api/poster" => {
            let name = q.get("name").cloned().unwrap_or_default();
            if bad(&name) || !ends_with_any(&name, VIDEO_EXT) {
                return text(400, "bad name");
            }
            let src = session_dir(None).join(&name);
            if !src.exists() {
                return text(404, "no");
            }
            return match store::video_poster(&src).and_then(|p| std::fs::read(p).map_err(|e| e.to_string())) {
                Ok(b) => jpeg(b),
                Err(e) => text(415, &e),
            };
        }
        _ => {}
    }
    if let Some(rest) = p.strip_prefix("/img/") {
        let name = decode(rest);
        if name.contains('/') || name.contains("..") {
            return text(400, "bad name");
        }
        let path = session_dir(None).join(&name);
        if !path.is_file() {
            return text(404, "no");
        }
        let w: i64 = match q.get("w") {
            None => 0,
            Some(s) => s.parse().unwrap_or(240),
        };
        let w = if w != 0 { w.clamp(32, 2400) as usize } else { 0 };
        if w > 0 {
            return match store::thumb_path(&name, w).and_then(|p| std::fs::read(p).map_err(|e| e.to_string())) {
                Ok(b) => jpeg(b),
                Err(e) => text(415, &e),
            };
        }
        return match load_editable(&path, None).map(oriented)
            .and_then(|im| img::jpeg_encode(&im, &img::JpegOpts::q(88))) {
            Ok(b) => jpeg(b),
            Err(e) => text(415, &e),
        };
    }
    if let Some(rest) = p.strip_prefix("/take/") {
        let name = decode(rest);
        if bad(&name) || !ends_with_any(&name, AUDIO_EXT) {
            return text(400, "bad name");
        }
        let fp = session_dir(None).join(&name);
        if !fp.is_file() {
            return text(404, "no");
        }
        return serve_range(&fp, h);
    }
    if let Some(rest) = p.strip_prefix("/clip/") {
        let name = decode(rest);
        if bad(&name) || !ends_with_any(&name, VIDEO_EXT) {
            return text(400, "bad name");
        }
        let mut fp = session_dir(None).join(&name);
        if !fp.is_file() {
            let alt = export_dir(None).join(&name);
            if alt.is_file() {
                fp = alt;
            } else {
                return text(404, "no");
            }
        }
        return serve_range(&fp, h);
    }
    if let Some(rest) = p.strip_prefix("/export/") {
        let name = decode(rest);
        if name.chars().any(|c| (c as u32) < 32) || name.contains('"') {
            return text(400, "bad name");
        }
        let mut path = export_dir(None).join(&name);
        if !path.is_file() {
            path = session_dir(None).join("exports").join(&name);
        }
        if name.contains('/') || name.is_empty() || !path.is_file() {
            return text(404, "no");
        }
        let ctype = match splitext(&name).1.to_lowercase().as_str() {
            ".png" => "image/png", ".webp" => "image/webp", _ => "image/jpeg",
        };
        return match file_bytes(&path) {
            Some(b) => Out::Full { code: 200, ctype: ctype.into(), body: b,
                                   extra: vec![("content-disposition".into(), format!("attachment; filename=\"{name}\""))] },
            None => text(404, "no"),
        };
    }
    text(404, "no")
}

fn state() -> Value {
    let present = camera_present();
    maybe_autoconnect(present);
    let c = cam();
    let (manual_off, recording, warning, error, rotation, mirror) = {
        let a = c.a.lock();
        (a.manual_off, a.recording, a.warning.clone(), a.error.clone(), a.rotation, a.mirror)
    };
    let ss = shots();
    let actual = advice::newest_frame(&ss).and_then(|p| advice::actual_settings_now(Some(&p)));
    let sdir = session_dir(None);
    let trash = sdir.join(".trash");
    let day = store::active_day();
    let feed = feed_status();
    let cfg = store::load_config();
    let mut cam_presets: Vec<String> = store::load_cam_presets().keys().cloned().collect();
    cam_presets.sort();
    let mut transient: Vec<String> = app().transient.lock().keys().cloned().collect();
    transient.sort();
    let wr = webcam_running();
    json!({
        "present": present,
        "manualOff": manual_off,
        "connected": c.is_open() && !wr,
        "recording": recording && c.is_open(),
        "warning": warning,
        "audioRec": audio::audio_rec_status(),
        "error": error,
        "rotation": rotation,
        "mirror": mirror,
        "liveTurn": live_turn(actual.as_ref()),
        "camPresets": cam_presets,
        "transientConfig": transient,
        "orphanExports": store::export_stems(None).iter().filter(|r| r["state"] == "orphan").count(),
        "converting": load::convert_backlog(),
        "webcam": wr || (feed_wanted() && feed.is_some()),
        "feed": feed,
        "dir": sdir,
        "session": day,
        "exportDir": export_dir(None),
        "freeMB": store::disk_free_mb(&sdir).unwrap_or(0.0).round_ties_even() as i64,
        "config": cfg,
        "today": today(),
        "browsingPast": day != today(),
        "sessions": store::sessions(),
        "shots": ss,
        "clips": store::clips(),
        "takes": store::takes(),
        "edits": store::load_edits(None),
        "sourceId": app().source_id,
        "startedAt": app().started_at,
        "nextStep": next_step_cached(),
        "toolsMissing": crate::tools::missing_tools(),
        "trashMB": crate::app::round(store::dir_size_mb(&trash), 1),
        "trash": store::list_dir(&trash).iter().filter(|f| trash.join(f).is_file()).count(),
        "engine": "rust",
    })
}

fn parse_json_param(q: &HashMap<String, String>, k: &str, default: Value) -> Option<Value> {
    match q.get(k) {
        None => Some(default),
        Some(s) => serde_json::from_str(s).ok(),
    }
}

fn preview_like(p: &str, q: &HashMap<String, String>) -> Out {
    let name = q.get("name").cloned().unwrap_or_default();
    if bad(&name) {
        return text(400, "bad name");
    }
    let src = session_dir(None).join(&name);
    if !src.exists() {
        return text(404, "no");
    }
    let (adj, crop) = match (parse_json_param(q, "adjust", json!({})), parse_json_param(q, "crop", Value::Null)) {
        (Some(a), Some(c)) => (a, c),
        _ => (json!({}), Value::Null),
    };
    let adj = if adj.is_object() { adj } else { json!({}) };
    let original = p == "/api/preview" && q.get("original").map(|s| s.as_str()) == Some("1");
    // The page asks for the stage, the crop pane and the histogram of the
    // SAME picture at once after every slider move: one render serves all.
    let key = format!("{}|{:?}|{}|{}|{}|{}", src.display(), crate::app::mtime(&src).map(f64::to_bits),
                      adj, crop, original, cam().mirror());
    let r = shared_render(key, || preview_base(&src).map(|base| {
        let im = straighten((*base).clone(), num(adj.get("straighten"), 0.0));
        let im = apply_crop(im, Some(&crop));
        if original { im } else { crate::grade::apply_edits_owned(im, &adj, Phase::All) }
    }));
    let im = match r {
        Ok(im) => im,
        Err(e) => return text(415, &e),
    };
    if p == "/api/preview" {
        return match img::jpeg_encode(&im, &img::JpegOpts::q(86)) {
            Ok(b) => jpeg(b),
            Err(e) => text(415, &e),
        };
    }
    let [r, g, b] = img::histograms(&im);
    let total = (im.w * im.h).max(1) as f64;
    let blown = r[255].max(g[255]).max(b[255]) as f64;
    let crushed = r[0].max(g[0]).max(b[0]) as f64;
    ok(json!({"r": r.to_vec(), "g": g.to_vec(), "b": b.to_vec(),
              "clipHigh": crate::app::round(100.0 * blown / total, 3),
              "clipLow": crate::app::round(100.0 * crushed / total, 3)}))
}

type Rendered = Result<std::sync::Arc<img::Rgb>, String>;
/// The last few preview renders, each computed ONCE however many requests
/// ask for it at the same moment: the first renders, the rest wait for it.
static RENDERS: parking_lot::Mutex<Vec<(String, std::sync::Arc<std::sync::OnceLock<Rendered>>)>> =
    parking_lot::Mutex::new(Vec::new());

fn shared_render(key: String, f: impl FnOnce() -> Result<img::Rgb, String>) -> Rendered {
    let cell = {
        let mut r = RENDERS.lock();
        match r.iter().find(|(k, _)| *k == key) {
            Some((_, c)) => c.clone(),
            None => {
                if r.len() >= 6 {
                    r.remove(0);
                }
                let c = std::sync::Arc::new(std::sync::OnceLock::new());
                r.push((key, c.clone()));
                c
            }
        }
    };
    cell.get_or_init(|| f().map(std::sync::Arc::new)).clone()
}

/// Decode a frame's preview base in the background so the NEXT photo the
/// user arrows to renders from memory: arrowing measured 218 ms a photo,
/// nearly all of it this decode. Answers at once; at most two run, so
/// holding an arrow key cannot queue up work.
fn prefetch(q: &HashMap<String, String>) -> Out {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static RUNNING: AtomicUsize = AtomicUsize::new(0);
    let name = q.get("name").cloned().unwrap_or_default();
    if bad(&name) {
        return text(400, "bad name");
    }
    let src = session_dir(None).join(&name);
    if !src.is_file() {
        return text(404, "no");
    }
    // full=1: the whole frame, for an export about to happen (the Export tab
    // is open on it). One at a time: it is 136 MB and ~270 ms of decode.
    let full = q.get("full").map(|v| v == "1").unwrap_or(false);
    static FULL_RUNNING: AtomicUsize = AtomicUsize::new(0);
    let (counter, cap) = if full { (&FULL_RUNNING, 1) } else { (&RUNNING, 2) };
    if counter.fetch_add(1, Ordering::SeqCst) >= cap {
        counter.fetch_sub(1, Ordering::SeqCst);
        return text(429, "busy");
    }
    std::thread::spawn(move || {
        if full {
            let _ = crate::export::full_base(&src);
        } else {
            let _ = preview_base(&src);
        }
        counter.fetch_sub(1, Ordering::SeqCst);
    });
    text(202, "ok")
}

fn loupe(q: &HashMap<String, String>) -> Out {
    let name = q.get("name").cloned().unwrap_or_default();
    if bad(&name) {
        return text(400, "bad name");
    }
    let src = session_dir(None).join(&name);
    if !src.exists() {
        return text(404, "no");
    }
    let f = |k: &str, d: &str| q.get(k).cloned().unwrap_or_else(|| d.into());
    let (Ok(fx), Ok(fy)) = (f("x", "0.5").parse::<f64>(), f("y", "0.5").parse::<f64>()) else {
        return text(400, "bad params");
    };
    let Ok(size) = f("size", "520").parse::<i64>() else { return text(400, "bad params") };
    let Some(adj) = parse_json_param(q, "adjust", json!({})) else { return text(400, "bad params") };
    let adj = if adj.is_object() { adj } else { json!({}) };
    let (fx, fy, size) = (fx.clamp(0.0, 1.0), fy.clamp(0.0, 1.0), size.clamp(120, 1200));
    let r = crate::export::full_base(&src).map(|base| {
        // Straightening turns the whole frame; without it the patch is cut
        // straight out of the shared copy.
        let deg = num(adj.get("straighten"), 0.0);
        let turned;
        let im: &img::Rgb = if deg == 0.0 { &base } else { turned = straighten((*base).clone(), deg); &turned };
        let (w, h) = (im.w as i64, im.h as i64);
        let (cx, cy) = ((fx * w as f64) as i64, (fy * h as f64) as i64);
        let half = size / 2;
        let left = (cx - half).min(w - size).max(0);
        let top = (cy - half).min(h - size).max(0);
        let patch = img::crop(im, left, top, left + size.min(w), top + size.min(h));
        apply_edits(&patch, &adj, Phase::All)
    });
    match r.and_then(|im| img::jpeg_encode(&im, &img::JpegOpts::q(92))) {
        Ok(b) => jpeg(b),
        Err(e) => text(415, &e),
    }
}

/// Byte ranges, so a long take scrubs instead of buffering. Streamed from a
/// reader thread in 256 KB chunks.
fn serve_range(fp: &Path, h: &HeaderMap) -> Out {
    let size = std::fs::metadata(fp).map(|m| m.len()).unwrap_or(0);
    let ctype = match splitext(&fname(fp)).1.to_lowercase().as_str() {
        ".mov" => "video/quicktime", ".mp4" | ".m4v" => "video/mp4", ".avi" => "video/x-msvideo",
        ".wav" => "audio/wav", ".flac" => "audio/flac", _ => "application/octet-stream",
    };
    let (mut start, mut end, mut partial) = (0u64, size.saturating_sub(1), false);
    if let Some(rng) = h.get("range").and_then(|v| v.to_str().ok()).and_then(|s| s.strip_prefix("bytes=")) {
        if size > 0 {
            let first = rng.split(',').next().unwrap_or("").trim();
            let (a, b) = first.split_once('-').unwrap_or((first, ""));
            let parsed: Option<(u64, u64)> = if a.is_empty() {
                b.parse::<u64>().ok().map(|n| (size.saturating_sub(n), size - 1))
            } else {
                a.parse::<u64>().ok().and_then(|s| if b.is_empty() { Some((s, size - 1)) }
                                                    else { b.parse::<u64>().ok().map(|e| (s, e)) })
            };
            if let Some((s, e)) = parsed {
                if s >= size {
                    return Out::Full { code: 416, ctype: ctype.into(), body: vec![],
                                       extra: vec![("content-range".into(), format!("bytes */{size}"))] };
                }
                start = s;
                end = e.min(size - 1).max(start);
                partial = true;
            }
        }
    }
    let length = if size == 0 { 0 } else { end - start + 1 };
    let mut headers = vec![("content-type".to_string(), ctype.to_string()),
                           ("accept-ranges".into(), "bytes".into()),
                           ("content-length".into(), length.to_string()),
                           ("cache-control".into(), "no-store".into())];
    if partial {
        headers.push(("content-range".into(), format!("bytes {start}-{end}/{size}")));
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(4);
    let fp = fp.to_path_buf();
    std::thread::spawn(move || {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut f) = std::fs::File::open(&fp) else { return };
        if f.seek(SeekFrom::Start(start)).is_err() {
            return;
        }
        let mut left = length;
        let mut buf = vec![0u8; 262144];
        while left > 0 {
            let n = (left as usize).min(buf.len());
            let got = match f.read(&mut buf[..n]) {
                Ok(0) | Err(_) => break,
                Ok(g) => g,
            };
            if tx.blocking_send(Bytes::copy_from_slice(&buf[..got])).is_err() {
                break;
            }
            left -= got as u64;
        }
    });
    Out::Stream { code: if partial { 206 } else { 200 }, headers, rx }
}

/// Live view as MJPEG. Ends only when the CAMERA is gone: a browser shows a
/// finished MJPEG exactly like a live one.
fn stream_preview() -> Out {
    const BOUNDARY: &str = "z8studioframe";
    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(2);
    std::thread::spawn(move || {
        let mut idle = 0;
        // The Z8 makes 30 live frames a second but hands back its latest one
        // as fast as it is asked: measured 268 sent a second, each new frame
        // about nine times, 13 MB/s for the browser to decode. That starved
        // the page, and the live view froze whenever the strip loaded tiles.
        // Send a frame only when it is new, and ask again a moment later.
        let mut last: Vec<u8> = Vec::new();
        loop {
            match cam().preview_jpeg() {
                Some(jpg) if jpg == last => {
                    if tx.is_closed() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(8));
                }
                None => {
                    idle = if cam().is_open() { 0 } else { idle + 1 };
                    if idle > 40 || tx.is_closed() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                Some(jpg) => {
                    idle = 0;
                    let mut part = format!("--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                                           jpg.len()).into_bytes();
                    part.extend_from_slice(&jpg);
                    last = jpg;
                    part.extend_from_slice(b"\r\n");
                    if tx.blocking_send(Bytes::from(part)).is_err() {
                        return;
                    }
                }
            }
        }
    });
    Out::Stream { code: 200, rx, headers: vec![
        ("content-type".into(), format!("multipart/x-mixed-replace; boundary={BOUNDARY}")),
        ("cache-control".into(), "no-store".into())] }
}

// ------------------------------------------------------------------ POST
fn names_ok(v: &Value) -> Option<Vec<String>> {
    let a = v.as_array()?;
    a.iter().map(|x| x.as_str().filter(|s| !s.contains('/') && !s.contains("..")).map(|s| s.to_string()))
        .collect()
}

fn post(p: &str, b: &Value) -> Out {
    match p {
        "/api/videos/load" => return ok(crate::recorder::load_video_script(b)),
        "/api/videos/run" => return ok(crate::recorder::run_kit_job(b)),
        "/api/videos/pick" => return ok(crate::recorder::pick_kit(b)),
        "/api/connect" => {
            if webcam_running() || feed_wanted() {
                return js(409, json!({"ok": false, "busy": true,
                    "error": "the webcam feed has the camera; press Tether to take it back"}));
            }
            cam().a.lock().manual_off = false;
            let okc = cam().connect();
            ok(json!({"ok": okc, "error": cam().a.lock().error}))
        }
        "/api/disconnect" => {
            cam().disconnect();
            ok(json!({"ok": true}))
        }
        "/api/orient" => orient(b),
        "/api/af" => match cam().autofocus() {
            Ok((o, m)) => ok(json!({"ok": o, "msg": m})),
            Err(e) => busy(e),
        },
        "/api/focus" => {
            // Whole lens steps: libgphoto2 truncates a fraction, so 0.4 would quietly drive 1 step.
            let steps = b.get("steps").and_then(|v| v.as_f64()).map(f64::round).unwrap_or(0.0);
            if steps == 0.0 || steps.abs() > 32767.0 {
                return js(400, json!({"ok": false, "error": "steps: a whole number from -32767 to 32767, not 0"}));
            }
            match cam().focus_drive(steps as i32) {
                Ok((o, m)) => ok(json!({"ok": o, "msg": m})),
                Err(e) => busy(e),
            }
        }
        "/api/af-area" => {
            // libgphoto2 hands x and y to the camera as unsigned, so a negative or huge one would wrap.
            let px = |k: &str| b.get(k).and_then(|v| v.as_f64()).map(f64::round)
                .filter(|v| (0.0..=32767.0).contains(v)).map(|v| v as i32);
            let (Some(x), Some(y)) = (px("x"), px("y")) else {
                return js(400, json!({"ok": false, "error": "x and y: live view pixels, 0 to 32767"}));
            };
            match cam().af_area(x, y) {
                Ok((o, m)) => ok(json!({"ok": o, "msg": m})),
                Err(e) => busy(e),
            }
        }
        "/api/shoot" => {
            let (fok, why) = store::disk_ok(&today_dir());
            if !fok {
                return js(507, json!({"ok": false, "name": null, "error": why}));
            }
            let focus = b.get("focus").filter(|v| !v.is_null()).map(|v| truthy(Some(v)));
            match cam().shoot(&today_dir(), focus) {
                Ok((o, res)) => {
                    let a = cam().a.lock();
                    let warn: Vec<String> = [a.focus_note.clone(), a.warning.clone()].into_iter().flatten().collect();
                    next_step_changed();
                    ok(json!({"ok": o, "name": if o { json!(res) } else { Value::Null },
                              "error": if o { Value::Null } else { json!(res) },
                              "warning": if warn.is_empty() { Value::Null } else { json!(warn.join(" ")) }}))
                }
                Err(e) => busy(e),
            }
        }
        "/api/setting" => {
            let key = b.get("key").map(py_str).unwrap_or_default();
            let val = b.get("value").map(py_str).unwrap_or_default();
            match cam().set_setting(&key, &val) {
                Ok((o, m)) => ok(json!({"ok": o, "msg": m})),
                Err(e) => busy(e),
            }
        }
        // A tap on the PHOTO/VIDEO lamp: restart live view and read again.
        "/api/lever" => match cam().recheck_lever() {
            Ok(v) => ok(json!({"lever": v})),
            Err(e) => busy(e),
        },
        "/api/movie" => match cam().movie(truthy(b.get("on"))) {
            Ok((o, m)) => ok(json!({"ok": o, "msg": m})),
            Err(e) => busy(e),
        },
        "/api/sync-audio" => sync_audio(b),
        "/api/audio-record" => {
            let (code, v) = audio::record(truthy(b.get("on")));
            js(code, v)
        }
        "/api/rec-finish" => {
            let (code, v) = crate::recorder::finish(b);
            js(code, v)
        }
        "/api/check-numbers" => {
            let (code, v) = crate::recorder::run_check();
            js(code, v)
        }
        "/api/reset-mic" => {
            let (code, v) = crate::recorder::reset_mic();
            js(code, v)
        }
        "/api/fix-layout" => {
            let (code, v) = crate::recorder::fix_layout();
            js(code, v)
        }
        "/api/open-charts" => {
            let (code, v) = crate::recorder::open_charts();
            js(code, v)
        }
        "/api/rec-open-folder" => {
            let (code, v) = crate::recorder::open_dir();
            js(code, v)
        }
        "/api/exposure-fix" => exposure_fix(b),
        "/api/mode" => {
            let mode = b.get("mode").and_then(|v| v.as_str()).unwrap_or("tether").to_string();
            let mut look = b.get("look").filter(|v| !v.is_null()).cloned();
            if mode == "webcam" && look.is_none() {
                look = webcam_look(None);
            }
            ok(set_mode(&mode, look.as_ref()))
        }
        "/api/delete" => delete(b),
        "/api/undelete" => {
            next_step_changed();
            let d = session_dir(None);
            let trash = d.join(".trash");
            let want = b.get("names").cloned().unwrap_or(json!([]));
            let Some(want) = names_ok(&want) else { return js(400, json!({"ok": false, "error": "bad names"})) };
            let mut back = vec![];
            for f in want {
                let (src, dst) = (trash.join(&f), d.join(&f));
                if src.is_file() && !dst.exists() && std::fs::rename(&src, &dst).is_ok() {
                    back.push(f);
                }
            }
            base_cache_clear();
            ok(json!({"ok": !back.is_empty(), "restored": back,
                      "error": if back.is_empty() { json!("nothing to restore") } else { Value::Null }}))
        }
        "/api/empty-trash" => {
            next_step_changed();
            let trash = session_dir(None).join(".trash");
            let mut n = 0;
            for f in store::list_dir(&trash) {
                let fp = trash.join(f);
                if fp.is_file() && std::fs::remove_file(fp).is_ok() {
                    n += 1;
                }
            }
            ok(json!({"ok": true, "removed": n}))
        }
        "/api/session" => {
            let want = get_str(b, "date").trim().to_string();
            if !want.is_empty() && !store::is_date(&want) {
                return js(400, json!({"ok": false, "error": "bad date"}));
            }
            *app().active_session.write() = if want.is_empty() { None } else { Some(want) };
            base_cache_clear();
            next_step_changed();
            ok(json!({"ok": true, "session": store::active_day()}))
        }
        "/api/pick" => {
            let name = get_str(b, "name");
            if name.contains('/') || name.is_empty() {
                return js(400, json!({"ok": false, "error": "bad name"}));
            }
            let flag = match b.get("flag") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) if s == "pick" || s == "reject" => Some(s.as_str()),
                _ => return js(400, json!({"ok": false, "error": "bad flag"})),
            };
            let picks = store::set_pick(name, flag);
            next_step_changed();
            ok(json!({"ok": true, "picks": picks}))
        }
        "/api/camera-preset" => camera_preset(b),
        "/api/rank" => rank(b),
        "/api/lighting" | "/api/auto-look" => measure(p, b),
        "/api/edits" => {
            let name = get_str(b, "name");
            if name.contains('/') || name.is_empty() {
                return js(400, json!({"ok": false, "error": "bad name"}));
            }
            let session = b.get("session").and_then(Value::as_str);
            if b.get("session").is_some() && !session.is_some_and(store::is_date) {
                return js(400, json!({"ok": false, "error": "bad session"}));
            }
            match store::save_edits(name, b.get("adjust"), b.get("crop"), session) {
                Ok(saved) => ok(json!({"ok": true, "edits": saved})),
                Err(e) => js(500, json!({"ok": false, "error": e})),
            }
        }
        "/api/auto-look-all" => auto_all(b),
        "/api/edits-apply-all" => {
            let adjust = b.get("adjust").filter(|v| truthy(Some(v))).cloned().unwrap_or(json!({}));
            let only = truthy(b.get("picks_only"));
            let existing = store::load_edits(None);
            let mut n = 0;
            for sh in shots() {
                if only && sh["pick"].as_str() != Some("pick") {
                    continue;
                }
                let name = sh["name"].as_str().unwrap_or("");
                let crop = existing.get(name).and_then(|e| e.get("crop")).cloned();
                if let Err(e) = store::save_edits(name, Some(&adjust), crop.as_ref(), None) {
                    return js(500, json!({"ok": false, "error": e, "applied": n}));
                }
                n += 1;
            }
            ok(json!({"ok": true, "applied": n}))
        }
        "/api/contact-sheet" => {
            let cols = num(b.get("cols"), 5.0) as usize;
            match export::contact_sheets(cols, 420, truthy(b.get("picks_only"))) {
                Ok(pages) => {
                    crate::app::release_memory();
                    let total: usize = pages.iter().map(|p| p.1).sum();
                    let first = &pages[0];
                    ok(json!({"ok": true, "path": first.0, "frames": total, "name": fname(&first.0),
                              "dims": [first.2.0, first.2.1],
                              "pages": pages.iter().map(|(fp, n, d)| json!({"name": fname(fp), "path": fp,
                                  "frames": n, "dims": [d.0, d.1]})).collect::<Vec<_>>()}))
                }
                Err(e) => js(400, json!({"ok": false, "error": e})),
            }
        }
        "/api/config" => config(b),
        "/api/call-look" => match feedctl::change_call_look(get_str(b, "id"), b.get("look"), truthy(b.get("follow"))) {
            Ok((live, id)) => ok(json!({"ok": true, "live": live, "id": id})),
            Err(e) => js(400, json!({"ok": false, "error": e})),
        },
        "/api/copy-edits" => {
            let src = get_str(b, "from").to_string();
            let targets: Vec<Value> = match b.get("to") {
                Some(Value::String(s)) => vec![json!(s)],
                Some(Value::Array(a)) => a.clone(),
                None | Some(Value::Null) => vec![],
                Some(_) => return js(400, json!({"ok": false, "error": "bad name"})),
            };
            let valid = !src.is_empty() && !src.contains('/') && !targets.is_empty()
                && targets.iter().all(|t| t.as_str().map(|s| !s.is_empty() && !s.contains('/') && !s.contains("..")).unwrap_or(false));
            if !valid {
                return js(400, json!({"ok": false, "error": "bad name"}));
            }
            let all = store::load_edits(None);
            let Some(e) = all.get(&src).filter(|e| truthy(Some(e))).cloned() else {
                return js(404, json!({"ok": false, "error": "that frame has no edits"}));
            };
            let mut done = vec![];
            for t in targets {
                let dst = t.as_str().unwrap_or("");
                if dst == src {
                    continue;
                }
                let crop = all.get(dst).and_then(|x| x.get("crop")).cloned();
                let adj = e.get("adjust").filter(|v| truthy(Some(v))).cloned().unwrap_or(json!({}));
                if let Err(e) = store::save_edits(dst, Some(&adj), crop.as_ref(), None) {
                    return js(500, json!({"ok": false, "error": e, "copied": done}));
                }
                done.push(dst.to_string());
            }
            ok(json!({"ok": true, "copied": done}))
        }
        "/api/card-import" => card_import(b),
        "/api/look" => {
            let label = get_str(b, "label").trim().to_string();
            if label.is_empty() {
                return js(400, json!({"ok": false, "error": "name it first"}));
            }
            let mut look = Map::new();
            look.insert("label".into(), json!(label));
            look.insert("note".into(), json!("saved from the studio"));
            for k in ["exposure", "shadows", "highlights", "contrast", "saturation", "warmth", "tint",
                      "vignette", "sharpness", "straighten"] {
                let d = crate::grade::adj_default(k).unwrap_or(1.0);
                look.insert(k.into(), json!(num(b.get(k), d)));
            }
            match store::save_look(look) {
                Ok(id) => ok(json!({"ok": true, "id": id})),
                Err(e) => js(500, json!({"ok": false, "error": e})),
            }
        }
        "/api/export" => {
            let (o, mut payload) = render(get_str(b, "name"), b.get("preset").unwrap_or(&json!({})),
                                          b.get("adjust").unwrap_or(&json!({})), b.get("crop"), None, None);
            payload["ok"] = json!(o);
            if o {
                next_step_changed();
            }
            js(if o { 200 } else { 415 }, payload)
        }
        "/api/open-folder" => {
            let target = if b.get("which").and_then(|v| v.as_str()) == Some("exports") { export_dir(None) }
                         else { session_dir(None) };
            if !target.is_dir() {
                return js(404, json!({"ok": false, "error": "no such folder"}));
            }
            if let Err(e) = need("xdg-open") {
                return js(500, json!({"ok": false, "error": e}));
            }
            match std::process::Command::new("xdg-open").arg(&target).stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()).spawn() {
                Ok(_) => ok(json!({"ok": true, "path": target})),
                Err(e) => js(500, json!({"ok": false, "error": e.to_string()})),
            }
        }
        "/api/grab" => grab(b),
        "/api/trim" => trim(b),
        "/api/tidy-exports" => ok(store::tidy_exports(None, !truthy(b.get("confirm")))),
        "/api/export-many" | "/api/export-all" => export_batch(p, b),
        "/api/job-cancel" => ok(json!({"ok": jobs::cancel(get_str(b, "id"))})),
        _ => text(404, "no"),
    }
}

/// An orientation LEASE: the previous one comes back unless something
/// confirms the new one, because a page can die without its own cleanup.
static LEASE: parking_lot::Mutex<(Option<(i64, bool)>, f64)> = parking_lot::Mutex::new((None, 0.0));

fn orient(b: &Value) -> Out {
    if let Some(l) = b.get("revert_after").filter(|v| truthy(Some(v))) {
        let secs = match l {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            Value::Bool(true) => Some(1.0),
            _ => None,
        };
        let Some(secs) = secs else {
            return js(400, json!({"ok": false, "error": "revert_after must be seconds"}));
        };
        let a = cam().a.lock();
        let mut le = LEASE.lock();
        if le.0.is_none() {
            le.0 = Some((a.rotation, a.mirror));
        }
        le.1 = now() + secs.clamp(1.0, 300.0);
    } else if b.get("revert_after").is_some() {
        *LEASE.lock() = (None, 0.0);
    }
    {
        let mut a = cam().a.lock();
        if let Some(r) = b.get("rotation") {
            a.rotation = (num(Some(r), 0.0) as i64).rem_euclid(360);
        }
        if let Some(m) = b.get("mirror") {
            a.mirror = truthy(Some(m));
        }
    }
    let a = cam().a.lock();
    ok(json!({"rotation": a.rotation, "mirror": a.mirror, "leased": LEASE.lock().1 != 0.0}))
}

/// Put a leased orientation back when nothing confirmed it.
pub fn orient_lease_expire() {
    let prev = {
        let mut le = LEASE.lock();
        if le.1 == 0.0 || le.0.is_none() || now() < le.1 {
            return;
        }
        let p = le.0.take();
        le.1 = 0.0;
        p
    };
    if let Some((rot, mir)) = prev {
        let mut a = cam().a.lock();
        if (a.rotation, a.mirror) != (rot, mir) {
            a.rotation = rot;
            a.mirror = mir;
            eprintln!("orientation lease expired, restored rotation {rot} mirror {mir}");
        }
    }
}

fn delete(b: &Value) -> Out {
    next_step_changed();
    let name = get_str(b, "name").to_string();
    if bad(&name) {
        return js(400, json!({"ok": false, "error": "bad name"}));
    }
    let d = session_dir(None);
    let trash = d.join(".trash");
    let _ = std::fs::create_dir_all(&trash);
    if ends_with_any(&name, AUDIO_EXT) {
        let fp = d.join(&name);
        if !fp.is_file() {
            return js(404, json!({"ok": false, "error": "no such take"}));
        }
        let _ = std::fs::rename(&fp, store::trash_dest(&trash, &name));
        return ok(json!({"ok": true, "removed": [name], "recoverable": true}));
    }
    let stem = splitext(&name).0.split('.').next().unwrap_or("").to_string();
    let mut gone = vec![];
    let mut files = store::list_dir(&d);
    files.sort();
    for f in files {
        let fp = d.join(&f);
        if splitext(&f).0.split('.').next() == Some(stem.as_str()) && fp.is_file()
            && std::fs::rename(&fp, store::trash_dest(&trash, &f)).is_ok() {
            gone.push(f);
        }
    }
    if gone.is_empty() {
        return js(404, json!({"ok": false, "error": "no such shot"}));
    }
    ok(json!({"ok": true, "removed": gone, "recoverable": true}))
}

fn camera_preset(b: &Value) -> Out {
    let act = get_str(b, "action");
    let name: String = get_str(b, "name").trim().chars().take(60).collect();
    let mut presets = store::load_cam_presets();
    match act {
        "save" => {
            if name.is_empty() {
                return js(400, json!({"ok": false, "error": "give the preset a name"}));
            }
            let vals = capture_cam_preset(&cam().settings());
            if vals.is_empty() {
                return js(409, json!({"ok": false, "error": "no settings to save, is the camera connected?"}));
            }
            presets.insert(name.clone(), json!({"values": vals, "at": now()}));
            store::save_cam_presets(&presets);
            ok(json!({"ok": true, "name": name, "values": vals, "presets": presets}))
        }
        "apply" => {
            let Some(p) = presets.get(&name) else { return js(404, json!({"ok": false, "error": "no such preset"})) };
            let vals = p.get("values").and_then(|v| v.as_object()).cloned().unwrap_or_default();
            let mut res = apply_cam_preset(&vals);
            res["name"] = json!(name);
            res["presets"] = Value::Object(presets.clone());
            let code = if res["ok"] == json!(true) { 200 } else { 409 };
            js(code, res)
        }
        "delete" => {
            if presets.shift_remove(&name).is_none() {
                return js(404, json!({"ok": false, "error": "no such preset"}));
            }
            store::save_cam_presets(&presets);
            ok(json!({"ok": true, "presets": presets}))
        }
        _ => js(400, json!({"ok": false, "error": "bad action"})),
    }
}

fn queue_of(b: &Value) -> Vec<Value> {
    let only = truthy(b.get("picks_only"));
    shots().into_iter().filter(|s| {
        let p = s["pick"].as_str();
        p != Some("reject") && (!only || p == Some("pick"))
    }).collect()
}

fn rank(b: &Value) -> Out {
    let names: Vec<String> = queue_of(b).iter().filter_map(|s| s["name"].as_str().map(|x| x.to_string())).collect();
    if names.is_empty() {
        return js(400, json!({"ok": false, "error": "nothing to rank"}));
    }
    let sess = store::active_day();
    let jid = jobs::new("rank", names.len());
    let (id, total) = (jid.clone(), names.len());
    jobs::spawn(jid.clone(), move || {
        // Score every frame first, THEN rank: focus is relative to the set.
        let mut raw = vec![];
        let finished = jobs::par_each(&names, jobs::batch_workers(), |n| {
            let fp = session_dir(Some(&sess)).join(n);
            let r = if !fp.exists() {
                Ok(json!({"name": n, "ok": false, "why": "gone from disk"}))
            } else {
                load_editable(&fp, Some(2200)).map(|im| match analysis::frame_quality(&oriented(im)) {
                    None => json!({"name": n, "ok": false, "why": "no face found in this frame"}),
                    Some(mut q) => {
                        q["name"] = json!(n);
                        q["ok"] = json!(true);
                        q
                    }
                })
            };
            match r {
                Ok(v) => (true, v),
                Err(e) => (false, json!({"name": n, "ok": false, "why": format!("Error: {e}")})),
            }
        }, |n, (okr, payload)| {
            raw.push(payload);
            jobs::step(&id, okr, json!({"name": n}))
        });
        if !finished {
            jobs::finish(&id, "cancelled");
            return;
        }
        jobs::set(&id, "ranked", json!(analysis::rank_scored(raw)));
        jobs::finish(&id, "done");
    });
    ok(json!({"ok": true, "job": jid, "total": total}))
}

fn measure(p: &str, b: &Value) -> Out {
    let name = get_str(b, "name");
    if bad(name) {
        return js(400, json!({"ok": false, "error": "bad name"}));
    }
    let src = session_dir(None).join(name);
    if !src.exists() {
        return js(404, json!({"ok": false, "error": "no such shot"}));
    }
    let base = match crate::export::full_base(&src) {
        Ok(im) => im,
        Err(e) => return js(500, json!({"ok": false, "error": format!("Error: {e}")})),
    };
    // Nothing to straighten or crop: measure the shared frame itself rather
    // than copying 136 MB first (about 100 ms of the old 135).
    let deg = num(b.get("straighten"), 0.0);
    let cropped = b.get("crop").map(|c| c.is_object()).unwrap_or(false);
    let owned;
    let im: &img::Rgb = if deg == 0.0 && !cropped {
        &base
    } else {
        owned = apply_crop(straighten((*base).clone(), deg), b.get("crop"));
        &owned
    };
    if p == "/api/lighting" {
        return ok(analysis::lighting_report(im));
    }
    let (adj, why, measured) = analysis::auto_look(im);
    ok(json!({"ok": true, "adjust": adj, "why": why, "measured": measured}))
}

fn auto_all(b: &Value) -> Out {
    if !jobs::running("auto").is_empty() {
        return js(409, json!({"ok": false, "busy": true, "error": "Auto is already grading these frames."}));
    }
    let existing = store::load_edits(None);
    let queue = queue_of(b);
    if queue.is_empty() {
        return js(400, json!({"ok": false, "error": "nothing to grade"}));
    }
    let sess = store::active_day();
    let jid = jobs::new("auto", queue.len());
    let (id, total) = (jid.clone(), queue.len());
    jobs::spawn(jid.clone(), move || {
        // Measuring runs in parallel; every save to edits.json happens on
        // this thread, one at a time, or two saves would race the file.
        let finished = jobs::par_each(&queue, jobs::batch_workers(), |sh| {
            let name = sh["name"].as_str().unwrap_or("").to_string();
            let e = existing.get(&name).cloned().unwrap_or(json!({}));
            let adjust = e.get("adjust").filter(|v| v.is_object()).cloned().unwrap_or(json!({}));
            load_editable(&session_dir(Some(&sess)).join(&name), None).map(|im| {
                let im = straighten(oriented(im), num(adjust.get("straighten"), 0.0));
                let im = apply_crop(im, e.get("crop"));
                analysis::auto_look(&im)
            })
        }, |sh, r| {
            let name = sh["name"].as_str().unwrap_or("").to_string();
            let e = existing.get(&name).cloned().unwrap_or(json!({}));
            let adjust = e.get("adjust").filter(|v| v.is_object()).cloned().unwrap_or(json!({}));
            let (okr, payload) = match r {
                Ok((adj, why, meas)) => {
                    // Keep the frame's own crop and straighten: Auto is a LOOK.
                    let mut keep = adjust.as_object().cloned().unwrap_or_default();
                    for (k, v) in adj {
                        keep.insert(k, v);
                    }
                    if let Some(s) = adjust.get("straighten") {
                        keep.insert("straighten".into(), s.clone());
                    }
                    match store::save_edits(&name, Some(&Value::Object(keep)), e.get("crop"), Some(&sess)) {
                        Ok(_) => (true, json!({"name": name, "faceY": meas["faceY"], "changes": why.len()})),
                        Err(e) => (false, json!({"name": name, "error": e})),
                    }
                }
                Err(err) => (false, json!({"name": name, "error": format!("Error: {err}")})),
            };
            jobs::step(&id, okr, payload)
        });
        jobs::finish(&id, if finished { "done" } else { "cancelled" });
    });
    ok(json!({"ok": true, "job": jid, "total": total}))
}

fn config(b: &Value) -> Out {
    let mut body = b.as_object().cloned().unwrap_or_default();
    let transient = truthy(body.shift_remove("transient").as_ref());
    let defaults = store::default_config();
    if transient {
        if let Some(k) = body.keys().find(|k| !defaults.contains_key(*k)) {
            return js(400, json!({"ok": false, "error": format!("unknown setting: {k}")}));
        }
        for k in ["export_dir", "capture_dir"] {
            let d = body.get(k).map(|v| if v.is_null() { String::new() } else { py_str(v) }).unwrap_or_default();
            if !d.is_empty() {
                let p = crate::app::expand_user(&d);
                if !p.is_dir() {
                    if let Err(e) = std::fs::create_dir_all(&p) {
                        return js(400, json!({"ok": false, "error": format!("cannot use that folder: {e}")}));
                    }
                }
            }
        }
        {
            let mut t = app().transient.lock();
            for (k, v) in body {
                if v.is_null() {
                    t.shift_remove(&k);
                } else {
                    t.insert(k, v);
                }
            }
        }
        let mut keys: Vec<String> = app().transient.lock().keys().cloned().collect();
        keys.sort();
        return ok(json!({"ok": true, "config": store::load_config(), "transient": keys,
                         "stored": store::stored_config(), "resolved": export_dir(None)}));
    }
    if let Err(e) = store::save_config(&body) {
        return js(400, json!({"ok": false, "error": e}));
    }
    let mut keys: Vec<String> = app().transient.lock().keys().cloned().collect();
    keys.sort();
    ok(json!({"ok": true, "config": store::load_config(), "transient": keys, "resolved": export_dir(None)}))
}

fn card_import(b: &Value) -> Out {
    next_step_changed();
    let want = b.get("files").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if want.is_empty() {
        return js(400, json!({"ok": false, "error": "nothing selected"}));
    }
    let (fok, why) = store::disk_ok(&today_dir());
    if !fok {
        return js(507, json!({"ok": false, "error": why}));
    }
    let queue: Vec<Value> = want.into_iter().take(400).collect();
    let dest = today_dir();
    let jid = jobs::new("import", queue.len());
    let (id, total) = (jid.clone(), queue.len());
    jobs::spawn(jid.clone(), move || {
        for f in &queue {
            let folder = f.get("folder").map(py_str).unwrap_or_default();
            let name = f.get("name").map(py_str).unwrap_or_default();
            if !folder.starts_with('/') || name.contains('/') || name.is_empty() {
                if !jobs::step(&id, false, json!({"name": name, "error": "bad path"})) {
                    jobs::finish(&id, "cancelled");
                    return;
                }
                continue;
            }
            let (fok, why) = store::disk_ok(&dest);
            if !fok {
                jobs::step(&id, false, json!({"name": name, "error": why, "disk_full": true}));
                jobs::finish(&id, "stopped");
                return;
            }
            let (okc, payload) = match cam().card_get(&folder, &name, &dest) {
                Ok((true, res)) => (true, json!({"name": res})),
                Ok((false, res)) => (false, json!({"name": name, "error": res})),
                Err(Busy(m)) => (false, json!({"name": name, "error": m})),
            };
            if !jobs::step(&id, okc, payload) {
                jobs::finish(&id, "cancelled");
                return;
            }
        }
        jobs::finish(&id, "done");
    });
    ok(json!({"ok": true, "job": jid, "total": total}))
}

fn clip_arg(b: &Value, key: &str, err: &str) -> Result<(String, PathBuf), Out> {
    let name = get_str(b, key).to_string();
    if bad(&name) || !ends_with_any(&name, VIDEO_EXT) {
        return Err(js(400, json!({"ok": false, "error": err})));
    }
    let src = session_dir(None).join(&name);
    if !src.exists() {
        return Err(js(404, json!({"ok": false, "error": "no such clip"})));
    }
    Ok((name, src))
}

fn grab(b: &Value) -> Out {
    let (name, src) = match clip_arg(b, "name", "bad clip") {
        Ok(v) => v,
        Err(o) => return o,
    };
    let at = match b.get("at") {
        None | Some(Value::Null) => 0.0,
        Some(v) => {
            let x = num(Some(v), f64::NAN);
            if x.is_nan() {
                return js(400, json!({"ok": false, "error": "bad timestamp"}));
            }
            x.max(0.0)
        }
    };
    let dur = num(store::video_meta(&src).get("duration"), 0.0);
    if dur > 0.0 && at > dur {
        return js(400, json!({"ok": false, "error": format!("that is past the end of a {dur:.1}s clip")}));
    }
    let d = session_dir(None);
    let (fok, why) = store::disk_ok(&d);
    if !fok {
        return js(507, json!({"ok": false, "error": why, "disk_full": true}));
    }
    if let Err(e) = need("ffmpeg") {
        return js(503, json!({"ok": false, "error": e}));
    }
    let stem = splitext(&name).0;
    let cs = (at * 100.0) as i64;
    let mut out = d.join(format!("{stem}-t{cs:06}.jpg"));
    let mut n = 2;
    while out.exists() {
        out = d.join(format!("{stem}-t{cs:06}~{n}.jpg"));
        n += 1;
    }
    let r = run_timeout(std::process::Command::new("ffmpeg").args(["-y", "-v", "error", "-i"]).arg(&src)
        .args(["-ss", &format!("{at:.3}"), "-frames:v", "1", "-q:v", "2"]).arg(&out), Duration::from_secs(300));
    if !out.exists() {
        let tail = r.ok().and_then(|o| last_line(&o.stderr)).unwrap_or_else(|| "ffmpeg produced nothing".into());
        return js(500, json!({"ok": false, "error": tail}));
    }
    base_cache_clear();
    let dims = std::fs::read(&out).ok().and_then(|d| img::jpeg_info(&d).ok()).map(|(w, h)| vec![w, h])
        .unwrap_or_default();
    ok(json!({"ok": true, "name": fname(&out), "path": out, "at": crate::app::round(at, 2),
              "size": std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0), "dims": dims}))
}

fn trim(b: &Value) -> Out {
    let (name, src) = match clip_arg(b, "name", "bad clip") {
        Ok(v) => v,
        Err(o) => return o,
    };
    let (start, mut end) = (num(b.get("start"), f64::NAN), num(b.get("end"), f64::NAN));
    let start = if b.get("start").map(|v| v.is_null()).unwrap_or(true) { 0.0 } else { start };
    if b.get("end").map(|v| v.is_null()).unwrap_or(true) {
        end = 0.0;
    }
    if start.is_nan() || end.is_nan() {
        return js(400, json!({"ok": false, "error": "bad range"}));
    }
    let start = start.max(0.0);
    let dur = num(store::video_meta(&src).get("duration"), 0.0);
    if dur > 0.0 {
        end = end.min(dur);
    }
    if end - start < 0.2 {
        return js(400, json!({"ok": false, "error": "the out point must be at least 0.2s after the in point"}));
    }
    let outdir = export_dir(None);
    let (fok, why) = store::disk_ok(&outdir);
    if !fok {
        return js(507, json!({"ok": false, "error": why}));
    }
    let (stem, ext) = splitext(&name);
    let mut out = outdir.join(format!("{stem}_trim{ext}"));
    let mut n = 2;
    while out.exists() {
        out = outdir.join(format!("{stem}_trim{n}{ext}"));
        n += 1;
    }
    let mut cmd = std::process::Command::new("ffmpeg");
    if b.get("mode").and_then(|v| v.as_str()) == Some("copy") {
        cmd.args(["-y", "-ss", &format!("{start:.3}"), "-to", &format!("{end:.3}"), "-i"]).arg(&src)
            .args(["-c", "copy", "-avoid_negative_ts", "make_zero"]).arg(&out);
    } else {
        cmd.args(["-y", "-i"]).arg(&src).args(["-ss", &format!("{start:.3}"), "-to", &format!("{end:.3}"),
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "aac",
            "-movflags", "+faststart"]).arg(&out);
    }
    if let Err(e) = need("ffmpeg") {
        return js(503, json!({"ok": false, "error": e}));
    }
    let jid = jobs::new("trim", 1);
    let id = jid.clone();
    let out2 = out.clone();
    jobs::spawn(jid.clone(), move || {
        match run_timeout(&mut cmd, Duration::from_secs(1800)) {
            Ok(o) if o.status.success() && out2.exists() => {
                jobs::step(&id, true, json!({"name": fname(&out2), "path": out2,
                    "size": std::fs::metadata(&out2).map(|m| m.len()).unwrap_or(0),
                    "seconds": crate::app::round(end - start, 2)}));
            }
            Ok(o) => {
                jobs::step(&id, false, json!({"name": name, "error": last_line(&o.stderr).unwrap_or_else(|| "ffmpeg failed".into())}));
            }
            Err(e) if e == "timed out" => {
                jobs::step(&id, false, json!({"name": name, "error": "ffmpeg timed out"}));
            }
            Err(e) => {
                jobs::step(&id, false, json!({"name": name, "error": e}));
            }
        }
        jobs::finish(&id, "done");
    });
    ok(json!({"ok": true, "job": jid, "total": 1, "out": fname(&out)}))
}

fn sync_audio(b: &Value) -> Out {
    let clip = get_str(b, "clip").to_string();
    let take = get_str(b, "take").to_string();
    if bad(&clip) || !ends_with_any(&clip, VIDEO_EXT) {
        return js(400, json!({"ok": false, "error": "bad clip"}));
    }
    if bad(&take) || !ends_with_any(&take, AUDIO_EXT) {
        return js(400, json!({"ok": false, "error": "bad take"}));
    }
    let (cpath, tpath) = (session_dir(None).join(&clip), session_dir(None).join(&take));
    for (fp, what) in [(&cpath, "clip"), (&tpath, "take")] {
        if !fp.exists() {
            return js(404, json!({"ok": false, "error": format!("no such {what}")}));
        }
    }
    if let Err(e) = need("ffmpeg") {
        return js(503, json!({"ok": false, "error": e}));
    }
    let outdir = export_dir(None);
    let (fok, why) = store::disk_ok(&outdir);
    if !fok {
        return js(507, json!({"ok": false, "error": why, "disk_full": true}));
    }
    let Some(cam_at) = audio::loudest_onset(&cpath, 20.0) else {
        return js(422, json!({"ok": false, "error": "the clip has no audio track to sync against"}));
    };
    let Some(take_at) = audio::loudest_onset(&tpath, 20.0) else {
        return js(422, json!({"ok": false, "error": "could not read the take"}));
    };
    let (offset, how) = match b.get("offset").filter(|v| !v.is_null()) {
        Some(v) => {
            let x = num(Some(v), f64::NAN);
            if x.is_nan() {
                return js(400, json!({"ok": false, "error": "bad offset"}));
            }
            (x, "manual")
        }
        None => (crate::app::round(take_at - cam_at, 3), "clap"),
    };
    let stem = splitext(&clip).0;
    let mut out = outdir.join(format!("{stem}_synced.mp4"));
    let mut n = 2;
    while out.exists() {
        out = outdir.join(format!("{stem}_synced{n}.mp4"));
        n += 1;
    }
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-y", "-v", "error", "-i"]).arg(&cpath);
    if offset >= 0.0 {
        cmd.args(["-ss", &format!("{offset:.3}"), "-i"]).arg(&tpath);
    } else {
        cmd.args(["-itsoffset", &format!("{:.3}", -offset), "-i"]).arg(&tpath);
    }
    cmd.args(["-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-shortest",
              "-movflags", "+faststart"]).arg(&out);
    let jid = jobs::new("sync", 1);
    let id = jid.clone();
    let out2 = out.clone();
    let clip2 = clip.clone();
    jobs::spawn(jid.clone(), move || {
        match run_timeout(&mut cmd, Duration::from_secs(1800)) {
            Ok(o) if o.status.success() && out2.exists() => {
                jobs::step(&id, true, json!({"name": fname(&out2), "path": out2,
                    "size": std::fs::metadata(&out2).map(|m| m.len()).unwrap_or(0),
                    "offset": offset, "how": how, "camera_clap": cam_at, "take_clap": take_at}));
            }
            Ok(o) => {
                jobs::step(&id, false, json!({"name": clip2, "error": last_line(&o.stderr).unwrap_or_else(|| "ffmpeg failed".into())}));
            }
            Err(e) => {
                jobs::step(&id, false, json!({"name": clip2, "error": e}));
            }
        }
        jobs::finish(&id, "done");
    });
    ok(json!({"ok": true, "job": jid, "total": 1, "offset": offset, "how": how,
              "camera_clap": cam_at, "take_clap": take_at}))
}

fn exposure_fix(b: &Value) -> Out {
    let was_webcam = webcam_running();
    let mut steps: Vec<String> = vec![];
    // A panic must not skip the handback below, or a call loses its camera.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        set_mode("tether", None);
        if !cam().is_open() {
            let err = cam().a.lock().error.clone().unwrap_or_else(|| "could not take the camera back".into());
            return js(409, json!({"ok": false, "error": err, "steps": steps}));
        }
        let cur = cam().settings();
        if cur.get("expprogram").and_then(|v| v.get("choices")).and_then(|c| c.as_array())
            .map(|c| c.iter().any(|x| x == "A")).unwrap_or(false) {
            match cam().set_setting("expprogram", "A") {
                Ok((true, _)) => steps.push("expprogram=A".into()),
                Ok((false, m)) | Err(Busy(m)) => steps.push(format!("expprogram refused: {m}")),
            }
        }
        let ap: Vec<String> = cur.get("aperture").and_then(|v| v.get("choices")).and_then(|c| c.as_array())
            .map(|c| c.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
        if let Some(first) = ap.first() {
            match cam().set_setting("aperture", first) {
                Ok((true, _)) => steps.push(format!("aperture={first}")),
                Ok((false, m)) | Err(Busy(m)) => steps.push(format!("aperture refused: {m}")),
            }
        }
        let after = cam().settings();
        let ecv = after.get("exposurecomp").and_then(|v| v.get("value")).map(py_str).unwrap_or_else(|| "0".into());
        if advice::ev_value(&ecv) != 0.0 {
            steps.push(format!("NOTE exposure comp {ecv} is now live: it does nothing in M with a fixed ISO, \
                                but aperture priority obeys it. Set it to 0 if that is not what you want."));
        }
        let pick = |m: &Map<String, Value>| -> Value {
            let mut o = Map::new();
            for k in ["expprogram", "aperture", "shutter", "iso"] {
                o.insert(k.into(), m.get(k).and_then(|v| v.get("value")).cloned().unwrap_or(Value::Null));
            }
            Value::Object(o)
        };
        ok(json!({"ok": true, "steps": steps, "before": pick(&cur), "after": pick(&after)}))
    }));
    // Always hand the camera back, with the look the feed was last started with.
    if was_webcam {
        let look = b.get("look").filter(|v| !v.is_null()).cloned().or_else(feedctl::marker_look);
        set_mode("webcam", look.as_ref());
    }
    result.unwrap_or_else(|e| std::panic::resume_unwind(e))
}

fn export_batch(p: &str, b: &Value) -> Out {
    next_step_changed();
    let saved = store::load_edits(None);
    let names: Vec<String> = if p == "/api/export-many" {
        match b.get("names").and_then(|v| if truthy(Some(v)) { names_ok(v) } else { None }) {
            Some(n) => n,
            None => return js(400, json!({"ok": false, "error": "bad names"})),
        }
    } else {
        let q = queue_of(b);
        if q.is_empty() {
            return js(400, json!({"ok": false, "error": "nothing to export"}));
        }
        q.iter().filter_map(|s| s["name"].as_str().map(|x| x.to_string())).collect()
    };
    if !jobs::running("export").is_empty() {
        return js(409, json!({"ok": false, "busy": true, "error": if p == "/api/export-many" {
            "An export is already running." } else {
            "An export is already running. A second one writes the same files twice and doubles the memory." }}));
    }
    if !jobs::running("auto").is_empty() {
        return js(409, json!({"ok": false, "busy": true, "error": if p == "/api/export-many" {
            "Auto is still grading these frames. Wait for it to finish." } else {
            "Auto is still grading these frames. The export would use the grades as they are right now and \
             miss the rest. Wait for it to finish." }}));
    }
    let preset = b.get("preset").cloned().unwrap_or(json!({}));
    let fallback = b.get("adjust").filter(|v| truthy(Some(v))).cloned().unwrap_or(json!({}));
    let sess = store::active_day();
    let outdir = export_dir(Some(&sess));
    let queue: Vec<String> = if p == "/api/export-many" {
        let have: std::collections::HashSet<String> = shots().iter()
            .filter_map(|s| s["name"].as_str().map(|x| x.to_string())).collect();
        let q: Vec<String> = names.into_iter().filter(|n| have.contains(n)).collect();
        if q.is_empty() {
            return js(404, json!({"ok": false, "error": "none of those frames are in this session"}));
        }
        q
    } else {
        names
    };
    let jid = jobs::new("export", queue.len());
    let (id, total) = (jid.clone(), queue.len());
    jobs::spawn(jid.clone(), move || {
        // Frames render in parallel; every step and every stop is decided on
        // this thread, in the order they finish.
        let mut end = "done";
        jobs::par_each(&queue, jobs::batch_workers(), |name| {
            let e = saved.get(name).cloned().unwrap_or(json!({}));
            let adj = e.get("adjust").filter(|v| truthy(Some(v))).cloned().unwrap_or_else(|| fallback.clone());
            render(name, &preset, &adj, e.get("crop"), Some(&sess), Some(&outdir))
        }, |_, (okr, payload)| {
            let full = payload.get("disk_full").map(|v| truthy(Some(v))).unwrap_or(false);
            let keep = jobs::step(&id, okr, payload);
            if !okr && full {
                end = "stopped";
                return false;
            }
            if !keep {
                end = "cancelled";
                return false;
            }
            true
        });
        jobs::finish(&id, end);
    });
    ok(json!({"ok": true, "job": jid, "total": total}))
}

#[cfg(test)]
mod phone_access_tests {
    use super::*;

    fn headers(host: &str, origin: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("host", host.parse().unwrap());
        if let Some(origin) = origin { h.insert("origin", origin.parse().unwrap()); }
        h
    }

    const REMOTE: &str = "https://studio.example.ts.net";

    #[test]
    fn private_proxy_can_read_and_post_without_opening_other_hosts() {
        let h = headers("studio.example.ts.net", Some(REMOTE));
        for m in [Method::GET, Method::POST] {
            assert!(request_origin_error(&m, &h, 8420, Some(REMOTE)).is_none());
            assert!(request_origin_error(&m, &h, 8420, None).is_some());
        }
        for host in ["studio.example.ts.net.evil.test", "evil.test", "192.168.1.2:8420"] {
            assert!(request_origin_error(&Method::GET, &headers(host, None), 8420, Some(REMOTE)).is_some());
        }
    }

    #[test]
    fn phone_posts_still_reject_cross_site_origins() {
        for origin in ["https://evil.test", "null", "http://studio.example.ts.net",
            "https://studio.example.ts.net.evil.test", "https://studio.example.ts.net:444"] {
            let h = headers("studio.example.ts.net", Some(origin));
            assert!(request_origin_error(&Method::POST, &h, 8420, Some(REMOTE)).is_some(), "{origin}");
        }
    }

    #[test]
    fn desktop_and_local_scripts_keep_working() {
        for host in ["127.0.0.1:8420", "localhost:8420", "[::1]:8420"] {
            let origin = format!("http://{host}");
            for remote in [None, Some(REMOTE)] {
                for origin in [None, Some(origin.as_str())] {
                    assert!(request_origin_error(&Method::POST, &headers(host, origin), 8420, remote).is_none());
                }
            }
        }
    }

    #[test]
    fn remote_origin_config_is_one_explicit_https_origin() {
        assert_eq!(crate::app::parse_remote_origin("https://Studio.Example.ts.net/"), Ok(REMOTE.into()));
        for bad in ["http://studio.example.ts.net", "https://*.ts.net", "https://studio.example.ts.net/path",
            "https://studio.example.ts.net?query", "https://user@studio.example.ts.net", "null", "*"] {
            assert!(crate::app::parse_remote_origin(bad).is_err(), "{bad}");
        }
    }
}
