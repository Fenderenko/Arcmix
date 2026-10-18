//! VST2 plugin host (64-bit) for microphone processing (experimental); VST3 — in vst3.rs.
//!
//! A plugin is a DLL with VSTPluginMain that returns an AEffect structure with function pointers.
//! processReplacing runs on the microphone thread; everything else (opening, the window, saving
//! state) — on the app's main (UI) thread, as plugins expect. A plugin runs inside our process: a
//! broken plugin can bring the app down — the cause is in crash.txt then.

mod vst3;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ffi::c_void,
    io::Read,
    path::Path,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
    },
    time::Duration,
};

use serde::Serialize;
use windows::{
    Win32::{
        Foundation::{FreeLibrary, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW},
        UI::{
            Controls::Dialogs::{GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW},
            WindowsAndMessaging::{
                AdjustWindowRectEx, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, IDC_ARROW, KillTimer, LoadCursorW, LoadIconW, RegisterClassExW, SW_RESTORE, SW_SHOW, SWP_NOACTIVATE,
                SWP_NOMOVE, SWP_NOZORDER, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_TIMER, WNDCLASSEXW,
            },
        },
    },
    core::{HSTRING, PCSTR, PCWSTR, PWSTR, w},
};

use crate::{
    APP_NAME, WM_CFG_CHG,
    config::{self, VstSlot},
    dsp::MicCtl,
    i18n::t,
    logf, server, util,
};

const EFF_OPEN: i32 = 0;
const EFF_CLOSE: i32 = 1;
const EFF_SET_SAMPLE_RATE: i32 = 10;
const EFF_SET_BLOCK_SIZE: i32 = 11;
const EFF_MAINS_CHANGED: i32 = 12;
const EFF_EDIT_GET_RECT: i32 = 13;
const EFF_EDIT_OPEN: i32 = 14;
const EFF_EDIT_CLOSE: i32 = 15;
const EFF_EDIT_IDLE: i32 = 19;
const EFF_GET_CHUNK: i32 = 23;
const EFF_SET_CHUNK: i32 = 24;
const EFF_GET_EFFECT_NAME: i32 = 45;
const EFF_GET_PRODUCT_STRING: i32 = 48;
const EFF_START_PROCESS: i32 = 71;
const EFF_STOP_PROCESS: i32 = 72;

const EFF_FLAGS_HAS_EDITOR: i32 = 1 << 0;
const EFF_FLAGS_CAN_REPLACING: i32 = 1 << 4;
const EFF_FLAGS_PROGRAM_CHUNKS: i32 = 1 << 5;
const EFF_FLAGS_IS_SYNTH: i32 = 1 << 8;

const VST_MAGIC: i32 = 0x56737450; // 'VstP'
const VST_BLOCK: usize = 1024;
/// WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX
const EDITOR_STYLE: u32 = 0x00CA_0000;

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct VstInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub err: String,
    pub editor: bool,
    pub open: bool,
}

#[derive(Serialize)]
pub struct Found {
    pub path: String,
    pub name: String,
}

pub(super) fn c_str(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).trim().to_string()
}

/// The DLL is built for 64-bit Windows (32-bit plugins do not load into a 64-bit app).
pub(super) fn is_pe64(path: &Path) -> Result<bool, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = vec![0u8; 4096];
    let n = f.read(&mut h).map_err(|e| e.to_string())?;
    if n < 64 || h[0] != b'M' || h[1] != b'Z' {
        return Err("not a DLL".into());
    }
    let off = u32::from_le_bytes([h[0x3c], h[0x3d], h[0x3e], h[0x3f]]) as u64;
    use std::io::{Seek, SeekFrom};
    let mut pe = [0u8; 6];
    if f.seek(SeekFrom::Start(off)).is_err() || f.read_exact(&mut pe).is_err() || &pe[..4] != b"PE\0\0" {
        return Err("not a DLL".into());
    }
    Ok(u16::from_le_bytes([pe[4], pe[5]]) == 0x8664)
}

