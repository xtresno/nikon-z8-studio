//! Rendering: the preview base cache, a frame's export (graded, or copied
//! byte for byte), and contact sheets.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::app::{mtime, num};
use crate::camera::cam;
use crate::grade::{adj_default, apply_crop, apply_edits, apply_edits_owned, fit_to, straighten, Phase};
use crate::img::{self, Rgb};
use crate::load::{load_editable, oriented};
use crate::store::{self, atomic_replace, disk_ok, export_dir, export_name, py_str,
                   session_dir, splitext, staging_path, write_out};

pub const PREVIEW_EDGE: usize = 1100;

type Key = (PathBuf, u64, bool);

struct Base {
    cache: Vec<(Key, Arc<Rgb>)>,
    inflight: HashMap<Key, Arc<Mutex<()>>>,
}

static BASE: std::sync::LazyLock<Mutex<Base>> =
    std::sync::LazyLock::new(|| Mutex::new(Base { cache: Vec::new(), inflight: HashMap::new() }));

/// The frame being inspected, full size and oriented: the loupe, Auto and the
/// Light report each decoded all 45 MP on every call (about 250 ms; a loupe
/// drag paid it per move). One frame is kept, and let go after a minute idle
/// so 136 MB is not held for good. Results are identical to a fresh decode.
static FULL: Mutex<Option<(Key, Arc<Rgb>, std::time::Instant)>> = Mutex::new(None);
static FULL_GATE: Mutex<()> = Mutex::new(());

pub fn full_base(path: &Path) -> Result<Arc<Rgb>, String> {
    let key: Key = (path.to_path_buf(), mtime(path).ok_or("no such frame")?.to_bits(), cam().mirror());
    let hit = |f: &mut Option<(Key, Arc<Rgb>, std::time::Instant)>| match f {
        Some((k, im, used)) if *k == key => {
            *used = std::time::Instant::now();
            Some(im.clone())
        }
        _ => None,
    };
    if let Some(im) = hit(&mut FULL.lock()) {
        return Ok(im);
    }
    // One decode at a time: a burst of loupe requests waits for the first.
    let _g = FULL_GATE.lock();
    if let Some(im) = hit(&mut FULL.lock()) {
        return Ok(im);
    }
    let im = Arc::new(oriented(load_editable(path, None)?));
    *FULL.lock() = Some((key, im.clone(), std::time::Instant::now()));
    Ok(im)
}

/// The full-size frame if it is ALREADY decoded; never decodes or evicts.
/// An export of the frame just inspected (Auto, Light, the loupe) skips the
/// decode and the EXIF turn; batch exports, three frames at once, only peek,
/// so they cannot churn the entry.
pub fn full_base_peek(path: &Path) -> Option<Arc<Rgb>> {
    let key: Key = (path.to_path_buf(), mtime(path)?.to_bits(), cam().mirror());
    match &mut *FULL.lock() {
        Some((k, im, used)) if *k == key => {
            *used = std::time::Instant::now();
            Some(im.clone())
        }
        _ => None,
    }
}

/// Let the full-size frame go once nobody has asked for it in a minute.
pub fn full_base_expire() {
    let mut f = FULL.lock();
    if matches!(&*f, Some((_, _, used)) if used.elapsed() > std::time::Duration::from_secs(60)) {
        *f = None;
    }
}

/// A downscaled, oriented copy of a frame, in memory. One decode per frame
/// however many requests ask at once (stage, crop pane and histogram arrive
/// together), drafted at 1/2 of twice the preview edge.
pub fn preview_base(path: &Path) -> Result<Arc<Rgb>, String> {
    let key: Key = (path.to_path_buf(), mtime(path).ok_or("no such frame")?.to_bits(), cam().mirror());
    let gate = {
        let mut b = BASE.lock();
        if let Some((_, im)) = b.cache.iter().find(|(k, _)| *k == key) {
            return Ok(im.clone());
        }
        b.inflight.entry(key.clone()).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
    };
    let _g = gate.lock();
    if let Some((_, im)) = BASE.lock().cache.iter().find(|(k, _)| *k == key) {
        return Ok(im.clone());
    }
    let r = load_editable(path, Some(2 * PREVIEW_EDGE)).map(|im| {
        let im = oriented(im);
        Arc::new(img::thumbnail(&im, PREVIEW_EDGE, PREVIEW_EDGE))
    });
    let mut b = BASE.lock();
    b.inflight.remove(&key);
    let im = r?;
    if b.cache.len() > 12 {
        b.cache.remove(0);      // the oldest, not everything
    }
    b.cache.push((key, im.clone()));
    Ok(im)
}

