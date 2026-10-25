//! The webcam frame path: decode, fill 16:9, grade, luma vignette, box
//! unsharp, 2x2 chroma, limited-range I420. Measured live on the Z8 at
//! 3.6 ms a frame on four threads against 10-13 ms in Python; its output
//! matches the Python keeper within 4 levels.
use rayon::prelude::*;

pub const W: usize = 1024;
pub const H: usize = 576;
pub const OUT_LEN: usize = W * H * 3 / 2;

#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub exposure: f64,
    pub contrast: f64,
    pub saturation: f64,
    pub warmth: f64,
    pub vignette: f64,
    pub sharpness: f64,
}

impl Look {
    pub fn neutral() -> Self {
        Look { exposure: 1.0, contrast: 1.0, saturation: 1.0, warmth: 0.0,
               vignette: 0.0, sharpness: 1.0 }
    }
}

pub struct Grader {
    lut: [[u8; 256]; 3],
    y_lut: [u8; 256],
    c_lut: [u8; 256],
    vig: Option<Vec<u8>>,
    sharp_amt: f32,
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 { v.max(lo).min(hi) }

impl Grader {
    /// Mirrors DirectWriter._build.
    pub fn new(look: &Look) -> Self {
        let (exp, con, sat) = (look.exposure, look.contrast, look.saturation);
        let br = clamp((exp - 1.0) * 0.45, -1.0, 1.0);
        let w = clamp(look.warmth, -1.0, 1.0);
        let mut lut = [[0u8; 256]; 3];
        for (c, off) in [w * 0.25, 0.0, -w * 0.25].iter().enumerate() {
            for x in 0..256 {
                let mut v = x as f64 / 255.0;
                v = (v + br - 0.5) * con + 0.5;
                v += off * 0.5 * 4.0 * v * (1.0 - v);
                // int() in Python truncates toward zero, as `as i64` does.
                lut[c][x] = ((v * 255.0 + 0.5) as i64).clamp(0, 255) as u8;
            }
        }
        let mut y_lut = [0u8; 256];
        let mut c_lut = [0u8; 256];
        for v in 0..256 {
            let f = v as f64;
            y_lut[v] = (16.0 + (f * 219.0 / 255.0).round_ties_even()) as u8;
            c_lut[v] = (128.0 + (16.0 + f * 224.0 / 255.0 - 128.0) * sat)
                .round_ties_even().clamp(16.0, 240.0) as u8;
        }
        let a = (look.vignette * 1.1).min(1.2);
        let vig = if a > 0.0 {
            // PIL's radial_gradient("L") (256x256, value sqrt(2 * d^2): the
            // 2.0 sits INSIDE the square root in Fill.c, so the edge midpoint
            // is 181, not 255) stretched to W x H, evaluated directly.
            let mut m = vec![0u8; W * H];
            for y in 0..H {
                let sy = (y as f64 + 0.5) * 256.0 / H as f64 - 0.5 - 128.0;
                for x in 0..W {
                    let sx = (x as f64 + 0.5) * 256.0 / W as f64 - 0.5 - 128.0;
                    let p = ((sx * sx + sy * sy) * 2.0).sqrt().min(255.0);
                    let v = 255.0 * (1.0 - a * 0.55 * (p / 255.0).powi(2));
                    m[y * W + x] = (v as i64).clamp(0, 255) as u8;
                }
            }
            Some(m)
        } else {
            None
        };
        let sharp_amt = clamp((look.sharpness - 1.0) * 1.2, 0.0, 2.0) as f32;
        Grader { lut, y_lut, c_lut, vig, sharp_amt }
    }
}

// ------------------------------------------------------------------ frame
/// Scratch planes, allocated once.
pub struct Frame {
    y: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
    tmp: Vec<u8>,
    scaled: Vec<u8>,
    pub out: Vec<u8>,
}

impl Default for Frame {
    fn default() -> Self {
        Frame { y: vec![0; W * H], cb: vec![0; W * H], cr: vec![0; W * H],
                tmp: vec![0; W * H], scaled: Vec::new(), out: vec![0; OUT_LEN] }
    }
}

#[inline(always)]
fn ycc(r: i32, g: i32, b: i32) -> (u8, u8, u8) {
    let y = (19595 * r + 38470 * g + 7471 * b + 32768) >> 16;
    let cb = 128 + ((-11059 * r - 21709 * g + 32768 * b + 32768) >> 16);
    let cr = 128 + ((32768 * r - 27439 * g - 5329 * b + 32768) >> 16);
    (y.clamp(0, 255) as u8, cb.clamp(0, 255) as u8, cr.clamp(0, 255) as u8)
}

/// One output row: LUT, YCbCr, luma vignette.
#[inline(always)]
fn row_ycc(g: &Grader, src: &[u8], graded: bool, vig: Option<&[u8]>,
           y: &mut [u8], cb: &mut [u8], cr: &mut [u8]) {
    for x in 0..W {
        let (mut r, mut gg, mut b) = (src[3 * x], src[3 * x + 1], src[3 * x + 2]);
        if graded {
            r = g.lut[0][r as usize];
            gg = g.lut[1][gg as usize];
            b = g.lut[2][b as usize];
        }
        let (mut yy, u, v) = ycc(r as i32, gg as i32, b as i32);
        if let Some(m) = vig {
            yy = ((yy as u32 * m[x] as u32) / 255) as u8;   // ImageChops.multiply
        }
        y[x] = yy;
        cb[x] = u;
        cr[x] = v;
    }
}

const BOX_W: u32 = (1 << 24) / 5;

#[inline(always)]
fn hblur_row(src: &[u8], dst: &mut [u8]) {
    let at = |i: isize| src[i.clamp(0, W as isize - 1) as usize] as u32;
    let mut acc: u32 = (-2..=2).map(at).sum();
    for x in 0..W {
        dst[x] = ((acc * BOX_W + (1 << 23)) >> 24) as u8;
        acc = acc + at(x as isize + 3) - at(x as isize - 2);
    }
}

/// Vertical box blur of `tmp` into a row, then unsharp against the original.
#[inline(always)]
fn vblur_sharpen_row(tmp: &[u8], row: usize, y: &mut [u8], alpha: f32) {
    let r = |k: isize| (row as isize + k).clamp(0, H as isize - 1) as usize * W;
    let rows = [r(-2), r(-1), r(0), r(1), r(2)];
    for x in 0..W {
        let s: u32 = rows.iter().map(|&o| tmp[o + x] as u32).sum();
        let blur = ((s * BOX_W + (1 << 23)) >> 24) as f32;
        let t = blur + alpha * (y[x] as f32 - blur);          // Image.blend
        y[x] = if t <= 0.0 { 0 } else if t >= 255.0 { 255 } else { t as u8 };
    }
}

impl Frame {
    /// Fill 16:9 and centre-crop. The Z8 sends 1024x680 (photo) or 1024x576
    /// (video), both a pure crop; other sizes go through a bilinear resize,
    /// which the real camera never exercises.
    fn source<'a>(&'a mut self, rgb: &'a [u8], iw: usize, ih: usize)
        -> (&'a [u8], usize, usize, usize) {
        let s = (W as f64 / iw as f64).max(H as f64 / ih as f64);
        // Resize whenever the frame is smaller than the target too: within
        // 0.1% of 1:1 (a 1023-wide frame) the crop below would underflow.
        let (buf, sw, sh): (&[u8], usize, usize) = if (s - 1.0).abs() > 1e-3 || iw < W || ih < H {
            let (nw, nh) = (((iw as f64 * s + 0.5) as usize).max(W), ((ih as f64 * s + 0.5) as usize).max(H));
            // Pillow's default resize filter, as the Python keeper used.
            self.scaled = crate::img::resize_slice(rgb, iw, ih, nw, nh, crate::img::Filter::Bicubic);
            (&self.scaled, nw, nh)
        } else {
            (rgb, iw, ih)
        };
        let (x0, y0) = ((sw - W) / 2, (sh - H) / 2);
        (buf, sw, x0, y0)
    }

