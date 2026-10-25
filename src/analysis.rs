//! Measuring a headshot: the skin, the eyes, focus, catchlight, the light,
//! and the Auto grade solved from those numbers. Every threshold carries the
//! measurement that set it; see the Python history in README.md.
use serde_json::{json, Map, Value};

use crate::app::round;
use crate::grade::{apply_edits, Phase};
use crate::img::{self, Gray, Rgb};

pub const AUTO_FACE_Y: f64 = 138.0;
pub const AUTO_SPREAD: f64 = 36.0;
pub const AUTO_WARMTH_CAP: f64 = 0.30;
pub const AUTO_SAT_CAP: f64 = 1.30;
pub const SKIN_IN_BOX_WANTED: f64 = 0.70;
pub const SKIN_IN_BOX_FLOOR: f64 = 0.45;
pub const CATCHLIGHT_MIN_PCT: f64 = 0.001;

pub struct Face {
    pub n: usize,
    pub frac: f64,
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub y: f64,
    pub p10: f64,
    pub p90: f64,
    /// Row-major, as the Python scan produced them.
    pub px: Vec<(usize, usize)>,
}

/// Measure the SKIN, not the frame: R > G > B with a real spread, above a
/// floor and below clipping.
pub fn skin_mask_stats(im: &Rgb) -> Option<Face> {
    let (mut rs, mut gs, mut bs) = (0u64, 0u64, 0u64);
    let mut lum = Vec::new();
    let mut idx = Vec::new();
    for y in 0..im.h {
        for x in 0..im.w {
            let [r, g, b] = im.get(x, y);
            if r >= 50 && r > g && g > b && (r as i32 - b as i32) >= 6 && r <= 250 {
                rs += r as u64;
                gs += g as u64;
                bs += b as u64;
                lum.push(0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64);
                idx.push((x, y));
            }
        }
    }
    let n = idx.len();
    if (n as f64) < (im.w * im.h) as f64 * 0.02 || n < 60 {
        return None;
    }
    lum.sort_by(|a, b| a.total_cmp(b));
    let nf = n as f64;
    Some(Face {
        n, frac: nf / (im.w * im.h) as f64,
        r: rs as f64 / nf, g: gs as f64 / nf, b: bs as f64 / nf,
        y: (0.299 * rs as f64 + 0.587 * gs as f64 + 0.114 * bs as f64) / nf,
        p10: lum[(nf * 0.10) as usize], p90: lum[(nf * 0.90) as usize], px: idx,
    })
}

pub struct Stat {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub y: f64,
}

/// Re-measure one FIXED set of pixels: the mask is chosen once and followed.
pub fn stats_at(im: &Rgb, idx: &[(usize, usize)]) -> Option<Stat> {
    if idx.is_empty() {
        return None;
    }
    let (mut rs, mut gs, mut bs) = (0u64, 0u64, 0u64);
    for &(x, y) in idx {
        if x >= im.w || y >= im.h {
            continue;
        }
        let [r, g, b] = im.get(x, y);
        rs += r as u64;
        gs += g as u64;
        bs += b as u64;
    }
    let n = idx.len() as f64;
    Some(Stat { r: rs as f64 / n, g: gs as f64 / n, b: bs as f64 / n,
                y: (0.299 * rs as f64 + 0.587 * gs as f64 + 0.114 * bs as f64) / n })
}

pub fn pct(hist: &[u64; 256], total: u64, p: f64) -> usize {
    let t = total as f64 * p;
    let mut c = 0u64;
    for (v, k) in hist.iter().enumerate() {
        c += k;
        if c as f64 >= t {
            return v;
        }
    }
    255
}

fn skin_grid(face: &Face, w: usize, h: usize) -> Vec<bool> {
    let mut g = vec![false; w * h];
    for &(x, y) in &face.px {
        g[y * w + x] = true;
    }
    g
}

fn head_geometry(face: &Face) -> (usize, usize, usize) {
    let mut ys: Vec<usize> = face.px.iter().map(|p| p.1).collect();
    ys.sort_unstable();
    let top = ys[(ys.len() as f64 * 0.02) as usize];
    let bot = ys[(ys.len() as f64 * 0.98) as usize];
    let head = top as f64 + (bot as f64 - top as f64) * 0.40;
    let mut xs: Vec<usize> = face.px.iter().filter(|p| p.1 as f64 <= head).map(|p| p.0).collect();
    if xs.is_empty() {
        xs = face.px.iter().map(|p| p.0).collect();
    }
    xs.sort_unstable();
    (top, bot, xs[xs.len() / 2])
}

