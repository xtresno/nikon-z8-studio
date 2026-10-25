//! The libgphoto2 C API, the ~25 calls this app makes, declared by hand.
//! Opaque pointers only; every wrapper turns a negative return into the same
//! "[-10] Timeout ..." text python-gphoto2 produced, because the rest of the
//! app recognises failures by those codes.
use std::ffi::{c_void, CStr, CString};
use std::os::raw::{c_char, c_float, c_int, c_ulong};

#[repr(C)]
pub struct GPContext {
    _p: [u8; 0],
}
#[repr(C)]
pub struct GCamera {
    _p: [u8; 0],
}
#[repr(C)]
pub struct CameraFile {
    _p: [u8; 0],
}
#[repr(C)]
pub struct CameraWidget {
    _p: [u8; 0],
}
#[repr(C)]
pub struct CameraList {
    _p: [u8; 0],
}

#[repr(C)]
pub struct CameraFilePath {
    pub name: [c_char; 128],
    pub folder: [c_char; 1024],
}

pub const GP_CAPTURE_IMAGE: c_int = 0;
pub const GP_FILE_TYPE_NORMAL: c_int = 1;

pub const W_WINDOW: c_int = 0;
pub const W_SECTION: c_int = 1;
pub const W_TEXT: c_int = 2;
pub const W_RANGE: c_int = 3;
pub const W_TOGGLE: c_int = 4;
pub const W_RADIO: c_int = 5;
pub const W_MENU: c_int = 6;
pub const W_BUTTON: c_int = 7;
pub const W_DATE: c_int = 8;

#[link(name = "gphoto2")]
extern "C" {
    fn gp_context_new() -> *mut GPContext;
    fn gp_context_unref(c: *mut GPContext);
    fn gp_camera_new(c: *mut *mut GCamera) -> c_int;
    fn gp_camera_init(c: *mut GCamera, ctx: *mut GPContext) -> c_int;
    fn gp_camera_exit(c: *mut GCamera, ctx: *mut GPContext) -> c_int;
    fn gp_camera_unref(c: *mut GCamera) -> c_int;
    fn gp_camera_capture_preview(c: *mut GCamera, f: *mut CameraFile, ctx: *mut GPContext) -> c_int;
    fn gp_camera_capture(c: *mut GCamera, t: c_int, p: *mut CameraFilePath, ctx: *mut GPContext) -> c_int;
    fn gp_camera_wait_for_event(c: *mut GCamera, timeout: c_int, t: *mut c_int, data: *mut *mut c_void,
                                ctx: *mut GPContext) -> c_int;
    fn gp_camera_file_get(c: *mut GCamera, folder: *const c_char, file: *const c_char, t: c_int,
                          f: *mut CameraFile, ctx: *mut GPContext) -> c_int;
    fn gp_camera_get_config(c: *mut GCamera, w: *mut *mut CameraWidget, ctx: *mut GPContext) -> c_int;
    fn gp_camera_set_config(c: *mut GCamera, w: *mut CameraWidget, ctx: *mut GPContext) -> c_int;
    fn gp_camera_folder_list_folders(c: *mut GCamera, folder: *const c_char, l: *mut CameraList,
                                     ctx: *mut GPContext) -> c_int;
    fn gp_camera_folder_list_files(c: *mut GCamera, folder: *const c_char, l: *mut CameraList,
                                   ctx: *mut GPContext) -> c_int;
    fn gp_file_new(f: *mut *mut CameraFile) -> c_int;
    fn gp_file_unref(f: *mut CameraFile) -> c_int;
    fn gp_file_get_data_and_size(f: *mut CameraFile, d: *mut *const c_char, n: *mut c_ulong) -> c_int;
    fn gp_widget_free(w: *mut CameraWidget) -> c_int;
    fn gp_widget_get_child_by_name(w: *mut CameraWidget, n: *const c_char, c: *mut *mut CameraWidget) -> c_int;
    fn gp_widget_get_child(w: *mut CameraWidget, i: c_int, c: *mut *mut CameraWidget) -> c_int;
    fn gp_widget_count_children(w: *mut CameraWidget) -> c_int;
    fn gp_widget_get_name(w: *mut CameraWidget, n: *mut *const c_char) -> c_int;
    fn gp_widget_get_type(w: *mut CameraWidget, t: *mut c_int) -> c_int;
    fn gp_widget_get_value(w: *mut CameraWidget, v: *mut c_void) -> c_int;
    fn gp_widget_set_value(w: *mut CameraWidget, v: *const c_void) -> c_int;
    fn gp_widget_count_choices(w: *mut CameraWidget) -> c_int;
    fn gp_widget_get_choice(w: *mut CameraWidget, i: c_int, c: *mut *const c_char) -> c_int;
    fn gp_widget_get_readonly(w: *mut CameraWidget, r: *mut c_int) -> c_int;
    fn gp_list_new(l: *mut *mut CameraList) -> c_int;
    fn gp_list_free(l: *mut CameraList) -> c_int;
    fn gp_list_count(l: *mut CameraList) -> c_int;
    fn gp_list_get_name(l: *mut CameraList, i: c_int, n: *mut *const c_char) -> c_int;
}

