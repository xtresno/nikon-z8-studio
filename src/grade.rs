//! The grade: exposure, tone, contrast, colour, balance, then vignette and
//! sharpening, in that photographic order, split into per-pixel ("point") and
//! neighbourhood ("spatial") phases so an export can fit between them.
use serde_json::Value;

use crate::app::num;
use crate::img::{self, Filter, Gray, Rgb};

pub const POINT_EDITS: &[&str] = &["exposure", "shadows", "highlights", "contrast", "saturation",
                                   "warmth", "tint"];
pub const SPATIAL_EDITS: &[&str] = &["vignette", "sharpness"];

/// What each slider reads as untouched.
pub const ADJ_DEFAULTS: &[(&str, f64)] = &[
    ("exposure", 1.0), ("contrast", 1.0), ("saturation", 1.0), ("sharpness", 1.0),
    ("shadows", 0.0), ("highlights", 0.0), ("warmth", 0.0), ("tint", 0.0),
    ("vignette", 0.0), ("straighten", 0.0),
];

pub fn adj_default(k: &str) -> Option<f64> {
    ADJ_DEFAULTS.iter().find(|(n, _)| *n == k).map(|(_, v)| *v)
}

/// Where a crop sits when it cannot keep the whole frame (ImageOps.fit
/// centering). Served to the page with the presets: one definition.
pub const ANCHORS: &[(&str, (f64, f64))] = &[
    ("center", (0.5, 0.5)), ("top", (0.5, 0.25)), ("face", (0.5, 0.35)), ("bottom", (0.5, 0.75)),
];

pub fn anchor(name: &str) -> (f64, f64) {
    ANCHORS.iter().find(|(n, _)| *n == name).map(|(_, c)| *c).unwrap_or((0.5, 0.5))
}

/// Lifts shadows and recovers highlights, weighted to one end of the range
/// each, so the midtones where skin lives barely move.
pub fn tone_lut(shadows: f64, highlights: f64) -> [u8; 256] {
    let mut lut = [0u8; 256];
    for (i, o) in lut.iter_mut().enumerate() {
        let mut x = i as f64 / 255.0;
        if shadows != 0.0 {
            x += shadows * 0.45 * (1.0 - x).powf(2.2);
            x = x.clamp(0.0, 1.0);
        }
        if highlights != 0.0 {
            x += highlights * 0.45 * x.powf(2.2);
            x = x.clamp(0.0, 1.0);
        }
        *o = (x * 255.0).round_ties_even().clamp(0.0, 255.0) as u8;
    }
    lut
}

fn scaled_lut(f: f64) -> [u8; 256] {
    let mut lut = [0u8; 256];
    for (p, o) in lut.iter_mut().enumerate() {
        // int() truncates toward zero, then min(255, .)
        *o = ((p as f64 * f).trunc()).clamp(0.0, 255.0) as u8;
    }
    lut
}

const IDENT: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = i as u8;
        i += 1;
    }
    t
};

/// The vignette mask for one size and strength, kept: rebuilding it (a
/// resize to the full frame, then a lookup) cost more than applying it.
/// The gradient is raised to a power so the face stays flat and the falloff
/// lives out near the corners.
fn vignette_mask(w: usize, h: usize, amount: f64) -> std::sync::Arc<Gray> {
    type Masks = Vec<((usize, usize, u64), std::sync::Arc<Gray>)>;
    static MASKS: parking_lot::Mutex<Masks> = parking_lot::Mutex::new(Vec::new());
    let key = (w, h, amount.to_bits());
    if let Some((_, m)) = MASKS.lock().iter().find(|(k, _)| *k == key) {
        return m.clone();
    }
    let mask = img::resize_gray(&img::radial_gradient(), w, h, Filter::Bilinear);
    let mut lut = [0u8; 256];
    for (p, o) in lut.iter_mut().enumerate() {
        let v = 255 - (((p as f64 / 255.0).powf(3.0)) * 255.0 * amount * 0.95) as i64;
        *o = v.clamp(0, 255) as u8;
    }
    let mask = std::sync::Arc::new(img::point_gray(&mask, &lut));
    let mut m = MASKS.lock();
    // A handful of sizes (preview, loupe, each export size) at the few
    // strengths in play; a full-size mask is 45 MB, so keep the list short.
    if m.len() >= 4 {
        m.remove(0);
    }
    m.push((key, mask.clone()));
    mask
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    All,
    Point,
    Spatial,
}

/// The grade. `adj` is the page's adjustment object; anything unreadable
/// falls back to the untouched value, as the Python f() did.
pub fn apply_edits(im: &Rgb, adj: &Value, phase: Phase) -> Rgb {
    apply_edits_owned(im.clone(), adj, phase)
}