/// (UI thread) Resizes a plugin window so its client area is w×h.
pub(super) fn size_editor(hw: isize, w: i32, h: i32) {
    if hw == 0 || w <= 0 || h <= 0 {
        return;
    }
    let mut rc = RECT { left: 0, top: 0, right: w, bottom: h };
    unsafe {
        let _ = AdjustWindowRectEx(&mut rc, WINDOW_STYLE(EDITOR_STYLE), false, WINDOW_EX_STYLE(0));
        let _ = SetWindowPos(HWND(hw as *mut _), None, 0, 0, rc.right - rc.left, rc.bottom - rc.top, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

// ---------- VST2 ----------

type Dispatcher = unsafe extern "C" fn(*mut c_void, i32, i32, isize, *mut c_void, f32) -> isize;
type ProcRep = unsafe extern "C" fn(*mut c_void, *mut *mut f32, *mut *mut f32, i32);
type HostCb = unsafe extern "C" fn(*mut c_void, i32, i32, isize, *mut c_void, f32) -> isize;

struct V2 {
    lib: HMODULE,
    eff: *mut c_void,
    disp: Dispatcher,
    proc_rep: ProcRep,
    num_in: usize,
    num_out: usize,
    /// audio thread only
    bufs: std::cell::UnsafeCell<V2Bufs>,
    /// UI thread: the configured rate, processing resumed
    rate: AtomicU32,
    resumed: AtomicBool,
}

struct V2Bufs {
    _ins: Vec<Vec<f32>>,
    _outs: Vec<Vec<f32>>,
    in_ptrs: Vec<*mut f32>,
    out_ptrs: Vec<*mut f32>,
}

impl V2 {
    fn dispatch(&self, op: i32, index: i32, value: isize, ptr: *mut c_void, opt: f32) -> isize {
        unsafe { (self.disp)(self.eff, op, index, value, ptr, opt) }
    }

    fn name(&self, path: &str) -> String {
        let mut nb = [0u8; 256];
        self.dispatch(EFF_GET_EFFECT_NAME, 0, 0, nb.as_mut_ptr() as *mut c_void, 0.0);
        let mut name = c_str(&nb);
        if name.is_empty() {
            nb = [0u8; 256];
            self.dispatch(EFF_GET_PRODUCT_STRING, 0, 0, nb.as_mut_ptr() as *mut c_void, 0.0);
            name = c_str(&nb);
        }
        if name.is_empty() {
            name = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        }
        name
    }

    /// The rate and block size (the plugin is suspended while they change).
    fn configure(&self, rate: u32) {
        let resumed = self.resumed.load(Ordering::Relaxed);
        if resumed && self.rate.load(Ordering::Relaxed) == rate {
            return;
        }
        if resumed {
            self.dispatch(EFF_STOP_PROCESS, 0, 0, std::ptr::null_mut(), 0.0);
            self.dispatch(EFF_MAINS_CHANGED, 0, 0, std::ptr::null_mut(), 0.0);
        }
        self.dispatch(EFF_SET_SAMPLE_RATE, 0, 0, std::ptr::null_mut(), rate as f32);
        self.dispatch(EFF_SET_BLOCK_SIZE, 0, VST_BLOCK as isize, std::ptr::null_mut(), 0.0);
        self.dispatch(EFF_MAINS_CHANGED, 0, 1, std::ptr::null_mut(), 0.0);
        self.dispatch(EFF_START_PROCESS, 0, 0, std::ptr::null_mut(), 0.0);
        self.rate.store(rate, Ordering::Relaxed);
        self.resumed.store(true, Ordering::Relaxed);
    }

    /// Processes n ≤ VST_BLOCK frames in place (microphone thread).
    fn run(&self, buf: &mut [f32], n: usize, ch: usize) {
        let b = unsafe { &mut *self.bufs.get() };
        for (i, &p) in b.in_ptrs.iter().enumerate() {
            let dst = unsafe { std::slice::from_raw_parts_mut(p, n) };
            let src = i.min(ch - 1);
            for (k, d) in dst.iter_mut().enumerate() {
                *d = buf[k * ch + src];
            }
        }
        unsafe { (self.proc_rep)(self.eff, b.in_ptrs.as_mut_ptr(), b.out_ptrs.as_mut_ptr(), n as i32) };
        for c in 0..ch {
            let src = unsafe { std::slice::from_raw_parts(b.out_ptrs[c.min(self.num_out - 1)], n) };
            for (k, &s) in src.iter().enumerate() {
                buf[k * ch + c] = s;
            }
        }
    }
}

fn load_v2(path: &str, rate: u32, state: &[u8]) -> Result<(V2, String, i32), String> {
    if !is_pe64(Path::new(path))? {
        return Err("32-bit".into());
    }
    let lib = unsafe { LoadLibraryExW(&HSTRING::from(path), None, LOAD_WITH_ALTERED_SEARCH_PATH) }.map_err(|e| format!("LoadLibrary: {e}"))?;
    let fail = |e: &str| -> Result<(V2, String, i32), String> {
        unsafe {
            let _ = FreeLibrary(lib);
        }
        Err(e.to_string())
    };
    let entry = unsafe { GetProcAddress(lib, PCSTR(b"VSTPluginMain\0".as_ptr())).or_else(|| GetProcAddress(lib, PCSTR(b"main\0".as_ptr()))) };
    let Some(entry) = entry else { return fail("not a VST2 plugin") };
    let entry: unsafe extern "C" fn(HostCb) -> *mut c_void = unsafe { std::mem::transmute(entry) };
    let eff = unsafe { entry(host_cb) };
    if eff.is_null() || unsafe { *(eff as *const i32) } != VST_MAGIC {
        return fail("not a VST2 plugin");
    }
    let at = |off: usize| unsafe { (eff as *const u8).add(off) };
    let (disp, proc_rep, num_in, num_out, flags) = unsafe {
        (
            *(at(8) as *const Option<Dispatcher>),
            *(at(120) as *const Option<ProcRep>),
            *(at(48) as *const i32),
            *(at(52) as *const i32),
            *(at(56) as *const i32),
        )
    };
    let Some(disp) = disp else { return fail("not a VST2 plugin") };
    let v = V2 {
        lib,
        eff,
        disp,
        // checked below; a no-op placeholder keeps the struct valid until then
        proc_rep: proc_rep.unwrap_or(noop_proc),
        num_in: num_in.max(0) as usize,
        num_out: num_out.max(0) as usize,
        bufs: std::cell::UnsafeCell::new(V2Bufs { _ins: Vec::new(), _outs: Vec::new(), in_ptrs: Vec::new(), out_ptrs: Vec::new() }),
        rate: AtomicU32::new(0),
        resumed: AtomicBool::new(false),
    };
    v.dispatch(EFF_OPEN, 0, 0, std::ptr::null_mut(), 0.0);
    let name = v.name(path);
    let bad = if flags & EFF_FLAGS_IS_SYNTH != 0 || v.num_in < 1 || v.num_out < 1 {
        Some("not an effect")
    } else if flags & EFF_FLAGS_CAN_REPLACING == 0 || proc_rep.is_none() {
        Some("unsupported (no processReplacing)")
    } else {
        None
    };
    if let Some(e) = bad {
        v.dispatch(EFF_CLOSE, 0, 0, std::ptr::null_mut(), 0.0);
        return fail(e);
    }
    let mut ins: Vec<Vec<f32>> = (0..v.num_in).map(|_| vec![0f32; VST_BLOCK]).collect();
    let mut outs: Vec<Vec<f32>> = (0..v.num_out).map(|_| vec![0f32; VST_BLOCK]).collect();
    let in_ptrs = ins.iter_mut().map(|b| b.as_mut_ptr()).collect();
    let out_ptrs = outs.iter_mut().map(|b| b.as_mut_ptr()).collect();
    unsafe { *v.bufs.get() = V2Bufs { _ins: ins, _outs: outs, in_ptrs, out_ptrs } };
    if !state.is_empty() && flags & EFF_FLAGS_PROGRAM_CHUNKS != 0 {
        v.dispatch(EFF_SET_CHUNK, 0, state.len() as isize, state.as_ptr() as *mut c_void, 0.0);
    }
    v.configure(rate);
    logf!("vst: loaded {name} ({path}), in {}, out {}, flags {flags:#x}", v.num_in, v.num_out);
    Ok((v, name, flags))
}

unsafe extern "C" fn noop_proc(_e: *mut c_void, _i: *mut *mut f32, _o: *mut *mut f32, _n: i32) {}

// ---------- the plugin ----------

pub(crate) struct Plug {
    id: String,
    name: String,
    flags: i32,
    v2: Option<V2>,
    v3: Option<vst3::V3>,
    on: AtomicBool,
    dirty: Arc<AtomicBool>,
    /// the editor window (UI thread), 0 — closed
    editor: Arc<AtomicIsize>,
    unloaded: AtomicBool,
}

// The plugin is driven from the UI thread and the microphone thread as VST hosts do.
unsafe impl Send for Plug {}
unsafe impl Sync for Plug {}

/// Effect pointer → plugin, for the host callback.
static BY_EFF: LazyLock<Mutex<HashMap<usize, std::sync::Weak<Plug>>>> = LazyLock::new(Default::default);
static VST_RATE: AtomicU32 = AtomicU32::new(48000);

fn plug_of(effect: *mut c_void) -> Option<Arc<Plug>> {
    BY_EFF.lock().unwrap_or_else(|e| e.into_inner()).get(&(effect as usize)).and_then(|w| w.upgrade())
}

/// UI thread only.
fn load(id: &str, path: &str, rate: u32, state: &[u8]) -> Result<Arc<Plug>, String> {
    let dirty = Arc::new(AtomicBool::new(false));
    let editor = Arc::new(AtomicIsize::new(0));
    let p = if Path::new(path).extension().is_some_and(|x| x.eq_ignore_ascii_case("vst3")) {
        let l = vst3::load(path, rate, state, dirty.clone(), editor.clone())?;
        Plug { id: id.into(), name: l.name, flags: l.flags, v2: None, v3: Some(l.v3), on: AtomicBool::new(true), dirty, editor, unloaded: AtomicBool::new(false) }
    } else {
        let (v, name, flags) = load_v2(path, rate, state)?;
        Plug { id: id.into(), name, flags, v2: Some(v), v3: None, on: AtomicBool::new(true), dirty, editor, unloaded: AtomicBool::new(false) }
    };
    let p = Arc::new(p);
    if let Some(v) = &p.v2 {
        BY_EFF.lock().unwrap_or_else(|e| e.into_inner()).insert(v.eff as usize, Arc::downgrade(&p));
    }
    Ok(p)
}

impl Plug {
    fn dispatch(&self, op: i32, index: i32, value: isize, ptr: *mut c_void, opt: f32) -> isize {
        match &self.v2 {
            Some(v) if !self.unloaded.load(Ordering::Acquire) => v.dispatch(op, index, value, ptr, opt),
            _ => 0,
        }
    }

    fn configure(&self, rate: u32) {
        if self.unloaded.load(Ordering::Acquire) {
            return;
        }
        if let Some(v) = &self.v3 {
            v.configure(rate);
        } else if let Some(v) = &self.v2 {
            v.configure(rate);
        }
    }

    /// Processes n ≤ VST_BLOCK frames in place (microphone thread).
    fn run(&self, buf: &mut [f32], n: usize, ch: usize) {
        if self.unloaded.load(Ordering::Acquire) {
            return;
        }
        if let Some(v) = &self.v3 {
            v.run(buf, n, ch);
        } else if let Some(v) = &self.v2 {
            v.run(buf, n, ch);
        }
    }

    fn chunk(&self) -> Option<Vec<u8>> {
        if self.unloaded.load(Ordering::Acquire) {
            return None;
        }
        if let Some(v) = &self.v3 {
            return v.save_state();
        }
        if self.flags & EFF_FLAGS_PROGRAM_CHUNKS == 0 {
            return None;
        }
        let mut ptr: *mut u8 = std::ptr::null_mut();
        let n = self.dispatch(EFF_GET_CHUNK, 0, 0, &mut ptr as *mut _ as *mut c_void, 0.0);
        if n <= 0 || ptr.is_null() || n > 32 << 20 {
            return None;
        }
        Some(unsafe { std::slice::from_raw_parts(ptr, n as usize) }.to_vec())
    }

    fn has_editor(&self) -> bool {
        self.flags & EFF_FLAGS_HAS_EDITOR != 0
    }

    fn unload(&self) {
        self.close_editor();
        if self.unloaded.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(v) = &self.v3 {
            v.unload();
            return;
        }
        if let Some(v) = &self.v2 {
            if v.resumed.swap(false, Ordering::Relaxed) {
                v.dispatch(EFF_STOP_PROCESS, 0, 0, std::ptr::null_mut(), 0.0);
                v.dispatch(EFF_MAINS_CHANGED, 0, 0, std::ptr::null_mut(), 0.0);
            }
            v.dispatch(EFF_CLOSE, 0, 0, std::ptr::null_mut(), 0.0);
            BY_EFF.lock().unwrap_or_else(|e| e.into_inner()).remove(&(v.eff as usize));
            unsafe {
                let _ = FreeLibrary(v.lib);
            }
        }
    }

    // ---------- plugin window ----------

    fn editor_rect(&self) -> (i32, i32) {
        let mut er: *const i16 = std::ptr::null();
        self.dispatch(EFF_EDIT_GET_RECT, 0, 0, &mut er as *mut _ as *mut c_void, 0.0);
        if er.is_null() {
            return (0, 0);
        }
        let r = unsafe { std::slice::from_raw_parts(er, 4) }; // top, left, bottom, right
        ((r[3] - r[1]) as i32, (r[2] - r[0]) as i32)
    }

    fn open_editor(self: &Arc<Self>) -> Result<(), String> {
        if !self.has_editor() {
            return Err("no editor".into());
        }
        let cur = self.editor.load(Ordering::Relaxed);
        if cur != 0 {
            unsafe {
                let _ = ShowWindow(HWND(cur as *mut _), SW_RESTORE);
                let _ = SetForegroundWindow(HWND(cur as *mut _));
            }
            return Ok(());
        }
        register_class()?;
        let inst = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
        let title = HSTRING::from(format!("{} — {APP_NAME}", self.name));
        let hw = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("ArcmixVst"), &title, WINDOW_STYLE(EDITOR_STYLE), CW_USEDEFAULT, CW_USEDEFAULT, 400, 300, None, None, Some(inst.into()), None) }
            .map_err(|_| "CreateWindowEx".to_string())?;
        let hwi = hw.0 as isize;
        self.editor.store(hwi, Ordering::Relaxed);
        WNDS.with(|m| m.borrow_mut().insert(hwi, self.clone()));
        if let Some(v) = &self.v3 {
            let (mut w, mut h) = match v.attach_view(hwi) {
                Ok(s) => s,
                Err(e) => {
                    WNDS.with(|m| m.borrow_mut().remove(&hwi));
                    self.editor.store(0, Ordering::Relaxed);
                    unsafe {
                        let _ = DestroyWindow(hw);
                    }
                    return Err(e);
                }
            };
            if w <= 0 || h <= 0 {
                (w, h) = (400, 300);
            }
            size_editor(hwi, w, h);
            unsafe {
                let _ = ShowWindow(hw, SW_SHOW);
                let _ = SetForegroundWindow(hw);
            }
            self.dirty.store(true, Ordering::Relaxed);
            return Ok(());
        }
        // some plugins know their size only after opening — asked twice
        let (mut w, mut h) = self.editor_rect();
        self.dispatch(EFF_EDIT_OPEN, 0, 0, hw.0, 0.0);
        let (w2, h2) = self.editor_rect();
        if w2 > 0 {
            (w, h) = (w2, h2);
        }
        if w <= 0 {
            (w, h) = (400, 300);
        }
        size_editor(hwi, w, h);
        unsafe {
            let _ = ShowWindow(hw, SW_SHOW);
            let _ = SetForegroundWindow(hw);
            SetTimer(Some(hw), 1, 30, None);
        }
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn close_editor(&self) {
        let hwi = self.editor.swap(0, Ordering::Relaxed);
        if hwi == 0 {
            return;
        }
        let hw = HWND(hwi as *mut _);
        if let Some(v) = &self.v3 {
            v.detach_view();
        } else {
            unsafe {
                let _ = KillTimer(Some(hw), 1);
            }
            self.dispatch(EFF_EDIT_CLOSE, 0, 0, std::ptr::null_mut(), 0.0);
        }
        WNDS.with(|m| m.borrow_mut().remove(&hwi));
        unsafe {
            let _ = DestroyWindow(hw);
        }
    }
}

thread_local! {
    /// (UI thread) Plugin windows.
    static WNDS: RefCell<HashMap<isize, Arc<Plug>>> = RefCell::new(HashMap::new());
    static CLASS_OK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn register_class() -> Result<(), String> {
    if CLASS_OK.with(|c| c.get()) {
        return Ok(());
    }
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: inst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(inst.into()), PCWSTR(1 as *const u16)).unwrap_or_default(),
            lpszClassName: w!("ArcmixVst"),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err("RegisterClassEx".into());
        }
    }
    CLASS_OK.with(|c| c.set(true));
    Ok(())
}