type LogFunc = unsafe extern "C" fn(level: c_int, domain: *const c_char, s: *const c_char, data: *mut c_void);

#[link(name = "gphoto2_port")]
extern "C" {
    fn gp_port_result_as_string(r: c_int) -> *const c_char;
    fn gp_log_add_func(level: c_int, f: LogFunc, data: *mut c_void) -> c_int;
    fn gp_log_remove_func(id: c_int) -> c_int;
}

const GP_LOG_DATA: c_int = 3;

/// The hex dumps libgphoto2 logs while a raw PTP call runs.
static DUMPS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

unsafe extern "C" fn keep_dump(_l: c_int, _d: *const c_char, s: *const c_char, _x: *mut c_void) {
    if s.is_null() {
        return;
    }
    let s = unsafe { CStr::from_ptr(s) }.to_string_lossy();
    if s.contains("hexdump") {
        DUMPS.lock().unwrap_or_else(|e| e.into_inner()).push(s.into_owned());
    }
}

/// The bytes of one libgphoto2 hex dump: "0000  18 00 ..-.. 00  ascii",
/// sixteen bytes a line in the fixed columns 6..53 (the ASCII column after
/// them can look like hex, so it is never read).
pub fn dump_bytes(dump: &str) -> Vec<u8> {
    dump.lines().filter(|l| l.len() > 6 && l.as_bytes()[..4].iter().all(u8::is_ascii_hexdigit) && &l[4..6] == "  ")
        .flat_map(|l| l.get(6..53.min(l.len())).unwrap_or("").replace('-', " ")
            .split_whitespace().filter_map(|t| u8::from_str_radix(t, 16).ok()).collect::<Vec<_>>())
        .collect()
}

/// The first byte of a PTP data container answering `op` among the dumps.
pub fn data_reply(dumps: &[String], op: u16) -> Option<u8> {
    dumps.iter().map(|d| dump_bytes(d)).find(|b| {
        b.len() >= 13 && b[4..6] == [2, 0] && b[6..8] == op.to_le_bytes()
            && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize == b.len()
    }).map(|b| b[12])
}

extern "C" {
    fn gp_result_as_string(r: c_int) -> *const c_char;
}

pub type GpResult<T> = Result<T, String>;

pub fn err_text(code: c_int) -> String {
    let s = unsafe {
        let p = gp_result_as_string(code);
        if p.is_null() { gp_port_result_as_string(code) } else { p }
    };
    let msg = if s.is_null() { "Unknown error".to_string() }
              else { unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned() };
    format!("[{code}] {msg}")
}

