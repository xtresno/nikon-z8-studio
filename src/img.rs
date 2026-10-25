//! The image engine: every Pillow operation the Python app used, ported with
//! Pillow's own arithmetic wherever it is known (blend truncation, L24 luma,
//! DIV255 compositing, the draft-scale rule), so a number measured by the
//! old app means the same thing here.
use std::ffi::c_void;
use std::os::raw::{c_char, c_int, c_uchar, c_ulong};

use fast_image_resize as fr;

// ------------------------------------------------------------------ types
/// Packed 8-bit RGB.
#[derive(Clone)]
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// 8-bit single channel.
#[derive(Clone)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Rgb {
    pub fn new(w: usize, h: usize, fill: [u8; 3]) -> Self {
        let mut px = vec![0u8; w * h * 3];
        if fill != [0, 0, 0] {
            for c in px.chunks_exact_mut(3) {
                c.copy_from_slice(&fill);
            }
        }
        Rgb { w, h, px }
    }
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.w + x) * 3;
        [self.px[i], self.px[i + 1], self.px[i + 2]]
    }
    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }
}

impl Gray {
    pub fn new(w: usize, h: usize, v: u8) -> Self {
        Gray { w, h, px: vec![v; w * h] }
    }
}

// ------------------------------------------------------------------ jpeg
type AllocFn = unsafe extern "C" fn(*mut c_void, usize) -> *mut c_uchar;

extern "C" {
    fn z8j_info(buf: *const c_uchar, len: c_ulong, w: *mut c_int, h: *mut c_int,
                comps: *mut c_int, msg: *mut c_char) -> c_int;
    fn z8j_decode(buf: *const c_uchar, len: c_ulong, denom: c_int, alloc: AllocFn,
                  ctx: *mut c_void, w: *mut c_int, h: *mut c_int, msg: *mut c_char) -> c_int;
    fn z8j_encode(px: *const c_uchar, w: c_int, h: c_int, comps: c_int, quality: c_int,
                  subsampling: c_int, optimize: c_int, dpi: c_int, progressive: c_int,
                  out: *mut *mut c_uchar, outlen: *mut c_ulong, msg: *mut c_char) -> c_int;
    fn z8j_free(p: *mut c_uchar);
}

unsafe extern "C" fn vec_alloc(ctx: *mut c_void, n: usize) -> *mut c_uchar {
    let v = &mut *(ctx as *mut Vec<u8>);
    v.resize(n, 0);
    v.as_mut_ptr()
}

