//! Opening a captured file as an upright RGB image: sniffing what the bytes
//! really are, queueing HEIF conversions off the shutter path, developing raw
//! through darktable-cli, and honouring EXIF orientation.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use crate::app::{home_dir, mtime};
use crate::img::{self, Rgb};
use crate::tools::{need, run_timeout};

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Kind {
    Heif,
    Jpeg,
    Png,
    Raw,
    Other,
}

pub fn head(path: &Path, n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    match std::fs::File::open(path) {
        Ok(mut f) => {
            let got = f.read(&mut buf).unwrap_or(0);
            buf.truncate(got);
            buf
        }
        Err(_) => Vec::new(),
    }
}

fn is_heif(h: &[u8]) -> bool {
    h.len() >= 12 && matches!(&h[4..12], b"ftypheic" | b"ftypheix" | b"ftypmif1" | b"ftyphevc")
}

/// What the file IS, from its first bytes. The Z8 reports a .nef name while
/// emitting HEIF, so an extension is a claim, not a fact.
pub fn sniff_kind(path: &Path) -> Kind {
    let h = head(path, 16);
    if h.is_empty() {
        return Kind::Other;
    }
    if is_heif(&h) {
        Kind::Heif
    } else if h.starts_with(&[0xff, 0xd8]) {
        Kind::Jpeg
    } else if h.starts_with(b"\x89PNG\r\n\x1a\n") {
        Kind::Png
    } else if h.starts_with(b"II*\0") || h.starts_with(b"MM\0*") {
        Kind::Raw
    } else {
        Kind::Other
    }
}

// ------------------------------------------------------------------ HEIF queue
// One worker, off the shutter path: heif-convert costs 4-5.5 s and a gigabyte
// per frame, and the photographer wants the shutter back.
struct Convert {
    pending: Mutex<HashSet<PathBuf>>,
    cv: Condvar,
    tx: Mutex<Option<mpsc::Sender<(PathBuf, PathBuf)>>>,
}

static CONVERT: std::sync::OnceLock<Convert> = std::sync::OnceLock::new();

fn conv() -> &'static Convert {
    CONVERT.get_or_init(|| Convert {
        pending: Mutex::new(HashSet::new()),
        cv: Condvar::new(),
        tx: Mutex::new(None),
    })
}

pub fn start_convert_worker() {
    let (tx, rx) = mpsc::channel::<(PathBuf, PathBuf)>();
    *conv().tx.lock() = Some(tx);
    std::thread::Builder::new().name("heif-convert".into()).spawn(move || {
        for (heic, jpg) in rx {
            let _ = run_timeout(Command::new("heif-convert").arg("-q").arg("95").arg(&heic).arg(&jpg),
                                Duration::from_secs(180));
            let c = conv();
            c.pending.lock().remove(&jpg);
            c.cv.notify_all();
        }
    }).ok();
}

pub fn await_conversion(jpg: &Path, timeout: f64) -> bool {
    let c = conv();
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    let mut p = c.pending.lock();
    while p.contains(jpg) && Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now()).min(Duration::from_secs(1));
        c.cv.wait_for(&mut p, left.max(Duration::from_millis(50)));
    }
    drop(p);
    jpg.exists()
}

pub fn convert_is_pending(jpg: &Path) -> bool {
    conv().pending.lock().contains(jpg)
}

pub fn convert_backlog() -> usize {
    conv().pending.lock().len()
}

/// Give a downloaded file the extension its bytes deserve, and queue a
/// working JPEG beside a HEIF.
pub fn normalise(path: &Path) -> PathBuf {
    let h = head(path, 16);
    let stem = path.with_extension("");
    if is_heif(&h) {
        let target = stem.with_extension("heic");
        if path != target {
            let _ = std::fs::rename(path, &target);
        }
        let jpg = stem.with_extension("jpg");
        if !jpg.exists() {
            let c = conv();
            let mut p = c.pending.lock();
            if !p.contains(&jpg) {
                p.insert(jpg.clone());
                if let Some(tx) = c.tx.lock().as_ref() {
                    let _ = tx.send((target.clone(), jpg.clone()));
                }
            }
        }
        return jpg;
    }
    let target = if h.starts_with(&[0xff, 0xd8]) {
        stem.with_extension("jpg")
    } else if h.starts_with(b"II*\0") || h.starts_with(b"MM\0*") {
        stem.with_extension("nef")
    } else {
        return path.to_path_buf();
    };
    if path != target {
        let _ = std::fs::rename(path, &target);
    }
    target
}