/// The grade on a frame the caller no longer needs: no copy up front. At
/// 45 MP a copy is 136 MB, and the export path used to hold the caller's
/// frame and the clone at once.
pub fn apply_edits_owned(im: Rgb, adj: &Value, phase: Phase) -> Rgb {
    let f = |k: &str, d: f64| num(adj.get(k), d);
    let point = phase != Phase::Spatial;
    let spatial = phase != Phase::Point;
    let mut im = im;
    if point {
        let exposure = f("exposure", 1.0);
        let (sh, hi) = (f("shadows", 0.0), f("highlights", 0.0));
        let contrast = f("contrast", 1.0);
        let saturation = f("saturation", 1.0);
        let (warmth, tint) = (f("warmth", 0.0), f("tint", 0.0));
        if exposure != 1.0 || sh != 0.0 || hi != 0.0 || contrast != 1.0
            || saturation != 1.0 || warmth != 0.0 || tint != 0.0 {
            // Compose byte lookups, keeping the rounding of EACH old step.
            // Contrast needs the mean after exposure/tone, so measure through
            // that lookup without allocating a 45 MP intermediate or gray copy.
            let tone = if sh != 0.0 || hi != 0.0 { tone_lut(sh, hi) } else { IDENT };
            let mut point = IDENT;
            for (v, out) in point.iter_mut().enumerate() {
                *out = tone[img::blend1(0, v as u8, exposure as f32) as usize];
            }
            if contrast != 1.0 {
                // An integer sum, so splitting it across cores changes nothing.
                use rayon::prelude::*;
                let sum: u64 = im.px.par_chunks(3 * 4096).map(|c| c.chunks_exact(3).map(|p|
                    img::luma(point[p[0] as usize], point[p[1] as usize], point[p[2] as usize]) as u64)
                    .sum::<u64>()).sum();
                let mean = (sum as f64 / (im.px.len() / 3).max(1) as f64 + 0.5) as u8;
                for v in &mut point { *v = img::blend1(mean, *v, contrast as f32); }
            }
            let r = if warmth != 0.0 { scaled_lut(1.0 + warmth * 0.18) } else { IDENT };
            let b = if warmth != 0.0 { scaled_lut(1.0 - warmth * 0.18) } else { IDENT };
            let g = if tint != 0.0 { scaled_lut(1.0 - tint * 0.15) } else { IDENT };
            // Saturation's blend as a table of blend1 itself: same bytes, no
            // float per channel. Rows run on every core.
            let sat = (saturation != 1.0).then(|| img::blend_table(saturation as f32));
            use rayon::prelude::*;
            im.px.par_chunks_mut(3 * 4096).for_each(|chunk| {
                for p in chunk.chunks_exact_mut(3) {
                    let (mut rr, mut gg, mut bb) = (point[p[0] as usize], point[p[1] as usize], point[p[2] as usize]);
                    if let Some(t) = &sat {
                        let l = (img::luma(rr, gg, bb) as usize) << 8;
                        rr = t[l | rr as usize];
                        gg = t[l | gg as usize];
                        bb = t[l | bb as usize];
                    }
                    p[0] = r[rr as usize]; p[1] = g[gg as usize]; p[2] = b[bb as usize];
                }
            });
        }
    }
    if spatial {
        let v = f("vignette", 0.0);
        if v != 0.0 {
            let mask = vignette_mask(im.w, im.h, v);
            im = img::darken_by_mask_owned(im, &mask);
        }
        let v = f("sharpness", 1.0);
        if v != 1.0 {
            im = img::enhance_sharpness(&im, v as f32);
        }
    }
    im
}

/// Level a tilted frame, then cut away the wedges the rotation leaves.
pub fn straighten(im: Rgb, degrees: f64) -> Rgb {
    if degrees == 0.0 || !degrees.is_finite() {
        return im;
    }
    let a = degrees.abs().to_radians();
    let (w, h) = (im.w as f64, im.h as f64);
    let out = img::rotate(&im, -degrees);
    let (ca, sa) = (a.cos(), a.sin());
    if w <= 0.0 || h <= 0.0 || (ca - sa) <= 0.0 {
        return out;
    }
    let denom = ca * ca - sa * sa;
    if denom <= 1e-6 {
        return out;
    }
    let nw = ((w * ca - h * sa) / denom).clamp(1.0, w);
    let nh = ((h * ca - w * sa) / denom).clamp(1.0, h);
    let (left, top) = ((w - nw) / 2.0, (h - nh) / 2.0);
    let r = |v: f64| v.round_ties_even() as i64;
    img::crop(&out, r(left), r(top), r(left + nw), r(top + nh))
}