fn plug_of_wnd(h: HWND) -> Option<Arc<Plug>> {
    WNDS.with(|m| m.borrow().get(&(h.0 as isize)).cloned())
}

unsafe extern "system" fn wnd_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let handled = util::safely_value("vst window", || match m {
        WM_TIMER => {
            if let Some(p) = plug_of_wnd(h) {
                p.dispatch(EFF_EDIT_IDLE, 0, 0, std::ptr::null_mut(), 0.0);
            }
            true
        }
        WM_CLOSE => match plug_of_wnd(h) {
            Some(p) => {
                p.close_editor();
                save_state();
                update_status(&config::get().mic_fx.plugins);
                true
            }
            None => false,
        },
        _ => false,
    });
    if handled == Some(true) {
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(h, m, wp, lp) }
}

// ---------- the callback for the plugin ----------

/// VstTimeInfo (only the rate and tempo are filled); written on the UI thread when the rate changes.
struct TimeInfo(std::cell::UnsafeCell<[u8; 128]>);
unsafe impl Sync for TimeInfo {}
static TIME_INFO: TimeInfo = TimeInfo(std::cell::UnsafeCell::new([0; 128]));

fn set_rate(rate: u32) {
    VST_RATE.store(rate, Ordering::Relaxed);
    let t = unsafe { &mut *TIME_INFO.0.get() };
    t[8..16].copy_from_slice(&(rate as f64).to_le_bytes());
    t[32..40].copy_from_slice(&120f64.to_le_bytes());
    t[64..68].copy_from_slice(&4u32.to_le_bytes());
    t[68..72].copy_from_slice(&4u32.to_le_bytes());
    t[84..88].copy_from_slice(&(1u32 << 10 | 1 << 13).to_le_bytes()); // tempo, timeSig valid
}