/// The brows-to-lower-lid band on the working copy: 10-28% of the skin's
/// height, centred on the head. KNOWN LIMIT: bare shoulders stretch it.
pub fn eye_band(small: &Rgb, face: &Face) -> (i64, i64, i64, i64) {
    let (top, bot, cx) = head_geometry(face);
    let span = 8.max(((bot - top) as f64 * 0.30) as i64);
    let (t, b) = (top as f64, bot as f64);
    ((cx as i64 - span).max(0), ((t + (b - t) * 0.10) as i64).max(0),
     (cx as i64 + span).min(small.w as i64), ((t + (b - t) * 0.28) as i64).min(small.h as i64))
}

/// (percent of the eye band, pixel count): small, neutral, brighter than
/// any skin, counted at full resolution.
pub fn catchlight(full: &Rgb, small: &Rgb, face: &Face, bx: (i64, i64, i64, i64)) -> (f64, u64) {
    let k = full.w as f64 / small.w as f64;
    let band = img::crop(full, (bx.0 as f64 * k) as i64, (bx.1 as f64 * k) as i64,
                         (bx.2 as f64 * k) as i64, (bx.3 as f64 * k) as i64);
    let mut lum: Vec<f64> = face.px.iter().step_by(7).map(|&(x, y)| {
        let [r, g, b] = small.get(x, y);
        0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64
    }).collect();
    lum.sort_by(|a, b| a.total_cmp(b));
    let at = if lum.is_empty() { 235.0 } else { (lum[(lum.len() as f64 * 0.99) as usize] + 15.0).min(250.0) };
    let l = img::to_gray(&band);
    let s = img::hsv_saturation(&band);
    let n = l.px.iter().zip(&s.px).filter(|(&v, &sat)| v as f64 >= at && sat <= 31).count() as u64;
    let total = (band.w * band.h).max(1) as f64;
    (n as f64 / total * 100.0, n)
}

/// Score one frame on the faults a photographer bins it for: focus, exposure,
/// eyes. None when there is no face to measure.
pub fn frame_quality(im: &Rgb) -> Option<Value> {
    let small = img::thumbnail(im, 640, 640);
    let face = skin_mask_stats(&small)?;
    let (sw, sh) = (small.w, small.h);
    let (top, bot, cx) = head_geometry(&face);
    let fx = cx as f64 / sw as f64;
    let fy = (top as f64 + (bot as f64 - top as f64) * 0.18) / sh as f64;
    let skin = skin_grid(&face, sw, sh);
    let sbw = 6.max((sw as f64 * 0.17) as usize);
    let sbh = 4.max((sh as f64 * 0.06) as usize);
    let step = 2.max(sbh / 3);
    let (mut best_frac, mut best_y) = (-1.0f64, (fy * sh as f64) as usize);
    let mut y_try = (fy * sh as f64) as usize;
    for _ in 0..22 {
        if y_try + sbh > sh {
            break;
        }
        let sx = ((fx * sw as f64) as i64 - (sbw / 2) as i64).clamp(0, (sw - sbw.min(sw)) as i64) as usize;
        let mut n = 0usize;
        for yy in y_try..y_try + sbh {
            for xx in sx..(sx + sbw).min(sw) {
                if skin[yy * sw + xx] {
                    n += 1;
                }
            }
        }
        let frac = n as f64 / (sbw * sbh) as f64;
        if frac > best_frac {
            best_frac = frac;
            best_y = y_try;
        }
        if frac >= SKIN_IN_BOX_WANTED {
            break;
        }
        y_try += step;
    }
    let _ = best_y;
    if best_frac < SKIN_IN_BOX_FLOOR {
        return None;
    }
    let band = eye_band(&small, &face);
    let k = im.w as f64 / sw as f64;
    let (x0, y0) = ((band.0 as f64 * k) as i64, (band.1 as f64 * k) as i64);
    let bw = 24.max(((band.2 - band.0) as f64 * k) as i64);
    let bh = 24.max(((band.3 - band.1) as f64 * k) as i64);
    let eye: Gray = img::to_gray(&img::crop(im, x0, y0, x0 + bw, y0 + bh));
    let mean = img::mean_gray(&eye);
    if mean < 8.0 {
        return None;
    }
    let eye = img::enhance_brightness_gray(&eye, (128.0 / mean) as f32);
    let lap = img::filter3x3_gray(&eye, [0., 1., 0., 1., -4., 1., 0., 1., 0.], 1.0, 0.0);
    let focus = img::stddev_gray(&lap).powi(2);
    let gl = img::histogram(&img::to_gray(&small));
    let tot: u64 = gl.iter().sum::<u64>().max(1);
    let clipped = gl[252..].iter().sum::<u64>() as f64 / tot as f64 * 100.0;
    let (catch, _) = catchlight(im, &small, &face, band);
    Some(json!({"focus": round(focus, 1), "skinInBox": round(best_frac, 3), "faceY": round(face.y, 1),
                "clipped": round(clipped, 3), "catchlight": round(catch, 4),
                "eyeBox": [x0, y0, bw, bh]}))
}