fn msg_str(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Stored size of a JPEG, from its header alone.
pub fn jpeg_info(data: &[u8]) -> Result<(usize, usize), String> {
    let (mut w, mut h, mut c) = (0, 0, 0);
    let mut msg = [0 as c_char; 256];
    let rc = unsafe {
        z8j_info(data.as_ptr(), data.len() as c_ulong, &mut w, &mut h, &mut c, msg.as_mut_ptr())
    };
    if rc != 0 {
        return Err(format!("cannot identify image file ({})", msg_str(&msg)));
    }
    Ok((w as usize, h as usize))
}

/// Pillow's draft rule: the largest of 8, 4, 2, 1 that still leaves the
/// decoded image at least as large as `req` on both sides.
pub fn draft_denom(w: usize, h: usize, req: usize) -> usize {
    if req == 0 {
        return 1;
    }
    let scale = (w / req).min(h / req);
    for a in [8, 4, 2, 1] {
        if scale >= a {
            return a;
        }
    }
    1
}

pub fn jpeg_decode(data: &[u8], denom: usize) -> Result<Rgb, String> {
    let mut out: Vec<u8> = Vec::new();
    let (mut w, mut h) = (0, 0);
    let mut msg = [0 as c_char; 256];
    let rc = unsafe {
        z8j_decode(data.as_ptr(), data.len() as c_ulong, denom as c_int, vec_alloc,
                   &mut out as *mut Vec<u8> as *mut c_void, &mut w, &mut h, msg.as_mut_ptr())
    };
    if rc != 0 {
        return Err(format!("cannot decode JPEG ({})", msg_str(&msg)));
    }
    Ok(Rgb { w: w as usize, h: h as usize, px: out })
}

pub struct JpegOpts {
    pub quality: i32,
    /// Pillow's names: 0 = 4:4:4, 1 = 4:2:2, 2 = 4:2:0, -1 = library default.
    pub subsampling: i32,
    pub optimize: bool,
    pub dpi: i32,
}

impl JpegOpts {
    pub fn q(quality: i32) -> Self {
        JpegOpts { quality, subsampling: -1, optimize: false, dpi: 0 }
    }
}

pub fn jpeg_encode_raw(px: &[u8], w: usize, h: usize, comps: usize, o: &JpegOpts)
    -> Result<Vec<u8>, String> {
    let mut out: *mut c_uchar = std::ptr::null_mut();
    let mut len: c_ulong = 0;
    let mut msg = [0 as c_char; 256];
    let rc = unsafe {
        z8j_encode(px.as_ptr(), w as c_int, h as c_int, comps as c_int,
                   o.quality.clamp(1, 100), o.subsampling, o.optimize as c_int, o.dpi, 0,
                   &mut out, &mut len, msg.as_mut_ptr())
    };
    if rc != 0 || out.is_null() {
        return Err(format!("JPEG encode failed ({})", msg_str(&msg)));
    }
    let v = unsafe { std::slice::from_raw_parts(out, len as usize).to_vec() };
    unsafe { z8j_free(out) };
    Ok(v)
}

pub fn jpeg_encode(im: &Rgb, o: &JpegOpts) -> Result<Vec<u8>, String> {
    jpeg_encode_raw(&im.px, im.w, im.h, 3, o)
}

pub fn png_encode(im: &Rgb, dpi: Option<u32>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, im.w as u32, im.h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        // Fast (fdeflate): 0.76 s against 9.81 s for Balanced on a 45 MP
        // frame, still lossless, and 16% larger (45.4 MB against 39.2 MB).
        enc.set_compression(png::Compression::Fast);
        if let Some(d) = dpi {
            // pHYs is per metre.
            let ppm = (d as f64 / 0.0254).round() as u32;
            enc.set_pixel_dims(Some(png::PixelDimensions {
                xppu: ppm, yppu: ppm, unit: png::Unit::Meter }));
        }
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(&im.px).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

pub fn png_decode(data: &[u8]) -> Result<Rgb, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(data));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut rd = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; rd.output_buffer_size().ok_or("png too large")?];
    let info = rd.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    let src = &buf[..info.buffer_size()];
    let mut px = Vec::with_capacity(w * h * 3);
    match info.color_type {
        png::ColorType::Rgb => px.extend_from_slice(src),
        png::ColorType::Rgba => src.chunks_exact(4).for_each(|p| px.extend_from_slice(&p[..3])),
        png::ColorType::Grayscale => src.iter().for_each(|&v| px.extend_from_slice(&[v, v, v])),
        png::ColorType::GrayscaleAlpha =>
            src.chunks_exact(2).for_each(|p| px.extend_from_slice(&[p[0], p[0], p[0]])),
        png::ColorType::Indexed => return Err("indexed PNG not expanded".into()),
    }
    Ok(Rgb { w, h, px })
}

pub fn tiff_encode(im: &Rgb) -> Result<Vec<u8>, String> {
    use tiff::encoder::{colortype, Compression, TiffEncoder};
    let mut cur = std::io::Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut cur).map_err(|e| e.to_string())?
            .with_compression(Compression::Lzw);
        enc.write_image::<colortype::RGB8>(im.w as u32, im.h as u32, &im.px)
            .map_err(|e| e.to_string())?;
    }
    Ok(cur.into_inner())
}

// ------------------------------------------------------------------ geometry
/// Pillow crop: the box may run past the edges, and the part outside is black.
pub fn crop(im: &Rgb, x0: i64, y0: i64, x1: i64, y1: i64) -> Rgb {
    let (w, h) = ((x1 - x0).max(0) as usize, (y1 - y0).max(0) as usize);
    let mut out = Rgb::new(w, h, [0, 0, 0]);
    for y in 0..h {
        let sy = y0 + y as i64;
        if sy < 0 || sy >= im.h as i64 {
            continue;
        }
        let lo = (-x0).max(0) as usize;
        let hi = ((im.w as i64 - x0).min(w as i64)).max(0) as usize;
        if lo >= hi {
            continue;
        }
        let s = (sy as usize * im.w + (x0 + lo as i64) as usize) * 3;
        let d = (y * w + lo) * 3;
        out.px[d..d + (hi - lo) * 3].copy_from_slice(&im.px[s..s + (hi - lo) * 3]);
    }
    out
}