unsafe extern "C" fn host_cb(effect: *mut c_void, opcode: i32, index: i32, value: isize, ptr: *mut c_void, _opt: f32) -> isize {
    match opcode {
        // automate, beginEdit, endEdit — the plugin state changed
        0 | 43 | 44 => {
            if let Some(p) = plug_of(effect) {
                p.dirty.store(true, Ordering::Relaxed);
            }
            0
        }
        1 => 2400, // version
        7 => TIME_INFO.0.get() as isize, // getTime
        13 => 1,   // ioChanged
        // sizeWindow(index = width, value = height)
        15 => match plug_of(effect) {
            Some(p) if p.editor.load(Ordering::Relaxed) != 0 => {
                let (w, h) = (index, value as i32);
                let ed = p.editor.clone();
                util::run_on_ui(move || size_editor(ed.load(Ordering::Relaxed), w, h));
                1
            }
            _ => 0,
        },
        16 => VST_RATE.load(Ordering::Relaxed) as isize, // getSampleRate
        17 => VST_BLOCK as isize,                        // getBlockSize
        24 => 1,                                         // getAutomationState: off
        // vendor / product string
        32 | 33 => {
            if ptr.is_null() {
                return 0;
            }
            let b = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, 64) };
            let n = APP_NAME.len().min(63);
            b[..n].copy_from_slice(&APP_NAME.as_bytes()[..n]);
            b[n] = 0;
            1
        }
        34 => 1000, // vendor version
        37 => {
            // canDo
            if ptr.is_null() {
                return 0;
            }
            let mut s = Vec::new();
            for i in 0..64 {
                let c = unsafe { *(ptr as *const u8).add(i) };
                if c == 0 {
                    break;
                }
                s.push(c);
            }
            matches!(s.as_slice(), b"sizeWindow" | b"supplyIdle" | b"sendVstTimeInfo") as isize
        }
        41 => 1, // getLanguage: English
        _ => 0,
    }
}