/// Crop from normalised 0..1 coordinates.
pub fn apply_crop(im: Rgb, crop: Option<&Value>) -> Rgb {
    let Some(c) = crop.filter(|c| c.is_object()) else { return im };
    let get = |k: &str| c.get(k).and_then(|v| match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    });
    let (Some(x), Some(y), Some(cw), Some(ch)) = (get("x"), get("y"), get("w"), get("h")) else {
        return im;
    };
    let (w, h) = (im.w as i64, im.h as i64);
    let r = |v: f64| v.round_ties_even() as i64;
    let left = r(x * w as f64).clamp(0, w - 1);
    let top = r(y * h as f64).clamp(0, h - 1);
    let right = r((x + cw) * w as f64).min(w).max(left + 1);
    let bottom = r((y + ch) * h as f64).min(h).max(top + 1);
    img::crop(&im, left, top, right, bottom)
}

pub fn fit_to(im: Rgb, w: usize, h: usize, mode: &str, anchor_name: &str) -> Rgb {
    if w == 0 || h == 0 {
        return im;
    }
    if mode == "contain" {
        return img::thumbnail(&im, w, h);
    }
    img::fit_cover(&im, w, h, anchor(anchor_name))
}

#[cfg(test)]
mod point_parity_tests {
    use super::*;
    use serde_json::json;

    // Independent reference: the previous sequence of image operations,
    // retaining its intermediate 8-bit rounding and contrast mean.
    fn separate_steps(base: &Rgb, a: &Value) -> Rgb {
        let f = |k, d| num(a.get(k), d);
        let mut im = base.clone();
        if f("exposure", 1.0) != 1.0 { im = img::enhance_brightness(&im, f("exposure", 1.0) as f32); }
        let (sh, hi) = (f("shadows", 0.0), f("highlights", 0.0));
        if sh != 0.0 || hi != 0.0 { let l = tone_lut(sh, hi); im = img::point3(&im, &l, &l, &l); }
        if f("contrast", 1.0) != 1.0 { im = img::enhance_contrast(&im, f("contrast", 1.0) as f32); }
        if f("saturation", 1.0) != 1.0 { im = img::enhance_color(&im, f("saturation", 1.0) as f32); }
        let (w, t) = (f("warmth", 0.0), f("tint", 0.0));
        if w != 0.0 || t != 0.0 {
            let r = if w != 0.0 { scaled_lut(1.0 + w * 0.18) } else { IDENT };
            let b = if w != 0.0 { scaled_lut(1.0 - w * 0.18) } else { IDENT };
            let g = if t != 0.0 { scaled_lut(1.0 - t * 0.15) } else { IDENT };
            im = img::point3(&im, &r, &g, &b);
        }
        im
    }

    #[test]
    fn fused_point_grade_is_byte_identical_to_separate_steps() {
        let presets: Value = serde_json::from_str(include_str!("../presets.json")).unwrap();
        let mut cases = presets["look"].as_array().unwrap().clone();
        cases.push(json!({}));
        for (k, lo, hi) in [("exposure", 0.0, 3.0), ("contrast", 0.0, 3.0), ("saturation", 0.0, 3.0),
            ("shadows", -1.0, 1.0), ("highlights", -1.0, 1.0), ("warmth", -1.0, 1.0), ("tint", -1.0, 1.0)] {
            for v in [lo, hi] { cases.push(json!({k: v})); }
        }
        let mut rng = 91u32;
        let mut next = || { rng = rng.wrapping_mul(1664525).wrapping_add(1013904223); (rng >> 24) as u8 };
        for _ in 0..80 {
            cases.push(json!({"exposure": next() as f64 / 85.0, "contrast": next() as f64 / 85.0,
                "saturation": next() as f64 / 85.0, "shadows": next() as f64 / 127.5 - 1.0,
                "highlights": next() as f64 / 127.5 - 1.0, "warmth": next() as f64 / 127.5 - 1.0,
                "tint": next() as f64 / 127.5 - 1.0}));
        }
        for (w, h) in [(0, 0), (1, 1), (7, 5), (257, 193)] {
            let im = Rgb { w, h, px: (0..w*h*3).map(|_| next()).collect() };
            for a in &cases {
                let expected = separate_steps(&im, a);
                let got = apply_edits(&im, a, Phase::Point);
                let diff = got.px.iter().zip(&expected.px).position(|(a, b)| a != b);
                assert_eq!(diff, None, "{w}x{h}, look {a}");
                assert_eq!(got.px.len(), expected.px.len());
            }
        }
    }
}