pub fn mirror(im: &Rgb) -> Rgb {
    let mut out = im.clone();
    for y in 0..im.h {
        for x in 0..im.w {
            let s = (y * im.w + (im.w - 1 - x)) * 3;
            let d = (y * im.w + x) * 3;
            out.px[d..d + 3].copy_from_slice(&im.px[s..s + 3]);
        }
    }
    out
}

/// ImageOps.exif_transpose for EXIF Orientation 1-8.
pub fn apply_orientation(im: Rgb, o: u32) -> Rgb {
    if o <= 1 || o > 8 {
        return im;
    }
    let (w, h) = (im.w, im.h);
    let swap = o >= 5;
    let (nw, nh) = if swap { (h, w) } else { (w, h) };
    let mut out = Rgb::new(nw, nh, [0, 0, 0]);
    // Where output (x, y) comes from, per orientation, chosen ONCE rather than
    // per pixel (turn_into). Measured on 45 MP before: 155-215 ms.
    match o {
        2 => turn_into(&im, &mut out, |x, y| (w - 1 - x, y)),
        3 => turn_into(&im, &mut out, |x, y| (w - 1 - x, h - 1 - y)),
        4 => turn_into(&im, &mut out, |x, y| (x, h - 1 - y)),
        5 => turn_into(&im, &mut out, |x, y| (y, x)),
        6 => turn_into(&im, &mut out, |x, y| (y, h - 1 - x)),
        7 => turn_into(&im, &mut out, |x, y| (w - 1 - y, h - 1 - x)),
        _ => turn_into(&im, &mut out, |x, y| (w - 1 - y, x)),       // 8
    }
    out
}

/// Fill `out` from `im`, output (x, y) taken from `src(x, y)`: bands of 64
/// output rows on every core, 64-pixel tiles within a band, so a 90 degree
/// turn's column-wise reads stay in cache. Generic, so each orientation's
/// position function is inlined into its own loop.
fn turn_into<F: Fn(usize, usize) -> (usize, usize) + Sync>(im: &Rgb, out: &mut Rgb, src: F) {
    use rayon::prelude::*;
    const T: usize = 64;
    let (w, nw) = (im.w, out.w);
    let sp = &im.px;
    out.px.par_chunks_mut(nw * 3 * T).enumerate().for_each(|(band, rows)| {
        let y0 = band * T;
        let bh = rows.len() / (nw * 3);
        for x0 in (0..nw).step_by(T) {
            let x1 = (x0 + T).min(nw);
            for dy in 0..bh {
                let y = y0 + dy;
                let row = &mut rows[dy * nw * 3..(dy + 1) * nw * 3];
                for x in x0..x1 {
                    let (sx, sy) = src(x, y);
                    let s = (sy * w + sx) * 3;
                    row[x * 3..x * 3 + 3].copy_from_slice(&sp[s..s + 3]);
                }
            }
        }
    });
}

#[derive(Clone, Copy)]
pub enum Filter {
    Lanczos,
    Bilinear,
    Bicubic,
    Box,
}

fn fr_filter(f: Filter) -> fr::FilterType {
    match f {
        Filter::Lanczos => fr::FilterType::Lanczos3,
        Filter::Bilinear => fr::FilterType::Bilinear,
        Filter::Bicubic => fr::FilterType::CatmullRom,
        Filter::Box => fr::FilterType::Box,
    }
}

/// Resize, optionally from a fractional source box (Pillow's `box=`).
pub fn resize_box(im: &Rgb, nw: usize, nh: usize, f: Filter,
                  bx: Option<(f64, f64, f64, f64)>) -> Rgb {
    let nw = nw.max(1);
    let nh = nh.max(1);
    if im.w == 0 || im.h == 0 {
        return Rgb::new(nw, nh, [0, 0, 0]);
    }
    let src = fr::images::ImageRef::new(im.w as u32, im.h as u32, &im.px, fr::PixelType::U8x3)
        .expect("source buffer");
    let mut dst = fr::images::Image::new(nw as u32, nh as u32, fr::PixelType::U8x3);
    let mut opts = fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr_filter(f)));
    if let Some((l, t, r, b)) = bx {
        opts = opts.crop(l, t, (r - l).max(1e-6), (b - t).max(1e-6));
    }
    fr::Resizer::new().resize(&src, &mut dst, &opts).expect("resize");
    Rgb { w: nw, h: nh, px: dst.into_vec() }
}