// ------------------------------------------------------------------ raw
fn dt_config() -> PathBuf {
    home_dir().join(".config/z8-studio-darktable")
}

/// NEFCompression 13 is High Efficiency, 14 is HE*: TicoRAW, which nothing
/// here can develop.
pub fn nef_is_high_efficiency(path: &Path) -> bool {
    if let Some(c) = nef_compression(path) {
        return matches!(c, 13 | 14);
    }
    if which("exiftool").is_none() {
        return false;
    }
    let out = run_timeout(Command::new("exiftool").args(["-n", "-s", "-s", "-s", "-NEFCompression"])
                          .arg(path), Duration::from_secs(20));
    matches!(out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).as_deref(),
             Ok("13") | Ok("14"))
}

/// Raws being developed right now: a second request for the same raw waits
/// for the first instead of running darktable onto the same file.
static DEVELOPING: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
static DEVELOPED: Condvar = Condvar::new();

pub fn develop_raw(path: &Path) -> Result<PathBuf, String> {
    let cache = PathBuf::from(format!("{}.dev.jpg", path.with_extension("").display()));
    let fresh = || matches!((mtime(&cache), mtime(path)), (Some(c), Some(s)) if c >= s);
    {
        let mut g = DEVELOPING.lock();
        while g.get_or_insert_with(HashSet::new).contains(path) {
            DEVELOPED.wait(&mut g);
        }
        if fresh() {
            return Ok(cache);
        }
        g.get_or_insert_with(HashSet::new).insert(path.to_path_buf());
    }
    let r = develop_into(path, &cache);
    if let Some(set) = DEVELOPING.lock().as_mut() {
        set.remove(path);
    }
    DEVELOPED.notify_all();
    r
}

/// darktable writes under a staging name that still ends in .jpg (it picks
/// the format by extension), then the finished file is renamed into place:
/// a reader never sees a half-written develop under the real name.
fn develop_into(path: &Path, cache: &Path) -> Result<PathBuf, String> {
    need("darktable-cli")?;
    let _ = std::fs::create_dir_all(dt_config());
    let staging = cache.with_file_name(format!(".{}.part-{}-{}.jpg",
        cache.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        std::process::id(), crate::store::tid()));
    let r = run_timeout(Command::new("darktable-cli").arg(path).arg(&staging)
        .args(["--width", "6000", "--height", "6000", "--hq", "true",
               "--apply-custom-presets", "false", "--core", "--configdir"])
        .arg(dt_config()).arg("--disable-opencl"), Duration::from_secs(180));
    if staging.exists() && std::fs::rename(&staging, cache).is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    if !cache.exists() {
        let _ = std::fs::remove_file(&staging);
        let err = r.map(|o| String::from_utf8_lossy(&o.stderr).into_owned()).unwrap_or_default();
        if nef_is_high_efficiency(path) || err.contains("corrupt") {
            return Err("this raw is High Efficiency (HE or HE*), which the raw developer cannot \
                        read. On the camera: PHOTO SHOOTING MENU > RAW recording > RAW \
                        compression > Lossless compressed. Frames already shot as HE have to be \
                        re-shot.".into());
        }
        let tail: String = err.chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(format!("raw develop failed: {}",
                           if tail.is_empty() { "darktable-cli produced nothing".into() } else { tail }));
    }
    Ok(cache.to_path_buf())
}

pub fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(bin)).find(|p| {
        use std::os::unix::fs::PermissionsExt;
        p.is_file() && std::fs::metadata(p).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    })
}