    /// Single thread, like the Python keeper.
    pub fn process(&mut self, g: &Grader, rgb: &[u8], iw: usize, ih: usize, graded: bool) {
        let mut ys = std::mem::take(&mut self.y);
        let mut cbs = std::mem::take(&mut self.cb);
        let mut crs = std::mem::take(&mut self.cr);
        let mut tmp = std::mem::take(&mut self.tmp);
        {
            let (src, sw, x0, y0) = self.source(rgb, iw, ih);
            let vig = if graded { g.vig.as_deref() } else { None };
            for r in 0..H {
                let o = ((y0 + r) * sw + x0) * 3;
                row_ycc(g, &src[o..o + W * 3], graded, vig.map(|m| &m[r * W..(r + 1) * W]),
                        &mut ys[r * W..(r + 1) * W], &mut cbs[r * W..(r + 1) * W],
                        &mut crs[r * W..(r + 1) * W]);
            }
        }
        if graded && g.sharp_amt > 0.0 {
            for r in 0..H {
                hblur_row(&ys[r * W..(r + 1) * W], &mut tmp[r * W..(r + 1) * W]);
            }
            for r in 0..H {
                vblur_sharpen_row(&tmp, r, &mut ys[r * W..(r + 1) * W], 1.0 + g.sharp_amt);
            }
        }
        finish(g, &ys, &cbs, &crs, &mut self.out);
        self.y = ys; self.cb = cbs; self.cr = crs; self.tmp = tmp;
    }