/// Resize packed RGB held in a plain slice (the keeper's decode buffer).
pub fn resize_slice(px: &[u8], w: usize, h: usize, nw: usize, nh: usize, f: Filter) -> Vec<u8> {
    let src = fr::images::ImageRef::new(w as u32, h as u32, px, fr::PixelType::U8x3).expect("source buffer");
    let mut dst = fr::images::Image::new(nw.max(1) as u32, nh.max(1) as u32, fr::PixelType::U8x3);
    let opts = fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr_filter(f)));
    fr::Resizer::new().resize(&src, &mut dst, &opts).expect("resize");
    dst.into_vec()
}

pub fn resize(im: &Rgb, nw: usize, nh: usize, f: Filter) -> Rgb {
    if nw == im.w && nh == im.h {
        return im.clone();
    }
    resize_box(im, nw, nh, f, None)
}

pub fn resize_gray(im: &Gray, nw: usize, nh: usize, f: Filter) -> Gray {
    let src = fr::images::ImageRef::new(im.w as u32, im.h as u32, &im.px, fr::PixelType::U8)
        .expect("source buffer");
    let mut dst = fr::images::Image::new(nw.max(1) as u32, nh.max(1) as u32, fr::PixelType::U8);
    let opts = fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr_filter(f)));
    fr::Resizer::new().resize(&src, &mut dst, &opts).expect("resize");
    Gray { w: nw.max(1), h: nh.max(1), px: dst.into_vec() }
}

/// Image.reduce: integer box average, the last row/column averaging what is left.
pub fn reduce(im: &Rgb, fx: usize, fy: usize) -> Rgb {
    if fx <= 1 && fy <= 1 {
        return im.clone();
    }
    let nw = im.w.div_ceil(fx);
    let nh = im.h.div_ceil(fy);
    let mut out = Rgb::new(nw, nh, [0, 0, 0]);
    // Every output row is its own box of source rows: all cores at once.
    use rayon::prelude::*;
    out.px.par_chunks_mut(nw * 3).enumerate().for_each(|(oy, orow)| {
        let y0 = oy * fy;
        let y1 = (y0 + fy).min(im.h);
        for ox in 0..nw {
            let x0 = ox * fx;
            let x1 = (x0 + fx).min(im.w);
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let mut acc = [0u32; 3];
            for y in y0..y1 {
                let row = &im.px[(y * im.w + x0) * 3..(y * im.w + x1) * 3];
                for p in row.chunks_exact(3) {
                    acc[0] += p[0] as u32;
                    acc[1] += p[1] as u32;
                    acc[2] += p[2] as u32;
                }
            }
            for c in 0..3 {
                orow[ox * 3 + c] = ((acc[c] + n / 2) / n) as u8;
            }
        }
    });
    out
}

/// The size Image.thumbnail would pick, or None when it would not shrink.
pub fn thumbnail_size(w: usize, h: usize, mw: usize, mh: usize) -> Option<(usize, usize)> {
    if mw >= w && mh >= h {
        return None;
    }
    let aspect = w as f64 / h as f64;
    let (x, y) = (mw as f64, mh as f64);
    let round_aspect = |number: f64, key: &dyn Fn(f64) -> f64| -> usize {
        let (a, b) = (number.floor(), number.ceil());
        let pick = if key(a) <= key(b) { a } else { b };
        (pick as usize).max(1)
    };
    if x / y >= aspect {
        let nx = round_aspect(y * aspect, &|n| (aspect - n / y).abs());
        Some((nx, mh))
    } else {
        let ny = round_aspect(x / aspect, &|n| if n == 0.0 { 0.0 } else { (aspect - x / n).abs() });
        Some((mw, ny))
    }
}

/// Image.thumbnail with Pillow's reducing_gap of 2.0: a cheap integer box
/// reduce first, then Lanczos for the rest.
pub fn thumbnail(im: &Rgb, mw: usize, mh: usize) -> Rgb {
    let Some((nw, nh)) = thumbnail_size(im.w, im.h, mw, mh) else {
        return im.clone();
    };
    let fx = (im.w / (nw * 2)).max(1);
    let fy = (im.h / (nh * 2)).max(1);
    if fx == 1 && fy == 1 {
        return resize(im, nw, nh, Filter::Lanczos);
    }
    // The reduced image covers w/fx columns exactly, 917.33 of them for
    // 8256/9: its last column averages only what was left over. Resizing
    // from the whole 918 stretched the picture by a third of a pixel, 44
    // levels at a sharp edge. Pillow resizes from the exact fractional box.
    let base = reduce(im, fx, fy);
    resize_box(&base, nw, nh, Filter::Lanczos,
               Some((0.0, 0.0, im.w as f64 / fx as f64, im.h as f64 / fy as f64)))
}