// ---------- the chain ----------

#[derive(Default)]
struct Mgr {
    /// in slot order (only the loaded ones)
    plugs: Vec<Arc<Plug>>,
    /// id → error
    errs: HashMap<String, String>,
    rate: u32,
    mp: Option<Arc<MicCtl>>,
}

thread_local! {
    /// (UI thread) The loaded plugins.
    static MGR: RefCell<Mgr> = RefCell::new(Mgr { rate: 48000, ..Default::default() });
}

static STATUS: Mutex<Vec<VstInfo>> = Mutex::new(Vec::new());

fn plugs() -> Vec<Arc<Plug>> {
    MGR.with(|m| m.borrow().plugs.clone())
}

/// (UI thread) Brings the loaded plugins in line with the settings and hands the chain to the
/// microphone thread.
pub fn sync() {
    let c = config::get();
    let (have_list, mut errs, rate) = MGR.with(|m| {
        let m = m.borrow();
        (m.plugs.clone(), m.errs.clone(), m.rate)
    });
    let want: Vec<VstSlot> = if c.mic_fx.on && c.on("vst") { c.mic_fx.plugins.clone() } else { Vec::new() };
    let have: HashMap<&str, &Arc<Plug>> = have_list.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut next = Vec::new();
    let mut keep = HashSet::new();
    for s in &want {
        keep.insert(s.id.clone());
        let mut p = have.get(s.id.as_str()).map(|p| (*p).clone());
        if p.is_none() && errs.get(&s.id).is_none_or(|e| e.is_empty()) {
            let st = base64_decode(&s.state).unwrap_or_default();
            set_rate(rate);
            match load(&s.id, &s.path, rate, &st) {
                Ok(x) => p = Some(x),
                Err(e) => {
                    logf!("vst: {}: {e}", s.path);
                    errs.insert(s.id.clone(), e);
                }
            }
        }
        if let Some(p) = p {
            p.on.store(s.on, Ordering::Relaxed);
            next.push(p);
        }
    }
    errs.retain(|id, _| keep.contains(id));
    let drop: Vec<Arc<Plug>> = have_list.iter().filter(|p| !keep.contains(&p.id)).cloned().collect();
    if !drop.is_empty() {
        save_state(); // while the list still has the ones being unloaded
    }
    MGR.with(|m| {
        let mut m = m.borrow_mut();
        m.plugs = next;
        m.errs = errs;
    });
    publish(); // waits until the microphone thread has left the old chain
    for p in drop {
        logf!("vst: unloaded {}", p.name);
        p.unload();
    }
    update_status(&want);
}