// ------------------------------------------------------------------ EXIF
/// EXIF Orientation 1-8 of a JPEG (or anything kamadak can read), 1 if none.
pub fn exif_orientation(data: &[u8]) -> u32 {
    let mut cur = std::io::Cursor::new(data);
    match exif::Reader::new().read_from_container(&mut cur) {
        Ok(ex) => ex.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
            .and_then(|f| f.value.get_uint(0)).unwrap_or(1),
        Err(_) => 1,
    }
}

/// Decode a JPEG or PNG file upright, drafted when only a small copy is wanted.
fn open_upright(path: &Path, draft_to: Option<usize>) -> Result<Rgb, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let kind = sniff_kind(path);
    let im = if kind == Kind::Png {
        img::png_decode(&data)?
    } else {
        let denom = match draft_to {
            Some(d) => {
                let (w, h) = img::jpeg_info(&data)?;
                img::draft_denom(w, h, d)
            }
            None => 1,
        };
        img::jpeg_decode(&data, denom)?
    };
    // A malformed tag must not cost the frame: exif_orientation says 1 then.
    Ok(img::apply_orientation(im, exif_orientation(&data)))
}

/// Open a captured file as upright RGB, developing raw or HEIF first.
pub fn load_editable(path: &Path, draft_to: Option<usize>) -> Result<Rgb, String> {
    let low = path.to_string_lossy().to_lowercase();
    if (low.ends_with(".jpg") || low.ends_with(".jpeg")) && !path.exists() {
        await_conversion(path, 200.0);
    }
    let mut src = path.to_path_buf();
    match sniff_kind(path) {
        Kind::Jpeg | Kind::Png => return open_upright(path, draft_to),
        Kind::Heif => {
            let sibling = path.with_extension("jpg");
            if (sibling.exists() || convert_is_pending(&sibling))
                && await_conversion(&sibling, 200.0) && sibling.exists() {
                return open_upright(&sibling, draft_to);
            }
            let tmp = PathBuf::from(format!("{}.conv.jpg", path.display()));
            let fresh = matches!((mtime(&tmp), mtime(path)), (Some(t), Some(s)) if t >= s);
            if !fresh {
                need("heif-convert")?;
                let r = run_timeout(Command::new("heif-convert").args(["-q", "95"]).arg(path).arg(&tmp),
                                    Duration::from_secs(180));
                if r.is_err() && !tmp.exists() {
                    return Err("heif-convert took over 3 minutes on this frame and was stopped".into());
                }
                if !tmp.exists() {
                    let err = r.map(|o| String::from_utf8_lossy(&o.stderr).trim().to_string())
                        .unwrap_or_default();
                    return Err(format!("heif-convert could not open this frame: {}",
                                       if err.is_empty() { "it produced no output".into() } else { err }));
                }
            }
            src = tmp;
        }
        Kind::Raw => src = develop_raw(path)?,
        Kind::Other => {
            if low.ends_with(".nef") || low.ends_with(".dng") || low.ends_with(".cr2")
                || low.ends_with(".arw") || low.ends_with(".raw") {
                src = develop_raw(path)?;
            }
        }
    }
    open_upright(&src, draft_to)
}

/// A raw file, by content or, for formats sniffing cannot name, by extension.
pub fn is_raw(path: &Path) -> bool {
    let low = path.to_string_lossy().to_lowercase();
    matches!(sniff_kind(path), Kind::Raw)
        || [".nef", ".dng", ".cr2", ".arw", ".raw"].iter().any(|e| low.ends_with(e))
}

/// Has this raw been developed already (its .dev.jpg cache exists)?
pub fn raw_is_developed(path: &Path) -> bool {
    PathBuf::from(format!("{}.dev.jpg", path.with_extension("").display())).exists()
}