/// ImageOps.fit: fill (w, h), cropping the overflow at `centering`.
pub fn fit_cover(im: &Rgb, w: usize, h: usize, centering: (f64, f64)) -> Rgb {
    let (lw, lh) = (im.w as f64, im.h as f64);
    let live = lw / lh;
    let outr = w as f64 / h as f64;
    let (cw, ch) = if (live - outr).abs() < f64::EPSILON {
        (lw, lh)
    } else if live >= outr {
        (outr * lh, lh)
    } else {
        (lw, lw / outr)
    };
    let left = (lw - cw) * centering.0;
    let top = (lh - ch) * centering.1;
    resize_box(im, w, h, Filter::Lanczos, Some((left, top, left + cw, top + ch)))
}

/// Pillow's rotate(expand=False) with BICUBIC: an affine sample of each output
/// pixel, black outside the source.
pub fn rotate(im: &Rgb, degrees: f64) -> Rgb {
    let (w, h) = (im.w as f64, im.h as f64);
    let ang = -(degrees % 360.0).to_radians();
    let r15 = |v: f64| (v * 1e15).round() / 1e15;
    let (a, b, d, e) = (r15(ang.cos()), r15(ang.sin()), r15(-ang.sin()), r15(ang.cos()));
    let (cx, cy) = (w / 2.0, h / 2.0);
    let c = a * -cx + b * -cy + cx;
    let f = d * -cx + e * -cy + cy;
    let mut out = Rgb::new(im.w, im.h, [0, 0, 0]);
    let at = |x: i64, y: i64, ch: usize| -> f64 {
        let xx = x.clamp(0, im.w as i64 - 1) as usize;
        let yy = y.clamp(0, im.h as i64 - 1) as usize;
        im.px[(yy * im.w + xx) * 3 + ch] as f64
    };
    let cubic = |v1: f64, v2: f64, v3: f64, v4: f64, t: f64| -> f64 {
        let p1 = v2;
        let p2 = -v1 + v3;
        let p3 = 2.0 * (v1 - v2) + v3 - v4;
        let p4 = -v1 + v2 - v3 + v4;
        p1 + t * (p2 + t * (p3 + t * p4))
    };
    for oy in 0..im.h {
        for ox in 0..im.w {
            let xo = ox as f64 + 0.5;
            let yo = oy as f64 + 0.5;
            let xin = a * xo + b * yo + c;
            let yin = d * xo + e * yo + f;
            if xin < 0.0 || xin >= w || yin < 0.0 || yin >= h {
                continue;
            }
            let (xs, ys) = (xin - 0.5, yin - 0.5);
            let (x0, y0) = (xs.floor() as i64, ys.floor() as i64);
            let (dx, dy) = (xs - x0 as f64, ys - y0 as f64);
            let o = (oy * im.w + ox) * 3;
            for ch in 0..3 {
                let mut rows = [0.0; 4];
                for (k, row) in rows.iter_mut().enumerate() {
                    let yy = y0 - 1 + k as i64;
                    *row = cubic(at(x0 - 1, yy, ch), at(x0, yy, ch), at(x0 + 1, yy, ch),
                                 at(x0 + 2, yy, ch), dx);
                }
                let v = cubic(rows[0], rows[1], rows[2], rows[3], dy);
                out.px[o + ch] = if v <= 0.0 { 0 } else if v >= 255.0 { 255 } else { (v + 0.5) as u8 };
            }
        }
    }
    out
}

// ------------------------------------------------------------------ colour
/// Pillow's RGB to L: L24 with rounding.
#[inline]
pub fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 19595 + g as u32 * 38470 + b as u32 * 7471 + 0x8000) >> 16) as u8
}

pub fn to_gray(im: &Rgb) -> Gray {
    let px = im.px.chunks_exact(3).map(|p| luma(p[0], p[1], p[2])).collect();
    Gray { w: im.w, h: im.h, px }
}