fn ok(code: c_int) -> GpResult<()> {
    if code < 0 { Err(err_text(code)) } else { Ok(()) }
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

fn from_c(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

/// An open camera and its context. Only ever used under the Camera lock.
pub struct Session {
    cam: *mut GCamera,
    ctx: *mut GPContext,
}

unsafe impl Send for Session {}

impl Session {
    pub fn open() -> GpResult<Session> {
        let ctx = unsafe { gp_context_new() };
        let mut cam = std::ptr::null_mut();
        ok(unsafe { gp_camera_new(&mut cam) })?;
        let r = unsafe { gp_camera_init(cam, ctx) };
        if r < 0 {
            unsafe {
                gp_camera_unref(cam);
                gp_context_unref(ctx);
            }
            return Err(err_text(r));
        }
        Ok(Session { cam, ctx })
    }

    pub fn preview(&self) -> GpResult<Vec<u8>> {
        let f = File::new()?;
        ok(unsafe { gp_camera_capture_preview(self.cam, f.0, self.ctx) })?;
        f.data()
    }

    /// (folder, name) of the new frame on the camera.
    pub fn capture(&self) -> GpResult<(String, String)> {
        let mut p = CameraFilePath { name: [0; 128], folder: [0; 1024] };
        ok(unsafe { gp_camera_capture(self.cam, GP_CAPTURE_IMAGE, &mut p, self.ctx) })?;
        Ok((from_c(p.folder.as_ptr()), from_c(p.name.as_ptr())))
    }

    pub fn file_get(&self, folder: &str, name: &str) -> GpResult<Vec<u8>> {
        let f = File::new()?;
        let (cf, cn) = (cstr(folder), cstr(name));
        ok(unsafe { gp_camera_file_get(self.cam, cf.as_ptr(), cn.as_ptr(), GP_FILE_TYPE_NORMAL,
                                       f.0, self.ctx) })?;
        f.data()
    }

    pub fn config(&self) -> GpResult<Config> {
        let mut w = std::ptr::null_mut();
        ok(unsafe { gp_camera_get_config(self.cam, &mut w, self.ctx) })?;
        Ok(Config(w))
    }

    pub fn set_config(&self, c: &Config) -> GpResult<()> {
        ok(unsafe { gp_camera_set_config(self.cam, c.0, self.ctx) })
    }

    fn list(&self, folder: &str, files: bool) -> GpResult<Vec<String>> {
        let mut l = std::ptr::null_mut();
        ok(unsafe { gp_list_new(&mut l) })?;
        let cf = cstr(folder);
        let r = unsafe {
            if files { gp_camera_folder_list_files(self.cam, cf.as_ptr(), l, self.ctx) }
            else { gp_camera_folder_list_folders(self.cam, cf.as_ptr(), l, self.ctx) }
        };
        let out = if r < 0 {
            Err(err_text(r))
        } else {
            let n = unsafe { gp_list_count(l) };
            Ok((0..n.max(0)).map(|i| {
                let mut p = std::ptr::null();
                unsafe { gp_list_get_name(l, i, &mut p) };
                from_c(p)
            }).collect())
        };
        unsafe { gp_list_free(l) };
        out
    }

    /// A one-byte device property the camera answers but does not list
    /// (so libgphoto2 offers no widget for it), read with a raw
    /// GetDevicePropValue through the "opcode" action. That action throws
    /// the reply away, so the reply is taken from libgphoto2's own data log
    /// for the length of the call. The caller holds the camera lock.
    pub fn prop_u8(&self, code: u16) -> GpResult<u8> {
        // A Nikon answers a property from the state it last REPORTED: until
        // its event queue is read, the lever read "photo" in video for good
        // (measured 2026-09-27; a fresh session read it right). Read it first.
        self.drain_events();
        let cfg = self.config()?;
        cfg.child("opcode")?.set_text(&format!("0x1015,0x{code:04x}"))?;
        DUMPS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let id = unsafe { gp_log_add_func(GP_LOG_DATA, keep_dump, std::ptr::null_mut()) };
        let r = self.set_config(&cfg);
        if id >= 0 {
            unsafe { gp_log_remove_func(id) };
        }
        r?;
        let dumps = std::mem::take(&mut *DUMPS.lock().unwrap_or_else(|e| e.into_inner()));
        data_reply(&dumps, 0x1015).ok_or_else(|| "no property data in the reply".into())
    }

    /// End the camera's live view as libgphoto2 does (its "viewfinder"
    /// action, which also clears its own in-live-view flag), so the next
    /// preview starts a fresh one. The Z8 latches its PHOTO/VIDEO lever when
    /// live view starts: a flip during live view is neither reported nor
    /// obeyed (stills refused) until live view restarts (measured
    /// 2026-09-27: VIDEO read for 10 minutes after a flip to PHOTO, and the
    /// camera's own "d1a6 changed" came with the live-view restart).
    pub fn end_live_view(&self) -> GpResult<()> {
        let cfg = self.config()?;
        cfg.child("viewfinder")?.set_int(0)?;
        self.set_config(&cfg)
    }

    /// Read whatever the camera has queued, without waiting. Nothing here
    /// uses the events themselves (captures wait for their own), so they are
    /// freed; a bounded loop, as a busy camera could queue without end.
    pub fn drain_events(&self) {
        const GP_EVENT_TIMEOUT: c_int = 1;
        for _ in 0..32 {
            let (mut t, mut data): (c_int, *mut c_void) = (GP_EVENT_TIMEOUT, std::ptr::null_mut());
            let r = unsafe { gp_camera_wait_for_event(self.cam, 0, &mut t, &mut data, self.ctx) };
            if !data.is_null() {
                unsafe { libc::free(data) };
            }
            if r < 0 || t == GP_EVENT_TIMEOUT {
                break;
            }
        }
    }

    /// Files the camera announces after a capture: in RAW+JPEG the capture
    /// call returns only the JPEG, and the NEF arrives as a FILE_ADDED event
    /// (measured 2026-10-25: the NEF was never downloaded). Stops at the
    /// first quiet `wait_ms`, or after `max_ms` in all.
    pub fn added_files(&self, wait_ms: c_int, max_ms: u64) -> Vec<(String, String)> {
        const GP_EVENT_TIMEOUT: c_int = 1;
        const GP_EVENT_FILE_ADDED: c_int = 2;
        let end = std::time::Instant::now() + std::time::Duration::from_millis(max_ms);
        let mut out = Vec::new();
        while std::time::Instant::now() < end {
            let (mut t, mut data): (c_int, *mut c_void) = (GP_EVENT_TIMEOUT, std::ptr::null_mut());
            let r = unsafe { gp_camera_wait_for_event(self.cam, wait_ms, &mut t, &mut data, self.ctx) };
            if t == GP_EVENT_FILE_ADDED && !data.is_null() {
                let p = unsafe { &*(data as *const CameraFilePath) };
                out.push((from_c(p.folder.as_ptr()), from_c(p.name.as_ptr())));
            }
            if !data.is_null() {
                unsafe { libc::free(data) };
            }
            if r < 0 || t == GP_EVENT_TIMEOUT {
                break;
            }
        }
        out
    }

    pub fn list_folders(&self, folder: &str) -> GpResult<Vec<String>> {
        self.list(folder, false)
    }

    pub fn list_files(&self, folder: &str) -> GpResult<Vec<String>> {
        self.list(folder, true)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            gp_camera_exit(self.cam, self.ctx);
            gp_camera_unref(self.cam);
            gp_context_unref(self.ctx);
        }
    }
}

struct File(*mut CameraFile);

impl File {
    fn new() -> GpResult<File> {
        let mut f = std::ptr::null_mut();
        ok(unsafe { gp_file_new(&mut f) })?;
        Ok(File(f))
    }
    fn data(&self) -> GpResult<Vec<u8>> {
        let mut d = std::ptr::null();
        let mut n: c_ulong = 0;
        ok(unsafe { gp_file_get_data_and_size(self.0, &mut d, &mut n) })?;
        if d.is_null() {
            return Ok(Vec::new());
        }
        Ok(unsafe { std::slice::from_raw_parts(d as *const u8, n as usize) }.to_vec())
    }
}

impl Drop for File {
    fn drop(&mut self) {
        unsafe { gp_file_unref(self.0) };
    }
}

/// A whole config tree. Widgets borrowed from it live as long as it does.
pub struct Config(*mut CameraWidget);

impl Drop for Config {
    fn drop(&mut self) {
        unsafe { gp_widget_free(self.0) };
    }
}

#[derive(Clone, Copy)]
pub struct Widget<'a>(*mut CameraWidget, std::marker::PhantomData<&'a Config>);