/// The new chain for the microphone thread.
fn publish() {
    let (mp, chain) = MGR.with(|m| {
        let m = m.borrow();
        (m.mp.clone(), m.plugs.clone())
    });
    let Some(mp) = mp else { return };
    if chain.is_empty() {
        mp.set_ext(None, Duration::from_millis(200));
        return;
    }
    mp.set_ext(
        Some(Box::new(move |buf: &mut [f32], ch: usize| {
            let frames = buf.len() / ch;
            for p in &chain {
                if !p.on.load(Ordering::Relaxed) {
                    continue;
                }
                let mut off = 0;
                while off < frames {
                    let n = VST_BLOCK.min(frames - off);
                    p.run(&mut buf[off * ch..(off + n) * ch], n, ch);
                    off += n;
                }
            }
        })),
        Duration::from_millis(200),
    );
}

/// (UI thread) The microphone stream started at ctl.rate.
pub fn attach(ctl: &Arc<MicCtl>) {
    let rate = ctl.rate;
    let (old, changed) = MGR.with(|m| {
        let mut m = m.borrow_mut();
        let old = m.mp.replace(ctl.clone()).filter(|o| !Arc::ptr_eq(o, ctl));
        let changed = rate != m.rate;
        m.rate = rate;
        (old, changed)
    });
    if let Some(o) = old {
        o.set_ext(None, Duration::from_millis(200));
    }
    if changed {
        set_rate(rate);
        for p in plugs() {
            p.configure(rate);
        }
    }
    sync();
}

fn update_status(want: &[VstSlot]) {
    let (by_id, errs) = MGR.with(|m| {
        let m = m.borrow();
        (m.plugs.iter().map(|p| (p.id.clone(), p.clone())).collect::<HashMap<_, _>>(), m.errs.clone())
    });
    let out = want
        .iter()
        .map(|s| {
            let mut vi = VstInfo {
                id: s.id.clone(),
                name: Path::new(&s.path).file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(),
                err: errs.get(&s.id).cloned().unwrap_or_default(),
                ..Default::default()
            };
            if let Some(p) = by_id.get(&s.id) {
                vi.name = p.name.clone();
                vi.editor = p.has_editor();
                vi.open = p.editor.load(Ordering::Relaxed) != 0;
            }
            vi
        })
        .collect();
    *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = out;
    server::notify_subs();
}