/// The S channel of Pillow's HSV conversion.
pub fn hsv_saturation(im: &Rgb) -> Gray {
    let px = im.px.chunks_exact(3).map(|p| {
        let mx = p[0].max(p[1]).max(p[2]);
        let mn = p[0].min(p[1]).min(p[2]);
        if mx == mn { 0 } else { (((mx - mn) as f32 / mx as f32) * 255.0) as u8 }
    }).collect();
    Gray { w: im.w, h: im.h, px }
}

/// Image.point with one table per channel.
pub fn point3(im: &Rgb, lr: &[u8; 256], lg: &[u8; 256], lb: &[u8; 256]) -> Rgb {
    let mut out = im.clone();
    for p in out.px.chunks_exact_mut(3) {
        p[0] = lr[p[0] as usize];
        p[1] = lg[p[1] as usize];
        p[2] = lb[p[2] as usize];
    }
    out
}

pub fn point_gray(g: &Gray, lut: &[u8; 256]) -> Gray {
    Gray { w: g.w, h: g.h, px: g.px.iter().map(|&v| lut[v as usize]).collect() }
}

/// Image.blend(a, b, alpha), Pillow's float arithmetic and truncation.
#[inline]
pub(crate) fn blend1(a: u8, b: u8, alpha: f32) -> u8 {
    let t = a as f32 + alpha * (b as f32 - a as f32);
    if t <= 0.0 { 0 } else if t >= 255.0 { 255 } else { t as u8 }
}

pub fn blend(a: &Rgb, b: &Rgb, alpha: f32) -> Rgb {
    let px = a.px.iter().zip(&b.px).map(|(&x, &y)| blend1(x, y, alpha)).collect();
    Rgb { w: a.w, h: a.h, px }
}

/// ImageEnhance.Brightness: blend with black.
pub fn enhance_brightness(im: &Rgb, f: f32) -> Rgb {
    let px = im.px.iter().map(|&v| blend1(0, v, f)).collect();
    Rgb { w: im.w, h: im.h, px }
}

pub fn enhance_brightness_gray(g: &Gray, f: f32) -> Gray {
    Gray { w: g.w, h: g.h, px: g.px.iter().map(|&v| blend1(0, v, f)).collect() }
}

/// ImageEnhance.Contrast: blend with a flat grey at the rounded mean luma.
pub fn enhance_contrast(im: &Rgb, f: f32) -> Rgb {
    let g = to_gray(im);
    let n = g.px.len().max(1) as f64;
    let mean = (g.px.iter().map(|&v| v as u64).sum::<u64>() as f64 / n + 0.5) as u8;
    let px = im.px.iter().map(|&v| blend1(mean, v, f)).collect();
    Rgb { w: im.w, h: im.h, px }
}

/// ImageEnhance.Color: blend with the image's own luma.
pub fn enhance_color(im: &Rgb, f: f32) -> Rgb {
    let mut out = im.clone();
    for p in out.px.chunks_exact_mut(3) {
        let l = luma(p[0], p[1], p[2]);
        p[0] = blend1(l, p[0], f);
        p[1] = blend1(l, p[1], f);
        p[2] = blend1(l, p[2], f);
    }
    out
}

/// A 3x3 kernel filter over every channel, borders copied from the source,
/// as Pillow's ImagingFilter3x3 leaves them.
pub fn filter3x3(im: &Rgb, k: [f32; 9], scale: f32, offset: f32) -> Rgb {
    let mut out = im.clone();
    if im.w < 3 || im.h < 3 {
        return out;
    }
    let k: Vec<f32> = k.iter().map(|v| v / scale).collect();
    for y in 1..im.h - 1 {
        for x in 1..im.w - 1 {
            for c in 0..3 {
                let mut s = offset;
                for dy in 0..3 {
                    for dx in 0..3 {
                        let v = im.px[((y + dy - 1) * im.w + (x + dx - 1)) * 3 + c] as f32;
                        s += v * k[dy * 3 + dx];
                    }
                }
                out.px[(y * im.w + x) * 3 + c] =
                    if s <= 0.0 { 0 } else if s >= 255.0 { 255 } else { (s + 0.5) as u8 };
            }
        }
    }
    out
}