/// The camera's own full-size JPEG inside a raw, upright: 0.16 s against
/// about 5 s to develop (measured on a Z8 NEF, 8256x5504). For strip tiles
/// ONLY: the edit preview and every export use the developed file, so what
/// is graded is what is exported. None when the raw carries no usable JPEG.
/// A TIFF-structured file (a NEF) read by positional reads: exiftool starts
/// Perl for every call (0.15-0.18 s a frame) and the exif crate reads all
/// 49 MB of a NEF to find one tag. These walk only the IFDs they need.
struct Tiff {
    f: std::fs::File,
    size: u64,
    le: bool,
}

/// One IFD entry: tag, type, count, and the value-or-offset bytes.
type Entry = (u16, u16, u32, [u8; 4]);

impl Tiff {
    fn open(path: &Path) -> Option<Tiff> {
        let f = std::fs::File::open(path).ok()?;
        let size = f.metadata().ok()?.len();
        let mut t = Tiff { f, size, le: true };
        t.le = match &t.at(0, 4)?[..] {
            b"II*\0" => true,
            b"MM\0*" => false,
            _ => return None,
        };
        Some(t)
    }
    fn at(&self, off: u64, n: usize) -> Option<Vec<u8>> {
        use std::os::unix::fs::FileExt;
        if off.checked_add(n as u64)? > self.size {
            return None;
        }
        let mut b = vec![0; n];
        self.f.read_exact_at(&mut b, off).ok()?;
        Some(b)
    }
    fn u16_(&self, b: &[u8]) -> u16 {
        if self.le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) }
    }
    fn u32_(&self, b: &[u8]) -> u32 {
        let a = [b[0], b[1], b[2], b[3]];
        if self.le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) }
    }
    /// The entries of the IFD at `off`, offsets taken from `base`.
    fn ifd(&self, base: u64, off: u64) -> Option<Vec<Entry>> {
        let n = self.u16_(&self.at(base + off, 2)?) as usize;
        if n == 0 || n > 512 {
            return None;
        }
        let raw = self.at(base + off + 2, 12 * n)?;
        Some(raw.chunks_exact(12).map(|e| (self.u16_(&e[0..2]), self.u16_(&e[2..4]), self.u32_(&e[4..8]),
                                           [e[8], e[9], e[10], e[11]])).collect())
    }
    fn ifd0(&self) -> Option<Vec<Entry>> {
        self.ifd(0, self.u32_(&self.at(4, 4)?) as u64)
    }
    /// A SHORT or LONG entry's first value.
    fn value(&self, e: &Entry) -> Option<u32> {
        match e.1 {
            3 => Some(self.u16_(&e.3[0..2]) as u32),
            4 | 13 => Some(self.u32_(&e.3)),
            _ => None,
        }
    }
}

/// The full-size JPEG a TIFF-based raw (NEF) carries in a SubIFD, the bytes
/// `exiftool -b -JpgFromRaw` prints.
pub fn tiff_jpg_from_raw(path: &Path) -> Option<Vec<u8>> {
    let t = Tiff::open(path)?;
    let &(_, typ, cnt, val) = t.ifd0()?.iter().find(|e| e.0 == 0x014A)?;      // SubIFDs
    let subs: Vec<u64> = match (typ, cnt) {
        (4 | 13, 1) => vec![t.u32_(&val) as u64],
        (4 | 13, c) if c <= 16 => t.at(t.u32_(&val) as u64, 4 * c as usize)?.chunks_exact(4)
            .map(|b| t.u32_(b) as u64).collect(),
        _ => return None,
    };
    for sub in subs {
        let Some(es) = t.ifd(0, sub) else { continue };
        let get = |tag: u16| es.iter().find(|e| e.0 == tag).map(|e| t.u32_(&e.3) as u64);
        if let (Some(off), Some(len)) = (get(0x0201), get(0x0202)) {
            if let Some(b) = t.at(off, len as usize) {
                if b.len() >= 4096 && b.starts_with(&[0xff, 0xd8]) {
                    return Some(b);
                }
            }
        }
    }
    None
}