pub fn status() -> Vec<VstInfo> {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// (UI thread) The state of plugins that changed — into the settings.
fn save_state() {
    let mut states = HashMap::new();
    for p in plugs() {
        if (p.dirty.swap(false, Ordering::Relaxed) || p.editor.load(Ordering::Relaxed) != 0)
            && let Some(b) = p.chunk()
        {
            states.insert(p.id.clone(), base64_encode(&b));
        }
    }
    if states.is_empty() {
        return;
    }
    config::set(|c| {
        for s in c.mic_fx.plugins.iter_mut() {
            if let Some(v) = states.remove(&s.id) {
                s.state = v;
            }
        }
    });
}

/// (UI thread) Every 10 s.
pub fn timer() {
    if !plugs().is_empty() {
        save_state();
    }
}

fn edit(id: &str) {
    if let Some(p) = plugs().into_iter().find(|p| p.id == id) {
        if let Err(e) = p.open_editor() {
            logf!("vst: editor {}: {e}", p.name);
        }
        update_status(&config::get().mic_fx.plugins);
    }
}

/// (UI thread) On exit: state saved, windows closed (unloading libraries on exit is not needed,
/// and risky).
pub fn shutdown() {
    save_state();
    for p in plugs() {
        p.close_editor();
    }
}

// ---------- finding plugins ----------

fn program_files() -> String {
    std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into())
}

fn vst_folders() -> Vec<String> {
    let pf = program_files();
    let mut dirs = vec![
        format!(r"{pf}\VSTPlugins"),
        format!(r"{pf}\Steinberg\VSTPlugins"),
        format!(r"{pf}\Common Files\VST2"),
        format!(r"{pf}\Common Files\Steinberg\VST2"),
        format!(r"{pf}\Vstplugins"),
    ];
    // the path plugin installers write
    if let Some(p) = util::reg_open_hklm64(r"SOFTWARE\VST").and_then(|k| k.string("VSTPluginsPath")).filter(|p| !p.is_empty()) {
        dirs.insert(0, p);
    }
    dirs
}

fn vst3_folder() -> String {
    format!(r"{}\Common Files\VST3", program_files())
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn is_ext(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext))
}

/// Walks dir up to 3 levels deep; f gets every entry (path, is a folder) and returns whether to go
/// into a folder.
fn walk(dir: &Path, depth: usize, out_len: &dyn Fn() -> usize, f: &mut dyn FnMut(&Path, bool) -> bool) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out_len() >= 300 {
            return;
        }
        let p = e.path();
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if f(&p, is_dir) && is_dir && depth < 3 {
            walk(&p, depth + 1, out_len, f);
        }
    }
}

/// Plugins found in the usual VST folders.
pub fn scan() -> Vec<Found> {
    let out = RefCell::new(Vec::new());
    let len = || out.borrow().len();
    // VST3: bundles (Name.vst3 folders) and single .vst3 files
    walk(Path::new(&vst3_folder()), 0, &len, &mut |p, is_dir| {
        if !is_ext(p, "vst3") {
            return true;
        }
        let bin = if is_dir { vst3::binary(&p.to_string_lossy()).ok() } else { Some(p.to_path_buf()) };
        if bin.is_some_and(|b| is_pe64(&b) == Ok(true)) {
            out.borrow_mut().push(Found { path: p.to_string_lossy().into_owned(), name: format!("{} (VST3)", stem(p)) });
        }
        false
    });
    let mut seen = HashSet::new();
    for d in vst_folders() {
        walk(Path::new(&d), 0, &len, &mut |p, is_dir| {
            if is_dir {
                return true;
            }
            let key = p.to_string_lossy().to_lowercase();
            if !is_ext(p, "dll") || !seen.insert(key) {
                return false;
            }
            if is_pe64(p) == Ok(true) {
                out.borrow_mut().push(Found { path: p.to_string_lossy().into_owned(), name: stem(p) });
            }
            false
        });
    }
    out.into_inner()
}

/// (UI thread) Pick a plugin file and add it.
pub fn pick() {
    let mut buf = vec![0u16; 1024];
    let flt: Vec<u16> = "VST (*.dll, *.vst3)\0*.dll;*.vst3\0\0".encode_utf16().collect();
    let title = HSTRING::from(t("Оберіть VST-плагін (.dll або .vst3)"));
    let dir = std::iter::once(vst3_folder()).chain(vst_folders()).find(|d| Path::new(d).is_dir()).map(HSTRING::from);
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(flt.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrInitialDir: dir.as_ref().map(|d| PCWSTR(d.as_ptr())).unwrap_or(PCWSTR::null()),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_EXPLORER,
        ..Default::default()
    };
    if !unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
        return;
    }
    let p = util::from_wide(&buf);
    if !p.is_empty() {
        add_slot(&p);
    }
}

fn new_slot_id() -> String {
    format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0))
}

/// (UI thread) A new plugin at the end of the chain.
fn add_slot(path: &str) {
    let path = path.to_string();
    config::set(|c| c.mic_fx.plugins.push(VstSlot { id: new_slot_id(), path, on: true, state: String::new() }));
    sync();
}