pub fn filter3x3_gray(g: &Gray, k: [f32; 9], scale: f32, offset: f32) -> Gray {
    let mut out = g.clone();
    if g.w < 3 || g.h < 3 {
        return out;
    }
    let k: Vec<f32> = k.iter().map(|v| v / scale).collect();
    for y in 1..g.h - 1 {
        for x in 1..g.w - 1 {
            let mut s = offset;
            for dy in 0..3 {
                for dx in 0..3 {
                    s += g.px[(y + dy - 1) * g.w + (x + dx - 1)] as f32 * k[dy * 3 + dx];
                }
            }
            out.px[y * g.w + x] = if s <= 0.0 { 0 } else if s >= 255.0 { 255 } else { (s + 0.5) as u8 };
        }
    }
    out
}

/// ImageEnhance.Sharpness: blend with ImageFilter.SMOOTH.
///
/// Fused and threaded, pixel for pixel what filter3x3 then blend produced:
/// the smoothing sum runs in the same order in f32, and the blend is a
/// 256x256 table of blend1 itself. 18 ms -> a few on a 1100 px preview.
pub fn enhance_sharpness(im: &Rgb, f: f32) -> Rgb {
    use rayon::prelude::*;
    let (w, h) = (im.w, im.h);
    let mut out = im.clone();
    if w < 3 || h < 3 {
        // filter3x3 returns the frame untouched; blend1(v, v, f) then applies.
        for v in out.px.iter_mut() {
            *v = blend1(*v, *v, f);
        }
        return out;
    }
    let lut = blend_table(f);
    let src = &im.px;
    let stride = w * 3;
    out.px.par_chunks_mut(stride).enumerate().for_each(|(y, row)| {
        let edge_row = y == 0 || y == h - 1;
        for i in 0..stride {
            let o = src[y * stride + i];
            let x = i / 3;
            // SMOOTH is (the 3x3 sum + 4 x centre) / 13. In integers that is
            // exact, and it rounds as the f32 version did: N/13 is never
            // within 1/26 of a .5 boundary, far beyond any float error. The
            // byte-pinned grade test holds it to the old output.
            let smooth = if edge_row || x == 0 || x == w - 1 {
                o
            } else {
                let (a, b, c) = ((y - 1) * stride + i, y * stride + i, (y + 1) * stride + i);
                let n = src[a - 3] as u32 + src[a] as u32 + src[a + 3] as u32
                    + src[b - 3] as u32 + 5 * src[b] as u32 + src[b + 3] as u32
                    + src[c - 3] as u32 + src[c] as u32 + src[c + 3] as u32;
                ((2 * n + 13) / 26) as u8
            };
            row[i] = lut[(smooth as usize) << 8 | o as usize];
        }
    });
    out
}

/// blend1(a, b, alpha) for every (a, b), indexed a << 8 | b: one lookup in
/// place of the float arithmetic, with exactly its results.
pub fn blend_table(alpha: f32) -> Vec<u8> {
    let mut t = vec![0u8; 65536];
    for a in 0..256usize {
        for b in 0..256usize {
            t[a << 8 | b] = blend1(a as u8, b as u8, alpha);
        }
    }
    t
}

/// darken_by_mask on a frame the caller no longer needs: in place, threaded.
pub fn darken_by_mask_owned(mut im: Rgb, mask: &Gray) -> Rgb {
    use rayon::prelude::*;
    im.px.par_chunks_mut(3 * 4096).zip(mask.px.par_chunks(4096)).for_each(|(px, m)| {
        for (p, &m) in px.chunks_exact_mut(3).zip(m) {
            for v in p.iter_mut() {
                let t = *v as u32 * m as u32 + 128;
                *v = ((t + (t >> 8)) >> 8) as u8;
            }
        }
    });
    im
}

/// Image.composite(a, black, mask) with Pillow's DIV255 rounding.
pub fn darken_by_mask(im: &Rgb, mask: &Gray) -> Rgb {
    let mut out = im.clone();
    for (p, &m) in out.px.chunks_exact_mut(3).zip(&mask.px) {
        for v in p.iter_mut() {
            let t = *v as u32 * m as u32 + 128;
            *v = ((t + (t >> 8)) >> 8) as u8;
        }
    }
    out
}