/// A widget's value, typed the way python-gphoto2 returned it.
#[derive(Clone, Debug, PartialEq)]
pub enum WValue {
    Text(String),
    Float(f32),
    Int(i32),
    None,
}

impl WValue {
    /// str(value) as Python printed it.
    pub fn py_str(&self) -> String {
        match self {
            WValue::Text(s) => s.clone(),
            WValue::Int(i) => i.to_string(),
            WValue::Float(f) => {
                let v = *f as f64;
                if v.fract() == 0.0 && v.abs() < 1e16 { format!("{v:.1}") } else { format!("{}", *f) }
            }
            WValue::None => "None".into(),
        }
    }
    pub fn json(&self) -> serde_json::Value {
        match self {
            WValue::Text(s) => serde_json::json!(s),
            WValue::Int(i) => serde_json::json!(i),
            WValue::Float(f) => serde_json::json!(*f as f64),
            WValue::None => serde_json::Value::Null,
        }
    }
}

impl Config {
    pub fn root(&self) -> Widget<'_> {
        Widget(self.0, std::marker::PhantomData)
    }
    pub fn child(&self, name: &str) -> GpResult<Widget<'_>> {
        self.root().child(name)
    }
}

impl<'a> Widget<'a> {
    pub fn child(&self, name: &str) -> GpResult<Widget<'a>> {
        let mut c = std::ptr::null_mut();
        let n = cstr(name);
        ok(unsafe { gp_widget_get_child_by_name(self.0, n.as_ptr(), &mut c) })?;
        Ok(Widget(c, std::marker::PhantomData))
    }
    pub fn children(&self) -> Vec<Widget<'a>> {
        let n = unsafe { gp_widget_count_children(self.0) };
        (0..n.max(0)).filter_map(|i| {
            let mut c = std::ptr::null_mut();
            (unsafe { gp_widget_get_child(self.0, i, &mut c) } >= 0)
                .then_some(Widget(c, std::marker::PhantomData))
        }).collect()
    }
    pub fn name(&self) -> String {
        let mut p = std::ptr::null();
        unsafe { gp_widget_get_name(self.0, &mut p) };
        from_c(p)
    }
    pub fn kind(&self) -> c_int {
        let mut t = 0;
        unsafe { gp_widget_get_type(self.0, &mut t) };
        t
    }
    pub fn readonly(&self) -> GpResult<bool> {
        let mut r = 0;
        ok(unsafe { gp_widget_get_readonly(self.0, &mut r) })?;
        Ok(r != 0)
    }
    pub fn choices(&self) -> Vec<String> {
        let n = unsafe { gp_widget_count_choices(self.0) };
        (0..n.max(0)).map(|i| {
            let mut p = std::ptr::null();
            unsafe { gp_widget_get_choice(self.0, i, &mut p) };
            from_c(p)
        }).collect()
    }
    pub fn value(&self) -> GpResult<WValue> {
        match self.kind() {
            W_TEXT | W_RADIO | W_MENU => {
                let mut p: *const c_char = std::ptr::null();
                ok(unsafe { gp_widget_get_value(self.0, &mut p as *mut _ as *mut c_void) })?;
                Ok(if p.is_null() { WValue::None } else { WValue::Text(from_c(p)) })
            }
            W_RANGE => {
                let mut f: c_float = 0.0;
                ok(unsafe { gp_widget_get_value(self.0, &mut f as *mut _ as *mut c_void) })?;
                Ok(WValue::Float(f))
            }
            W_TOGGLE | W_DATE => {
                let mut i: c_int = 0;
                ok(unsafe { gp_widget_get_value(self.0, &mut i as *mut _ as *mut c_void) })?;
                Ok(WValue::Int(i))
            }
            _ => Err("[-2] Bad parameters".into()),
        }
    }
    /// Set from text the way the page sends it, converted to the widget's type.
    pub fn set_text(&self, v: &str) -> GpResult<()> {
        match self.kind() {
            W_TEXT | W_RADIO | W_MENU => {
                let c = cstr(v);
                ok(unsafe { gp_widget_set_value(self.0, c.as_ptr() as *const c_void) })
            }
            W_RANGE => {
                let f: c_float = v.trim().parse().map_err(|_| "[-2] Bad parameters".to_string())?;
                ok(unsafe { gp_widget_set_value(self.0, &f as *const _ as *const c_void) })
            }
            W_TOGGLE | W_DATE => {
                let i: c_int = v.trim().parse::<f64>().map(|x| x as c_int)
                    .map_err(|_| "[-2] Bad parameters".to_string())?;
                ok(unsafe { gp_widget_set_value(self.0, &i as *const _ as *const c_void) })
            }
            _ => Err("[-2] Bad parameters".into()),
        }
    }
    pub fn set_int(&self, i: i32) -> GpResult<()> {
        match self.kind() {
            W_TOGGLE | W_DATE => ok(unsafe { gp_widget_set_value(self.0, &i as *const _ as *const c_void) }),
            _ => self.set_text(&i.to_string()),
        }
    }
}