pub fn base_cache_clear() {
    BASE.lock().cache.clear();
}

/// Shared by single and batch export. `which` pins the session and `outdir`
/// the destination for a whole batch. (ok, payload)
pub fn render(name: &str, pre: &Value, adj: &Value, crop: Option<&Value>, which: Option<&str>,
              outdir: Option<&Path>) -> (bool, Value) {
    let outdir: PathBuf = outdir.map(|p| p.to_path_buf()).unwrap_or_else(|| export_dir(which));
    let (ok, why) = disk_ok(&outdir);
    if !ok {
        return (false, json!({"name": name, "error": why, "disk_full": true}));
    }
    if name.contains('/') || name.contains("..") {
        return (false, json!({"name": name, "error": "bad name"}));
    }
    let src = session_dir(which).join(name);
    if !src.exists() {
        return (false, json!({"name": name, "error": "no such shot"}));
    }
    let crop = crop.filter(|c| crate::app::truthy(Some(c)));
    let fmt = pre.get("format").map(py_str).unwrap_or_default().to_uppercase();
    if fmt == "COPY" {
        // The camera's own file, byte for byte; and say exactly which edits
        // it did not apply.
        let mut ignored: Vec<String> = adj.as_object().map(|m| m.iter().filter(|(k, v)| {
            adj_default(k).map(|d| (num(Some(v), d) - d).abs() > 1e-9).unwrap_or(false)
        }).map(|(k, _)| k.clone()).collect()).unwrap_or_default();
        if crop.is_some() {
            ignored.push("crop".into());
        }
        ignored.sort();
        ignored.dedup();
        let ext = splitext(name).1.to_lowercase();
        let out_name = export_name(name, pre, Some(&ext));
        let out = outdir.join(&out_name);
        let r = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&outdir)?;
            let tmp = staging_path(&out);
            let res = std::fs::copy(&src, &tmp).and_then(|_| atomic_replace(&tmp, &out));
            if res.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            res
        })();
        if let Err(e) = r {
            return (false, json!({"name": name, "error": e.to_string()}));
        }
        let dims = std::fs::read(&out).ok().and_then(|d| img::jpeg_info(&d).ok().map(|(w, h)| {
            let o = crate::load::exif_orientation(&d);
            if o >= 5 { vec![h, w] } else { vec![w, h] }
        })).unwrap_or_else(|| vec![0, 0]);
        return (true, json!({"name": out_name, "path": out,
                             "size": std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
                             "dims": dims, "copied": true, "ignored_edits": ignored}));
    }
    let r = (|| -> Result<(String, PathBuf, u64, (usize, usize)), String> {
        let base = match full_base_peek(&src) {
            Some(b) => (*b).clone(),
            None => oriented(load_editable(&src, None)?),
        };
        let im = straighten(base, num(adj.get("straighten"), 0.0));
        let im = apply_crop(im, crop);
        let im = apply_edits_owned(im, adj, Phase::Point);
        let w = num(pre.get("w"), 0.0) as usize;
        let h = num(pre.get("h"), 0.0) as usize;
        let fit = pre.get("fit").map(py_str).unwrap_or_else(|| "cover".into());
        let anchor = pre.get("anchor").map(py_str).unwrap_or_else(|| "center".into());
        let im = fit_to(im, w, h, &fit, &anchor);
        let im = apply_edits_owned(im, adj, Phase::Spatial);
        let (n, p, s) = write_out(&im, name, pre, &outdir)?;
        Ok((n, p, s, (im.w, im.h)))
    })();
    match r {
        Ok((n, p, s, (w, h))) => (true, json!({"name": n, "path": p, "size": s, "dims": [w, h]})),
        Err(e) => (false, json!({"name": name, "error": e})),
    }
}