/// Image.radial_gradient("L"): 256x256, sqrt(2 d^2) from the centre.
pub fn radial_gradient() -> Gray {
    let mut g = Gray::new(256, 256, 0);
    for y in 0..256i64 {
        for x in 0..256i64 {
            let d2 = ((x - 128) * (x - 128) + (y - 128) * (y - 128)) as f64;
            g.px[(y * 256 + x) as usize] = ((d2 * 2.0).sqrt() as i64).min(255) as u8;
        }
    }
    g
}

// ------------------------------------------------------------------ stats
pub fn histogram(g: &Gray) -> [u64; 256] {
    let mut h = [0u64; 256];
    for &v in &g.px {
        h[v as usize] += 1;
    }
    h
}

pub fn histograms(im: &Rgb) -> [[u64; 256]; 3] {
    let mut h = [[0u64; 256]; 3];
    for p in im.px.chunks_exact(3) {
        h[0][p[0] as usize] += 1;
        h[1][p[1] as usize] += 1;
        h[2][p[2] as usize] += 1;
    }
    h
}

pub fn mean_rgb(im: &Rgb) -> [f64; 3] {
    let n = (im.w * im.h).max(1) as f64;
    let mut s = [0u64; 3];
    for p in im.px.chunks_exact(3) {
        s[0] += p[0] as u64;
        s[1] += p[1] as u64;
        s[2] += p[2] as u64;
    }
    [s[0] as f64 / n, s[1] as f64 / n, s[2] as f64 / n]
}

pub fn mean_gray(g: &Gray) -> f64 {
    g.px.iter().map(|&v| v as u64).sum::<u64>() as f64 / g.px.len().max(1) as f64
}

/// ImageStat stddev, population form.
pub fn stddev_gray(g: &Gray) -> f64 {
    let n = g.px.len().max(1) as f64;
    let m = mean_gray(g);
    let s2: f64 = g.px.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / n;
    (s2 - m * m).max(0.0).sqrt()
}

// ------------------------------------------------------------------ drawing
pub struct Font {
    font: ab_glyph::FontVec,
}

impl Font {
    pub fn load(paths: &[&str]) -> Option<Font> {
        for p in paths {
            if let Ok(bytes) = std::fs::read(p) {
                if let Ok(font) = ab_glyph::FontVec::try_from_vec(bytes) {
                    return Some(Font { font });
                }
            }
        }
        None
    }

    pub fn text_width(&self, text: &str, px: f32) -> f32 {
        use ab_glyph::{Font as _, ScaleFont};
        let sf = self.font.as_scaled(px);
        let mut x = 0.0;
        let mut prev = None;
        for ch in text.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            x += sf.h_advance(id);
            prev = Some(id);
        }
        x
    }

    /// Draw with the top of the ascent at `y`, like Pillow's default anchor.
    pub fn draw(&self, im: &mut Rgb, text: &str, x: f32, y: f32, px: f32, color: [u8; 3]) {
        use ab_glyph::{point, Font as _, ScaleFont};
        let sf = self.font.as_scaled(px);
        let base = y + sf.ascent();
        let mut cx = x;
        let mut prev = None;
        for ch in text.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                cx += sf.kern(p, id);
            }
            let g = id.with_scale_and_position(px, point(cx, base));
            cx += sf.h_advance(id);
            prev = Some(id);
            if let Some(o) = self.font.outline_glyph(g) {
                let bb = o.px_bounds();
                o.draw(|gx, gy, cov| {
                    let xx = bb.min.x as i64 + gx as i64;
                    let yy = bb.min.y as i64 + gy as i64;
                    if xx < 0 || yy < 0 || xx >= im.w as i64 || yy >= im.h as i64 {
                        return;
                    }
                    let i = (yy as usize * im.w + xx as usize) * 3;
                    let a = cov.clamp(0.0, 1.0);
                    for c in 0..3 {
                        let v = im.px[i + c] as f32 * (1.0 - a) + color[c] as f32 * a;
                        im.px[i + c] = v.round().clamp(0.0, 255.0) as u8;
                    }
                });
            }
        }
    }
}

pub fn paste(dst: &mut Rgb, src: &Rgb, x: usize, y: usize) {
    for row in 0..src.h {
        let dy = y + row;
        if dy >= dst.h {
            break;
        }
        let n = src.w.min(dst.w.saturating_sub(x));
        if n == 0 {
            continue;
        }
        let d = (dy * dst.w + x) * 3;
        let s = row * src.w * 3;
        dst.px[d..d + n * 3].copy_from_slice(&src.px[s..s + n * 3]);
    }
}