/// add | remove | toggle | edit | up | down
pub fn op(v: &serde_json::Value) {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let (op, id, path) = (s("op"), s("id"), s("path"));
    let on = v.get("on").and_then(|x| x.as_bool()).unwrap_or(false);
    match op.as_str() {
        "add" => {
            // a .dll/.vst3 file or a .vst3 bundle folder
            let p = Path::new(&path);
            if p.is_file() || (p.is_dir() && is_ext(p, "vst3")) {
                util::run_on_ui(move || add_slot(&path));
            }
        }
        "edit" => util::run_on_ui(move || edit(&id)),
        "remove" | "toggle" | "up" | "down" => {
            config::set(|c| {
                let l = &mut c.mic_fx.plugins;
                if let Some(i) = l.iter().position(|x| x.id == id) {
                    match op.as_str() {
                        "remove" => {
                            l.remove(i);
                        }
                        "toggle" => l[i].on = on,
                        "up" if i > 0 => l.swap(i - 1, i),
                        "down" if i + 1 < l.len() => l.swap(i + 1, i),
                        _ => {}
                    }
                }
            });
            util::post_ui(WM_CFG_CHG, 0);
        }
        _ => {}
    }
}

// ---------- base64 (the saved plugin state) ----------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(B64[(n >> 18) as usize & 63] as char);
        s.push(B64[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    s
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.as_bytes();
    if s.len() % 4 != 0 {
        return None;
    }
    let val = |c: u8| -> Option<u32> { B64.iter().position(|&x| x == c).map(|p| p as u32) };
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for (ci, c) in s.chunks(4).enumerate() {
        let last = ci == s.len() / 4 - 1;
        let pad = c.iter().rev().take_while(|&&x| x == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for &x in &c[..4 - pad] {
            n = n << 6 | val(x)?;
        }
        n <<= 6 * pad as u32;
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for b in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7]] {
            let e = base64_encode(b);
            assert_eq!(base64_decode(&e).as_deref(), Some(b), "{e}");
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_decode("Zm9v!mFy"), None);
    }

    /// Real plugins (paths in ARCMIX_TEST_VST, separated by ';'): load, process, save and restore
    /// the state, unload. Run by hand: cargo test -- --ignored real_plugins
    #[test]
    #[ignore]
    fn real_plugins() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let paths = std::env::var("ARCMIX_TEST_VST").expect("ARCMIX_TEST_VST");
        for path in paths.split(';').filter(|p| !p.is_empty()) {
            let mut state = Vec::new();
            for (round, rate) in [(0, 48000u32), (1, 44100)] {
                set_rate(rate);
                let p = load("t", path, rate, &state).unwrap_or_else(|e| panic!("{path}: {e}"));
                let mut ph = 0f64;
                let mut peak = 0f32;
                for _ in 0..50 {
                    let mut buf = vec![0f32; 2 * 960];
                    for k in 0..960 {
                        let v = (0.5 * ph.sin()) as f32;
                        ph += 2.0 * std::f64::consts::PI * 440.0 / rate as f64;
                        buf[2 * k] = v;
                        buf[2 * k + 1] = v;
                    }
                    p.run(&mut buf, 960, 2);
                    assert!(buf.iter().all(|v| v.is_finite()), "{path}: not finite");
                    peak = buf.iter().fold(peak, |m, v| m.max(v.abs()));
                }
                if round == 0 {
                    state = p.chunk().unwrap_or_default();
                }
                println!("{} ({path}) @ {rate}: peak out {peak:.3}, editor {}, state {} bytes", p.name, p.has_editor(), state.len());
                assert!(peak > 0.01, "{path}: silence at the output");
                p.unload();
            }
        }
    }

    /// Plugin windows of real plugins (ARCMIX_TEST_VST): open, run the message loop a moment,
    /// check the size, close. Run by hand: cargo test -- --ignored real_editors
    #[test]
    #[ignore]
    fn real_editors() {
        use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetClientRect, MSG, PM_REMOVE, PeekMessageW, TranslateMessage};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let paths = std::env::var("ARCMIX_TEST_VST").expect("ARCMIX_TEST_VST");
        for path in paths.split(';').filter(|p| !p.is_empty()) {
            set_rate(48000);
            let p = load("t", path, 48000, &[]).unwrap_or_else(|e| panic!("{path}: {e}"));
            p.open_editor().unwrap_or_else(|e| panic!("{path}: editor: {e}"));
            let hw = HWND(p.editor.load(Ordering::Relaxed) as *mut _);
            let t = std::time::Instant::now();
            while t.elapsed() < Duration::from_millis(1500) {
                unsafe {
                    let mut m = MSG::default();
                    while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&m);
                        DispatchMessageW(&m);
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let mut rc = RECT::default();
            unsafe {
                let _ = GetClientRect(hw, &mut rc);
            }
            println!("{}: editor {}x{}", p.name, rc.right - rc.left, rc.bottom - rc.top);
            assert!(rc.right - rc.left > 100 && rc.bottom - rc.top > 100, "{path}: editor too small");
            p.close_editor();
            assert!(p.dirty.load(Ordering::Relaxed), "{path}: not marked for saving");
            p.unload();
        }
    }
}