// ------------------------------------------------------------------ sheets
const ROWS_PER_PAGE: usize = 6;

/// One written page: its path, how many frames, its pixel size.
pub type SheetPage = (PathBuf, usize, (usize, usize));

fn sheet_font() -> Option<img::Font> {
    img::Font::load(&["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                      "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"])
}

fn render_sheet(rows: &[Value], which: Option<&str>, saved: &Map<String, Value>, font: Option<&img::Font>,
                cols: usize, cell: usize, out: &Path) -> Result<SheetPage, String> {
    let (pad, label_h) = (10usize, 22usize);
    let mut thumbs: Vec<(String, Rgb)> = Vec::new();
    for sh in rows {
        let name = sh["name"].as_str().unwrap_or("");
        let src = session_dir(which).join(name);
        let e = saved.get(name).cloned().unwrap_or(json!({}));
        let adjust = e.get("adjust").cloned().unwrap_or(json!({}));
        let im = match load_editable(&src, Some(cell * 2)) {
            Ok(im) => straighten(oriented(im), num(adjust.get("straighten"), 0.0)),
            Err(e) => {
                eprintln!("contact sheet: skipping {name}: {e}");
                continue;
            }
        };
        let im = apply_crop(im, e.get("crop"));
        let im = apply_edits(&im, &adjust, Phase::All);
        thumbs.push((name.to_string(), img::thumbnail(&im, cell, cell)));
    }
    if thumbs.is_empty() {
        return Err("none of these frames could be read".into());
    }
    let ncols = cols.min(thumbs.len());
    let nrows = thumbs.len().div_ceil(ncols);
    let cw = thumbs.iter().map(|t| t.1.w).max().unwrap_or(0) + pad * 2;
    let ch = thumbs.iter().map(|t| t.1.h).max().unwrap_or(0) + pad * 2 + label_h;
    let mut sheet = Rgb::new(ncols * cw, nrows * ch, [18, 20, 24]);
    for (i, (name, t)) in thumbs.iter().enumerate() {
        let (cx, cy) = ((i % ncols) * cw, (i / ncols) * ch);
        img::paste(&mut sheet, t, cx + (cw - t.w) / 2, cy + pad);
        if let Some(f) = font {
            f.draw(&mut sheet, splitext(name).0, (cx + pad) as f32, (cy + ch - label_h + 3) as f32, 13.0,
                   [150, 160, 172]);
        }
    }
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = staging_path(out);
    let bytes = img::jpeg_encode(&sheet, &img::JpegOpts { quality: 88, subsampling: -1, optimize: true, dpi: 0 })?;
    let r = std::fs::write(&tmp, bytes).and_then(|_| atomic_replace(&tmp, out));
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    Ok((out.to_path_buf(), thumbs.len(), (sheet.w, sheet.h)))
}

/// The shoot as one JPEG, or pages of them. Rejects never appear.
pub fn contact_sheets(cols: usize, cell: usize, only_picks: bool) -> Result<Vec<SheetPage>, String> {
    let rows: Vec<Value> = store::shots().into_iter().filter(|s| {
        let p = s["pick"].as_str();
        p != Some("reject") && (!only_picks || p == Some("pick"))
    }).collect();
    if rows.is_empty() {
        return Err("nothing to put on the sheet".into());
    }
    let cols = cols.clamp(1, 12);
    let per_page = cols * ROWS_PER_PAGE;
    let pages: Vec<&[Value]> = rows.chunks(per_page).collect();
    let saved = store::load_edits(None);
    let font = sheet_font();
    let outdir = export_dir(None);
    let mut out = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        let fname_ = if pages.len() == 1 { format!("contact-sheet-{}.jpg", page.len()) }
                     else { format!("contact-sheet-{}-of-{}.jpg", i + 1, pages.len()) };
        out.push(render_sheet(page, None, &saved, font.as_ref(), cols, cell, &outdir.join(fname_))?);
    }
    Ok(out)
}