/// IFD0's Orientation of a TIFF-based file, None when it is not a TIFF.
pub fn tiff_orientation(path: &Path) -> Option<u32> {
    let t = Tiff::open(path)?;
    Some(t.ifd0()?.iter().find(|e| e.0 == 0x0112).and_then(|e| t.value(e)).unwrap_or(1))
}

/// Nikon's NEFCompression, what `exiftool -n -NEFCompression` prints: EXIF
/// IFD, then the MakerNote, which is its own TIFF after a "Nikon\0" and
/// version prefix, offsets from that header. Older bodies store it as tag
/// 0x0093; the Z8 as the int16u at byte 10 of the 0x0051 block.
pub fn nef_compression(path: &Path) -> Option<u32> {
    let t = Tiff::open(path)?;
    let exif = t.ifd0()?.iter().find(|e| e.0 == 0x8769).and_then(|e| t.value(e))? as u64;
    let mn = *t.ifd(0, exif)?.iter().find(|e| e.0 == 0x927C)?;
    let off = t.u32_(&mn.3) as u64;
    if t.at(off, 6)? != b"Nikon\0" {
        return None;
    }
    let base = off + 10;
    let le = match &t.at(base, 4)?[..] {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let inner = Tiff { f: t.f.try_clone().ok()?, size: t.size, le };
    let first = inner.u32_(&inner.at(base + 4, 4)?) as u64;
    let es = inner.ifd(base, first)?;
    if let Some(v) = es.iter().find(|e| e.0 == 0x0093).and_then(|e| inner.value(e)) {
        return Some(v);
    }
    let blk = es.iter().find(|e| e.0 == 0x0051 && e.1 == 7 && e.2 >= 12)?;
    Some(inner.u16_(&inner.at(base + inner.u32_(&blk.3) as u64 + 10, 2)?) as u32)
}

pub fn raw_embedded(path: &Path, draft_to: Option<usize>) -> Option<Rgb> {
    let slow = || {
        which("exiftool")?;
        ["-JpgFromRaw", "-PreviewImage"].iter().find_map(|tag| {
            let o = run_timeout(Command::new("exiftool").args(["-b", tag]).arg(path),
                                Duration::from_secs(10)).ok()?;
            (o.stdout.len() >= 4096 && o.stdout.starts_with(&[0xff, 0xd8])).then_some(o.stdout)
        })
    };
    if let Some(data) = tiff_jpg_from_raw(path).or_else(slow) {
        let denom = match draft_to {
            Some(d) => {
                let (w, h) = img::jpeg_info(&data).ok()?;
                img::draft_denom(w, h, d)
            }
            None => 1,
        };
        let im = img::jpeg_decode(&data, denom).ok()?;
        // The embedded JPEG is sensor-shaped; the turn is on the raw itself.
        return Some(img::apply_orientation(im, file_orientation(path)));
    }
    None
}

/// EXIF Orientation of a file on disk (TIFF-based raws included), 1 if none.
fn file_orientation(path: &Path) -> u32 {
    if let Some(o) = tiff_orientation(path) {
        return o;
    }
    let Ok(f) = std::fs::File::open(path) else { return 1 };
    let mut r = std::io::BufReader::new(f);
    match exif::Reader::new().read_from_container(&mut r) {
        Ok(ex) => ex.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
            .and_then(|f| f.value.get_uint(0)).unwrap_or(1),
        Err(_) => 1,
    }
}

/// The smallest image embedded in a JPEG's MPF index (APP2 "MPF") whose
/// UPRIGHT size covers want_w x want_h, decoded and turned upright. A Z8 JPEG
/// carries 640x424 and 1620x1080 renditions beside the 8256x5504 frame; a
/// strip tile from the small one reads ~160 KB instead of the whole 17 MB
/// entropy stream. Only the header and the chosen image are read from disk.
/// None when there is no MPF, nothing big enough, or anything is malformed.
pub fn mpf_preview(path: &Path, want_w: usize, want_h: usize) -> Option<(Rgb, usize, usize)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = vec![0u8; 256 * 1024];
    let n = f.read(&mut head).ok()?;
    head.truncate(n);
    let (entries, orient) = mpf_entries(&head)?;
    // The FRAME's size, upright: the previews are not quite its shape (640x424
    // against 8256x5504), so a tile is sized from the frame, never from them.
    let (mw, mh) = sof_size(&head)?;
    let turn = orient >= 5;
    let (mw, mh) = if turn { (mh, mw) } else { (mw, mh) };
    let (ww, wh) = if turn { (want_h, want_w) } else { (want_w, want_h) };
    // Smallest first; the first one big enough wins, so a tile reads only
    // the 640 px preview (~160 KB).
    let mut cand: Vec<(u64, u64)> = entries.into_iter().skip(1)
        .filter(|&(off, size)| off > 0 && size > 0 && size <= 8 << 20).collect();
    cand.sort_by_key(|&(_, size)| size);
    for (off, size) in cand {
        f.seek(SeekFrom::Start(off)).ok()?;
        let mut data = vec![0u8; size as usize];
        f.read_exact(&mut data).ok()?;
        let Ok((w, h)) = img::jpeg_info(&data) else { continue };
        if w >= ww && h >= wh {
            let im = img::jpeg_decode(&data, 1).ok()?;
            return Some((img::apply_orientation(im, orient), mw, mh));
        }
    }
    None
}