/// Rank already-scored frames, worst fault first. Focus is judged against
/// the set's 90th percentile, never an absolute.
pub fn rank_scored(scored: Vec<Value>) -> Vec<Value> {
    let (mut good, bad): (Vec<Value>, Vec<Value>) = scored.into_iter()
        .partition(|x| x.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
    if good.is_empty() {
        return bad;
    }
    let mut foc: Vec<f64> = good.iter().map(|x| x["focus"].as_f64().unwrap_or(0.0)).collect();
    foc.sort_by(|a, b| a.total_cmp(b));
    let best = if foc.len() >= 5 { foc[(foc.len() - 1).min((foc.len() as f64 * 0.9) as usize)] }
               else { foc[foc.len() - 1] };
    let best = if best == 0.0 { 1.0 } else { best };
    for x in good.iter_mut() {
        let focus_pct = ((x["focus"].as_f64().unwrap_or(0.0) / best * 100.0).round_ties_even() as i64).min(100);
        let mut faults: Vec<(&str, f64)> = Vec::new();
        if focus_pct < 50 {
            faults.push(("soft", (100 - focus_pct) as f64));
        }
        let clipped = x["clipped"].as_f64().unwrap_or(0.0);
        if clipped > 0.5 {
            faults.push(("blown highlights", 40.0 + clipped));
        }
        let off = (x["faceY"].as_f64().unwrap_or(0.0) - 138.0).abs() / 138.0 * 100.0;
        if off > 25.0 {
            faults.push(("badly exposed", off));
        }
        if x["catchlight"].as_f64().unwrap_or(0.0) < CATCHLIGHT_MIN_PCT {
            faults.push(("no light in the eyes", 20.0));
        }
        let penalty = faults.iter().map(|f| f.1).fold(0.0, f64::max);
        x["focusPct"] = json!(focus_pct);
        x["faults"] = json!(faults.iter().map(|f| f.0).collect::<Vec<_>>());
        x["penalty"] = json!(round(penalty, 1));
    }
    good.sort_by(|a, b| {
        let pa = a["penalty"].as_f64().unwrap_or(0.0);
        let pb = b["penalty"].as_f64().unwrap_or(0.0);
        pa.total_cmp(&pb).then(b["focusPct"].as_i64().cmp(&a["focusPct"].as_i64()))
    });
    good.extend(bad);
    good
}

// ------------------------------------------------------------------ lighting
struct BandM {
    y: f64,
    skin_only: bool,
}

/// Measure the lighting and say, in words, what to change.
pub fn lighting_report(im: &Rgb) -> Value {
    let small = img::thumbnail(im, 640, 640);
    let (w, h) = (small.w, small.h);
    let Some(face) = skin_mask_stats(&small) else {
        return json!({"ok": false, "error": "no face found in this frame, nothing to measure"});
    };
    let (top, bot, cx) = head_geometry(&face);
    let span = 8.max(((bot - top) as f64 * 0.30) as i64);
    let skin = skin_grid(&face, w, h);
    let band = |y0: f64, y1: f64| -> Option<BandM> {
        let x0 = (cx as i64 - span).max(0);
        let x1 = (cx as i64 + span).min(w as i64);
        let (y0, y1) = ((y0 as i64).max(0), (y1 as i64).min(h as i64));
        if y1 <= y0 || x1 <= x0 {
            return None;
        }
        let (mut rs, mut gs, mut bs, mut n) = (0u64, 0u64, 0u64, 0u64);
        for yy in y0..y1 {
            for xx in x0..x1 {
                let (xu, yu) = (xx as usize, yy as usize);
                if !skin[yu * w + xu] {
                    continue;
                }
                let [r, g, b] = small.get(xu, yu);
                rs += r as u64;
                gs += g as u64;
                bs += b as u64;
                n += 1;
            }
        }
        // Too little skin in the band to say anything: fall back to the
        // rectangle rather than invent a number, and flag it skin_only=false.
        if n < 40 {
            let m = img::mean_rgb(&img::crop(&small, x0, y0, x1, y1));
            return Some(BandM { y: 0.299 * m[0] + 0.587 * m[1] + 0.114 * m[2], skin_only: false });
        }
        let nf = n as f64;
        Some(BandM { y: (0.299 * rs as f64 + 0.587 * gs as f64 + 0.114 * bs as f64) / nf, skin_only: true })
    };
    let (t, b) = (top as f64, bot as f64);
    // Eyes in the upper third of the head, cheeks below, both relative to
    // the skin mask: a fixed box lands on an eyelash in one frame and a
    // forehead in the next, which reported 13 of 23 sharp frames soft.
    let eyes = band(t + (b - t) * 0.13, t + (b - t) * 0.24);
    let cheeks = band(t + (b - t) * 0.33, t + (b - t) * 0.50);
    let gl = img::histogram(&img::to_gray(&small));
    let tot: u64 = gl.iter().sum::<u64>().max(1);
    let (p1, p99) = (pct(&gl, tot, 0.01), pct(&gl, tot, 0.99));
    // A catchlight is a tiny specular highlight, so it is COUNTED as
    // near-white pixels in a taller eye band, never averaged.
    let (spec, spec_px) = if eyes.is_some() { catchlight(im, &small, &face, eye_band(&small, &face)) }
                          else { (0.0, 0) };
    // Signed: on a crop narrower than the corner box the left edge goes
    // negative, and the crop fills the outside with black as Pillow's did.
    let cs = 6.max(w.min(h) / 8) as i64;
    let wi = w as i64;
    let corner: Vec<f64> = [(0, 0, cs, cs), (wi - cs, 0, wi, cs)].iter().map(|&(a, b2, c, d)| {
        let m = img::mean_rgb(&img::crop(&small, a, b2, c, d));
        0.299 * m[0] + 0.587 * m[1] + 0.114 * m[2]
    }).collect();
    let backdrop = corner.iter().sum::<f64>() / corner.len() as f64;
    let eye_vs = match (&eyes, &cheeks) { (Some(e), Some(c)) => Some(round(e.y - c.y, 1)), _ => None };
    let range_used = ((p99 as f64 - p1 as f64) / 255.0 * 100.0).round_ties_even();
    let separation = cheeks.as_ref().map(|c| round(c.y - backdrop, 1));
    let lsr = round(face.p90 / face.p10.max(1.0), 2);
    let m = json!({
        "faceY": cheeks.as_ref().map(|c| round(c.y, 1)),
        "eyeY": eyes.as_ref().map(|e| round(e.y, 1)),
        "eyeVsCheek": eye_vs,
        "catchlightPct": round(spec, 4),
        "catchlightPx": spec_px,
        "rangeUsed": range_used as i64,
        "p1": p1, "p99": p99,
        "backdropY": round(backdrop, 1),
        "separation": separation,
        "litShadowRatio": lsr,
        "skinSpread": round(face.r - face.b, 1),
        "skinOnlyBands": matches!((&eyes, &cheeks), (Some(e), Some(c)) if e.skin_only && c.skin_only),
    });
    let mut notes = Vec::new();
    // 1. The overhead-key signature: a brow ridge lit from straight above
    //    shadows its own eye socket, and eye colour is the first casualty.
    if let Some(ev) = eye_vs.filter(|v| *v < -12.0) {
        notes.push(json!({"level": "bad",
            "what": format!("Your eyes sit {:.0} levels darker than your cheeks.", ev.abs()),
            "why": "A key light directly above you shadows your own brow ridge into your eye sockets, which is why eye colour does not read.",
            "fix": "Hold a sheet of white card at chest height angled up at your face, or lower the key to about your eye line and move it in front of you rather than over you."}));
    }
    // 2. No catchlight. The 2026-09-21 casting export measured 0.002%.
    if round(spec, 4) < CATCHLIGHT_MIN_PCT {
        notes.push(json!({"level": "bad",
            "what": format!("There is no catchlight in your eyes ({spec_px} bright neutral pixels found across both eyes)."),
            "why": "A catchlight is the reflection of the light in the wet of the eye. It is the single thing that makes eyes look alive, and a source directly overhead does not put one there.",
            "fix": "Bring the key round in front of you, roughly eye level and angled down about 25 degrees."}));
    }
    // 3. Flatness as a share of the available range, not a contrast opinion:
    //    below about 75% the picture reads muddy on any screen.
    if range_used < 75.0 {
        notes.push(json!({"level": "warn",
            "what": format!("The frame only uses {}% of the available range ({p1} to {p99} of 0 to 255).", range_used as i64),
            "why": "Nothing in the picture is properly bright or properly dark, which is what reads as bland. One large soft source and nothing else does this.",
            "fix": "Add separation: a second small light on the backdrop, or a rim light behind you. Grading cannot invent a highlight the scene does not contain."}));
    }
    // 4. Separation: a face that matches its backdrop has nothing to stand
    //    out against, whatever the grade.
    if let Some(sep) = separation.filter(|s| *s < 40.0) {
        notes.push(json!({"level": "warn",
            "what": format!("Your face is only {sep:.0} levels brighter than the backdrop."),
            "why": "A headshot separates the subject from the background by light, not by grading.",
            "fix": "Step two or three feet further from the backdrop so it falls off darker."}));
    }
    // 5. The light's shape. Both ends are faults, in opposite directions: a
    //    portrait usually wants a lit-to-shadow ratio of 2.0 to 3.0.
    if lsr < 1.7 {
        notes.push(json!({"level": "warn",
            "what": format!("The light is very flat (lit to shadow ratio {lsr:.2})."),
            "why": "Flat light gives a face no shape. A portrait usually wants 2.0 to 3.0.",
            "fix": "Move the key further to one side, or pull your fill back."}));
    } else if lsr > 4.0 {
        notes.push(json!({"level": "warn",
            "what": format!("The light is very contrasty (lit to shadow ratio {lsr:.2})."),
            "why": "The shadow side of the face is losing detail.",
            "fix": "Add fill on the shadow side, or bring the key more to the front."}));
    }
    if notes.is_empty() {
        notes.push(json!({"level": "good", "what": "The lighting measures well.",
            "why": "Eyes lit, a catchlight present, the range used, and the subject separated from the backdrop.",
            "fix": ""}));
    }
    json!({"ok": true, "measured": m, "notes": notes})
}

// ------------------------------------------------------------------ auto look
fn adj_value(adj: &Map<String, Value>) -> Value {
    Value::Object(adj.clone())
}

fn getf(adj: &Map<String, Value>, k: &str) -> f64 {
    adj.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

/// Derive a look from what the image measures. Every value is solved from a
/// measurement and clamped; the reasons come back with it.
pub fn auto_look(im: &Rgb) -> (Map<String, Value>, Vec<String>, Value) {
    let small = img::thumbnail(im, 420, 420);
    let (w, h) = (small.w, small.h);
    let mut src = "skin";
    let face = match skin_mask_stats(&small) {
        Some(f) => f,
        None => {
            let (bx0, by0) = ((w as f64 * 0.28) as usize, (h as f64 * 0.14) as usize);
            let (bx1, by1) = ((w as f64 * 0.72) as usize, (h as f64 * 0.56) as usize);
            let bx = img::crop(&small, bx0 as i64, by0 as i64, bx1 as i64, by1 as i64);
            let m = img::mean_rgb(&bx);
            let g2 = img::histogram(&img::to_gray(&bx));
            let tot: u64 = g2.iter().sum::<u64>().max(1);
            let mut px = Vec::new();
            let mut yy = by0;
            while yy < by1 {
                let mut xx = bx0;
                while xx < bx1 {
                    px.push((xx, yy));
                    xx += 3;
                }
                yy += 3;
            }
            src = "centre region (no skin tones found)";
            Face { n: 0, frac: 0.0, r: m[0], g: m[1], b: m[2],
                   y: 0.299 * m[0] + 0.587 * m[1] + 0.114 * m[2],
                   p10: pct(&g2, tot, 0.10) as f64, p90: pct(&g2, tot, 0.90) as f64, px }
        }
    };
    let gl = img::histogram(&img::to_gray(&small));
    let tot: u64 = gl.iter().sum::<u64>().max(1);
    let (fp1, fp99) = (pct(&gl, tot, 0.01), pct(&gl, tot, 0.99));
    let mut why: Vec<String> = Vec::new();
    let mut adj = Map::new();
    for (k, v) in [("exposure", 1.0), ("shadows", 0.0), ("highlights", 0.0), ("contrast", 1.0),
                   ("saturation", 1.0), ("warmth", 0.0), ("tint", 0.0), ("vignette", 0.0),
                   ("sharpness", 1.0)] {
        adj.insert(k.into(), json!(v));
    }
    // 1. Exposure from the face, held back so nothing blows out.
    let fy = face.y.max(1.0);
    let mut exp = AUTO_FACE_Y / fy;
    let ceiling = 248.0 / (fp99 as f64).max(1.0);
    if exp > ceiling && ceiling > 0.8 {
        why.push(format!("exposure held at {:.2} instead of {exp:.2} to keep the highlights off 255",
                         exp.min(ceiling)));
        exp = ceiling;
    }
    exp = exp.clamp(0.70, 2.20);
    if (exp - 1.0).abs() >= 0.02 {
        adj.insert("exposure".into(), json!(round(exp, 3)));
        why.push(format!("face measured {fy:.0}, wants about {AUTO_FACE_Y:.0}: {:+.2} stops", exp.log2()));
    }
    // 2. White balance, solved directly.
    let spread = face.r - face.b;
    let denom = 0.18 * (face.r + face.b).max(1.0);
    let warmth = ((AUTO_SPREAD - spread) / denom).clamp(-AUTO_WARMTH_CAP, AUTO_WARMTH_CAP);
    if warmth.abs() >= 0.02 {
        adj.insert("warmth".into(), json!(round(warmth, 3)));
        why.push(format!("skin red-blue spread {spread:+.0}, wants about +{AUTO_SPREAD:.0}: warmth {warmth:+.2}"));
    }
    let reached = spread + warmth * denom;
    if reached < AUTO_SPREAD - 1.0 && reached > 1.0 {
        let sat = (AUTO_SPREAD / reached).clamp(1.0, AUTO_SAT_CAP);
        if sat > 1.01 {
            adj.insert("saturation".into(), json!(round(sat, 3)));
            why.push(format!("white balance alone reaches {reached:+.0}: saturation {sat:.2} for the rest, \
                              which leaves a neutral backdrop neutral"));
        }
    }
    // 3. Green/magenta.
    let g_excess = face.g - (face.r + face.b) / 2.0;
    let tint = ((g_excess + 2.0) / (0.15 * face.g.max(1.0))).clamp(-0.35, 0.35);
    if tint.abs() >= 0.02 {
        adj.insert("tint".into(), json!(round(tint, 3)));
        why.push(format!("green sits {g_excess:+.1} above the red/blue average: tint {tint:+.2}"));
    }
    // 4. Contrast from the range the file uses, only ever added.
    let span = (fp99 as i64 - fp1 as i64).max(1);
    if span < 200 {
        let c = (200.0 / span as f64).clamp(1.0, 1.22);
        if c > 1.01 {
            adj.insert("contrast".into(), json!(round(c, 3)));
            why.push(format!("the file uses only {fp1}..{fp99} of 0..255: contrast {c:.2}"));
        }
    }
    // 5. Shadows from the face's own dark side.
    if face.p10 < face.y * 0.55 {
        let lift = ((face.y * 0.55 - face.p10) / face.y.max(1.0)).min(0.28);
        if lift >= 0.03 {
            adj.insert("shadows".into(), json!(round(lift, 3)));
            why.push(format!("the shadow side of the face reads {:.0} against a mean of {:.0}: shadows +{lift:.2}",
                             face.p10, face.y));
        }
    }
    // 6. Highlights, measured after exposure.
    let top = fp99 as f64 * getf(&adj, "exposure");
    if top > 235.0 {
        let hh = -((top - 235.0) / 255.0 * 2.0).min(0.35);
        if hh.abs() >= 0.02 {
            adj.insert("highlights".into(), json!(round(hh, 3)));
            why.push(format!("brightening puts the top of the histogram at {top:.0}: highlights {hh:+.2}"));
        }
    }
    // 7. Taste, named as such.
    adj.insert("sharpness".into(), json!(1.15));
    adj.insert("vignette".into(), json!(0.10));
    // 8. Close the loop: render the look, measure the face, correct.
    if getf(&adj, "contrast") > 1.01 && getf(&adj, "shadows") < 0.10 {
        let s = round(((getf(&adj, "contrast") - 1.0) * 0.7).min(0.20), 3);
        adj.insert("shadows".into(), json!(s));
        why.push(format!("added contrast would crush the low end: shadows +{s:.2} to hold it open"));
    }
    let src_burnt = gl[251..].iter().sum::<u64>() as f64 / tot as f64;
    for _ in 0..3 {
        let looked = apply_edits(&small, &adj_value(&adj), Phase::All);
        let Some(got) = stats_at(&looked, &face.px) else { break };
        if (AUTO_FACE_Y - got.y).abs() < 3.0 {
            break;
        }
        let lh = img::histogram(&img::to_gray(&looked));
        let lt: u64 = lh.iter().sum::<u64>().max(1);
        let burnt = lh[251..].iter().sum::<u64>() as f64 / lt as f64;
        if burnt > 0.002 {
            why.push(format!("stopped brightening at {:.2}: {:.1}% of the frame is already at the top",
                             getf(&adj, "exposure"), burnt * 100.0));
            break;
        }
        let scaled = (getf(&adj, "exposure") * (AUTO_FACE_Y / got.y.max(1.0))).clamp(0.70, 2.20);
        if scaled > getf(&adj, "exposure") && src_burnt > 0.01 {
            why.push(format!("{:.0}% of the file is already clipped: not brightening past {:.2}",
                             src_burnt * 100.0, getf(&adj, "exposure")));
            break;
        }
        if (scaled - getf(&adj, "exposure")).abs() < 0.005 {
            break;
        }
        adj.insert("exposure".into(), json!(round(scaled, 3)));
        why.push(format!("the whole look delivered {:.0} rather than {AUTO_FACE_Y:.0}: exposure corrected to {:.2}",
                         got.y, getf(&adj, "exposure")));
    }
    // Colour, measured on the finished look, on real skin only.
    if src == "skin" {
        for _ in 0..4 {
            let Some(got) = stats_at(&apply_edits(&small, &adj_value(&adj), Phase::All), &face.px) else { break };
            let now_spread = got.r - got.b;
            let miss = now_spread - AUTO_SPREAD;
            if miss.abs() < 1.5 {
                break;
            }
            let denom_now = 0.18 * (got.r + got.b).max(1.0);
            let want_w = getf(&adj, "warmth") - miss / denom_now;
            let capped = want_w.clamp(-AUTO_WARMTH_CAP, AUTO_WARMTH_CAP);
            if (capped - getf(&adj, "warmth")).abs() >= 0.005 {
                adj.insert("warmth".into(), json!(round(capped, 3)));
                why.push(format!("the finished look put skin at {now_spread:+.0} red-blue, wants +{AUTO_SPREAD:.0}: warmth {:+.2}",
                                 getf(&adj, "warmth")));
                continue;
            }
            if miss > 0.0 && getf(&adj, "saturation") > 0.85 {
                let s = round((getf(&adj, "saturation") * AUTO_SPREAD / now_spread).max(0.85), 3);
                adj.insert("saturation".into(), json!(s));
                why.push(format!("warmth is at its limit and skin still reads {now_spread:+.0}: saturation {s:.2}"));
                continue;
            }
            break;
        }
    }
    let measured = json!({"source": src, "faceY": round(face.y, 1), "spread": round(spread, 1),
                          "skinFraction": round(face.frac, 3), "p1": fp1, "p99": fp99});
    let _ = face.n;
    (adj, why, measured)
}