    /// The same work split across cores by row bands.
    pub fn process_par(&mut self, g: &Grader, rgb: &[u8], iw: usize, ih: usize, graded: bool) {
        let mut ys = std::mem::take(&mut self.y);
        let mut cbs = std::mem::take(&mut self.cb);
        let mut crs = std::mem::take(&mut self.cr);
        let mut tmp = std::mem::take(&mut self.tmp);
        {
            let (src, sw, x0, y0) = self.source(rgb, iw, ih);
            let vig = if graded { g.vig.as_deref() } else { None };
            ys.par_chunks_mut(W).zip(cbs.par_chunks_mut(W)).zip(crs.par_chunks_mut(W))
                .enumerate().for_each(|(r, ((y, cb), cr))| {
                    let o = ((y0 + r) * sw + x0) * 3;
                    row_ycc(g, &src[o..o + W * 3], graded,
                            vig.map(|m| &m[r * W..(r + 1) * W]), y, cb, cr);
                });
        }
        if graded && g.sharp_amt > 0.0 {
            tmp.par_chunks_mut(W).zip(ys.par_chunks(W))
                .for_each(|(t, y)| hblur_row(y, t));
            let a = 1.0 + g.sharp_amt;
            ys.par_chunks_mut(W).enumerate()
                .for_each(|(r, y)| vblur_sharpen_row(&tmp, r, y, a));
        }
        finish(g, &ys, &cbs, &crs, &mut self.out);
        self.y = ys; self.cb = cbs; self.cr = crs; self.tmp = tmp;
    }
}

/// Limited-range Y, and chroma averaged 2x2 (Image.reduce(2)) then mapped.
fn finish(g: &Grader, ys: &[u8], cbs: &[u8], crs: &[u8], out: &mut [u8]) {
    let (yo, rest) = out.split_at_mut(W * H);
    let (uo, vo) = rest.split_at_mut(W * H / 4);
    for (o, &v) in yo.iter_mut().zip(ys) {
        *o = g.y_lut[v as usize];
    }
    for (plane, dst) in [(cbs, uo), (crs, vo)] {
        for j in 0..H / 2 {
            let (a, b) = (&plane[2 * j * W..], &plane[(2 * j + 1) * W..]);
            for i in 0..W / 2 {
                let s = a[2 * i] as u32 + a[2 * i + 1] as u32
                    + b[2 * i] as u32 + b[2 * i + 1] as u32;
                dst[j * (W / 2) + i] = g.c_lut[((s + 2) >> 2) as usize];
            }
        }
    }
}