/// Width and height from a JPEG's SOF marker, reading only the header.
fn sof_size(d: &[u8]) -> Option<(usize, usize)> {
    let mut i = 2;
    while i + 9 <= d.len() && d[i] == 0xFF {
        let m = d[i + 1];
        let len = u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
        if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let h = u16::from_be_bytes([d[i + 5], d[i + 6]]) as usize;
            let w = u16::from_be_bytes([d[i + 7], d[i + 8]]) as usize;
            return (w > 0 && h > 0).then_some((w, h));
        }
        if m == 0xDA {
            return None;
        }
        i += 2 + len;
    }
    None
}

/// (absolute offset, size) of every image in the MPF index, and the main
/// image's EXIF orientation, from the first bytes of a JPEG.
fn mpf_entries(d: &[u8]) -> Option<(Vec<(u64, u64)>, u32)> {
    let be16 = |b: &[u8]| u16::from_be_bytes([b[0], b[1]]) as usize;
    if d.get(0..2)? != [0xFF, 0xD8] {
        return None;
    }
    let orient = exif_orientation(d);
    let mut i = 2;
    while i + 4 <= d.len() && d[i] == 0xFF {
        let m = d[i + 1];
        if m == 0xDA {
            break;
        }
        let len = be16(d.get(i + 2..i + 4)?);
        if m == 0xE2 && d.get(i + 4..i + 8)? == b"MPF\0" {
            let base = i + 8;
            let le = d.get(base..base + 2)? == b"II";
            let r16 = |o: usize| -> Option<usize> { let b = d.get(base + o..base + o + 2)?;
                Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) } as usize) };
            let r32 = |o: usize| -> Option<u64> { let b: [u8; 4] = d.get(base + o..base + o + 4)?.try_into().ok()?;
                Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) } as u64) };
            let ifd = r32(4)? as usize;
            for k in 0..r16(ifd)? {
                let e = ifd + 2 + 12 * k;
                if r16(e)? == 0xB002 {
                    let (cnt, at) = (r32(e + 4)? as usize, r32(e + 8)? as usize);
                    let mut out = Vec::new();
                    for j in 0..cnt / 16 {
                        let (size, off) = (r32(at + 16 * j + 4)?, r32(at + 16 * j + 8)?);
                        out.push((if off == 0 { 0 } else { base as u64 + off }, size));
                    }
                    return Some((out, orient));
                }
            }
            return None;
        }
        i += 2 + len;
    }
    None
}

/// Apply the session's mirror (the Rotate control is live-view only).
pub fn oriented(im: Rgb) -> Rgb {
    if crate::camera::cam().mirror() {
        img::mirror(&im)
    } else {
        im
    }
}
