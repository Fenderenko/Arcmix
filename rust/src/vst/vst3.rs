//! VST3 plugin host (64-bit) for microphone processing — experimental, in the same chain as VST2.
//!
//! VST3 is built almost like COM: the module (a DLL with the .vst3 extension, usually inside a
//! bundle folder Name.vst3\Contents\x86_64-win\) exports GetPluginFactory; the factory creates
//! the component (audio processing) and the controller (parameters, window). The host implements
//! a few interfaces itself:
//!   IHostApplication — "who am I" and message creation;
//!   IComponentHandler — parameter changes in the plugin window (passed on to audio processing);
//!   IPlugFrame — the plugin asks to resize its window;
//!   IBStream — saving and restoring state;
//!   IMessage / IAttributeList — exchange between the component and the controller;
//!   IParameterChanges / IParamValueQueue — parameter changes for process().
//! Host objects are `Shell<T>`: a C++-style header (vtable, IID, reference count) followed by the
//! Rust data; temporary ones (messages, attribute lists, streams) are freed by their last Release.
//!
//! Structure layouts and method order checked against the VST3 SDK headers (MIT, Steinberg).

use std::{
    collections::HashMap,
    ffi::c_void,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicPtr, AtomicU32, AtomicU64, Ordering},
    },
};

use windows::{
    Win32::{
        Foundation::{FreeLibrary, HMODULE},
        System::LibraryLoader::{GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW},
    },
    core::{HSTRING, PCSTR},
};

use super::{EFF_FLAGS_HAS_EDITOR, VST_BLOCK, c_str, is_pe64, size_editor};
use crate::{APP_NAME, logf};

// ---------- ids and constants ----------

/// DECLARE_CLASS_IID(l1, l2, l3, l4) in the COM-compatible byte order (Windows).
const fn iid(l1: u32, l2: u32, l3: u32, l4: u32) -> [u8; 16] {
    let a = l1.to_le_bytes();
    let b = ((l2 >> 16) as u16).to_le_bytes();
    let c = (l2 as u16).to_le_bytes();
    let d = l3.to_be_bytes();
    let e = l4.to_be_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], e[0], e[1], e[2], e[3]]
}

type Iid = [u8; 16];

const IID_FUNKNOWN: Iid = iid(0x00000000, 0x00000000, 0xC0000000, 0x00000046);
const IID_FACTORY2: Iid = iid(0x0007B650, 0xF24B4C0B, 0xA464EDB9, 0xF00B2ABB);
const IID_COMPONENT: Iid = iid(0xE831FF31, 0xF2D54301, 0x928EBBEE, 0x25697802);
const IID_AUDIO_PROCESSOR: Iid = iid(0x42043F99, 0xB7DA453C, 0xA569E79D, 0x9AAEC33D);
const IID_EDIT_CONTROLLER: Iid = iid(0xDCD7BBE3, 0x7742448D, 0xA874AACC, 0x979C759E);
const IID_CONNECTION_POINT: Iid = iid(0x70A4156F, 0x6E6E4026, 0x989148BF, 0xAA60D8D1);
const IID_MESSAGE: Iid = iid(0x936F033B, 0xC6C047DB, 0xBB0882F8, 0x13C1E613);
const IID_ATTRIBUTE_LIST: Iid = iid(0x1E5F0AEB, 0xCC7F4533, 0xA2544011, 0x38AD5EE4);
const IID_HOST_APP: Iid = iid(0x58E595CC, 0xDB2D4969, 0x8B6AAF8C, 0x36A664E5);
const IID_COMP_HANDLER: Iid = iid(0x93A0BEA3, 0x0BD045DB, 0x8E890B0C, 0xC1E46AC6);
const IID_BSTREAM: Iid = iid(0xC3BF6EA2, 0x30994752, 0x9B6BF990, 0x1EE33E9B);
const IID_PARAM_CHANGES: Iid = iid(0xA4779663, 0x0BB64A56, 0xB44384A8, 0x466FEB9D);
const IID_PARAM_QUEUE: Iid = iid(0x01263A18, 0xED074F6F, 0x98C9D356, 0x4686F9BA);
const IID_PLUG_FRAME: Iid = iid(0x367FAF01, 0xAFA94693, 0x8D4DA2A0, 0xED0882A3);

const K_OK: i32 = 0;
const K_FALSE: i32 = 1;
const K_NO_INTERFACE: i32 = 0x80004002u32 as i32;
const K_INVALID_ARG: i32 = 0x80070057u32 as i32;
const MAX_PARAMS: usize = 64;
const K_AUDIO: usize = 0;
const K_INPUT: usize = 0;
const K_OUTPUT: usize = 1;
const SPEAKER_MONO: u64 = 0x80000;
const SPEAKER_STEREO: u64 = 0x3;

// ---------- calling plugin objects ----------

/// A raw pointer to a plugin object (vtable at offset 0).
#[derive(Clone, Copy)]
struct Obj(*mut c_void);

impl Obj {
    const NULL: Obj = Obj(std::ptr::null_mut());

    fn is_null(self) -> bool {
        self.0.is_null()
    }

    unsafe fn method<F: Copy>(self, idx: usize) -> F {
        unsafe {
            let vt = *(self.0 as *const *const usize);
            std::mem::transmute_copy(&*vt.add(idx))
        }
    }

    unsafe fn call0(self, idx: usize) -> i32 {
        unsafe { self.method::<unsafe extern "system" fn(*mut c_void) -> i32>(idx)(self.0) }
    }

    unsafe fn call1(self, idx: usize, a: usize) -> i32 {
        unsafe { self.method::<unsafe extern "system" fn(*mut c_void, usize) -> i32>(idx)(self.0, a) }
    }

    unsafe fn call2(self, idx: usize, a: usize, b: usize) -> i32 {
        unsafe { self.method::<unsafe extern "system" fn(*mut c_void, usize, usize) -> i32>(idx)(self.0, a, b) }
    }

    unsafe fn call3(self, idx: usize, a: usize, b: usize, c: usize) -> i32 {
        unsafe { self.method::<unsafe extern "system" fn(*mut c_void, usize, usize, usize) -> i32>(idx)(self.0, a, b, c) }
    }

    unsafe fn call4(self, idx: usize, a: usize, b: usize, c: usize, d: usize) -> i32 {
        unsafe { self.method::<unsafe extern "system" fn(*mut c_void, usize, usize, usize, usize) -> i32>(idx)(self.0, a, b, c, d) }
    }

    /// A method returning a pointer.
    unsafe fn call1p(self, idx: usize, a: usize) -> Obj {
        unsafe { Obj(self.method::<unsafe extern "system" fn(*mut c_void, usize) -> *mut c_void>(idx)(self.0, a)) }
    }

    unsafe fn qi(self, iid: &Iid) -> Obj {
        if self.is_null() {
            return Obj::NULL;
        }
        let mut p = std::ptr::null_mut::<c_void>();
        unsafe {
            if self.call2(0, iid.as_ptr() as usize, &mut p as *mut _ as usize) >= 0 && !p.is_null() {
                return Obj(p);
            }
        }
        Obj::NULL
    }

    unsafe fn release(self) {
        if !self.is_null() {
            unsafe {
                self.call0(2);
            }
        }
    }
}

// ---------- host objects ----------

#[repr(C)]
struct Head {
    vtbl: *const c_void,
    iid: &'static Iid,
    refs: AtomicI32,
    /// frees a temporary object on its last Release; None — lives until the plugin is unloaded
    free: Option<unsafe fn(*mut Head)>,
}

#[repr(C)]
struct Shell<T> {
    head: Head,
    data: T,
}

unsafe fn free_shell<T>(h: *mut Head) {
    drop(unsafe { Box::from_raw(h as *mut Shell<T>) });
}

fn new_shell<T>(vtbl: *const c_void, iid: &'static Iid, data: T, temp: bool) -> *mut Head {
    let free: Option<unsafe fn(*mut Head)> = if temp { Some(free_shell::<T>) } else { None };
    Box::into_raw(Box::new(Shell { head: Head { vtbl, iid, refs: AtomicI32::new(1), free }, data })) as *mut Head
}

/// The Rust data of a host object.
unsafe fn data<'a, T>(this: *mut Head) -> &'a T {
    unsafe { &(*(this as *const Shell<T>)).data }
}

unsafe extern "system" fn qi(this: *mut Head, iid: *const Iid, obj: *mut *mut c_void) -> i32 {
    if obj.is_null() || iid.is_null() {
        return K_INVALID_ARG;
    }
    unsafe {
        let want = &*iid;
        if *want == IID_FUNKNOWN || want == (*this).iid {
            *obj = this as *mut c_void;
            add_ref(this);
            return K_OK;
        }
        *obj = std::ptr::null_mut();
    }
    K_NO_INTERFACE
}

unsafe extern "system" fn add_ref(this: *mut Head) -> u32 {
    unsafe { ((*this).refs.fetch_add(1, Ordering::AcqRel) + 1) as u32 }
}

unsafe extern "system" fn release(this: *mut Head) -> u32 {
    unsafe {
        let n = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if n == 0
            && let Some(f) = (*this).free
        {
            f(this);
        }
        n.max(0) as u32
    }
}

/// Frees a persistent host object (on unload).
unsafe fn drop_persistent<T>(h: *mut Head) {
    if !h.is_null() {
        unsafe { free_shell::<T>(h) };
    }
}

type Qi = unsafe extern "system" fn(*mut Head, *const Iid, *mut *mut c_void) -> i32;
type Ref = unsafe extern "system" fn(*mut Head) -> u32;

// --- IHostApplication ---

#[repr(C)]
struct HostVt {
    qi: Qi,
    add_ref: Ref,
    release: Ref,
    get_name: unsafe extern "system" fn(*mut Head, *mut u16) -> i32,
    create_instance: unsafe extern "system" fn(*mut Head, *const Iid, *const Iid, *mut *mut c_void) -> i32,
}

static HOST_VT: HostVt = HostVt { qi, add_ref, release, get_name: host_get_name, create_instance: host_create_instance };

struct SendPtr(*mut Head);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

static HOST: LazyLock<SendPtr> = LazyLock::new(|| SendPtr(new_shell(&HOST_VT as *const _ as *const c_void, &IID_HOST_APP, (), false)));

unsafe extern "system" fn host_get_name(_this: *mut Head, name: *mut u16) -> i32 {
    if name.is_null() {
        return K_INVALID_ARG;
    }
    let dst = unsafe { std::slice::from_raw_parts_mut(name, 128) };
    let mut n = 0;
    for (d, c) in dst[..127].iter_mut().zip(APP_NAME.encode_utf16()) {
        *d = c;
        n += 1;
    }
    dst[n] = 0;
    K_OK
}

unsafe extern "system" fn host_create_instance(_this: *mut Head, cid: *const Iid, iid: *const Iid, obj: *mut *mut c_void) -> i32 {
    if obj.is_null() || cid.is_null() || iid.is_null() {
        return K_INVALID_ARG;
    }
    unsafe {
        let (c, i) = (&*cid, &*iid);
        if *c == IID_MESSAGE && *i == IID_MESSAGE {
            *obj = new_message() as *mut c_void;
            return K_OK;
        }
        if *c == IID_ATTRIBUTE_LIST && *i == IID_ATTRIBUTE_LIST {
            *obj = new_attrs() as *mut c_void;
            return K_OK;
        }
        *obj = std::ptr::null_mut();
    }
    K_NO_INTERFACE
}

// --- IComponentHandler (the plugin window changes parameters) ---

#[repr(C)]
struct HandlerVt {
    qi: Qi,
    add_ref: Ref,
    release: Ref,
    begin_edit: unsafe extern "system" fn(*mut Head, u32) -> i32,
    perform_edit: unsafe extern "system" fn(*mut Head, u32, f64) -> i32,
    end_edit: unsafe extern "system" fn(*mut Head, u32) -> i32,
    restart_component: unsafe extern "system" fn(*mut Head, i32) -> i32,
}

static HANDLER_VT: HandlerVt = HandlerVt { qi, add_ref, release, begin_edit, perform_edit, end_edit, restart_component };

/// What the handler shares with the plugin.
struct HandlerData {
    pend: Arc<Mutex<Vec<(u32, f64)>>>,
    dirty: Arc<AtomicBool>,
}

unsafe extern "system" fn begin_edit(_this: *mut Head, _id: u32) -> i32 {
    K_OK
}

unsafe extern "system" fn perform_edit(this: *mut Head, id: u32, value: f64) -> i32 {
    let d = unsafe { data::<HandlerData>(this) };
    queue_param(&d.pend, id, value);
    K_OK
}

unsafe extern "system" fn end_edit(this: *mut Head, _id: u32) -> i32 {
    unsafe { data::<HandlerData>(this) }.dirty.store(true, Ordering::Relaxed);
    K_OK
}

unsafe extern "system" fn restart_component(this: *mut Head, _flags: i32) -> i32 {
    unsafe { data::<HandlerData>(this) }.dirty.store(true, Ordering::Relaxed);
    K_OK
}

fn queue_param(pend: &Mutex<Vec<(u32, f64)>>, id: u32, val: f64) {
    let mut p = pend.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(x) = p.iter_mut().find(|x| x.0 == id) {
        x.1 = val;
    } else if p.len() < MAX_PARAMS {
        p.push((id, val));
    }
}

// --- IPlugFrame ---

#[repr(C)]
struct FrameVt {
    qi: Qi,
    add_ref: Ref,
    release: Ref,
    resize_view: unsafe extern "system" fn(*mut Head, *mut c_void, *mut [i32; 4]) -> i32,
}

static FRAME_VT: FrameVt = FrameVt { qi, add_ref, release, resize_view };

struct FrameData {
    editor: Arc<AtomicIsize>,
}

unsafe extern "system" fn resize_view(this: *mut Head, view: *mut c_void, rect: *mut [i32; 4]) -> i32 {
    if rect.is_null() || view.is_null() {
        return K_INVALID_ARG;
    }
    unsafe {
        let r = &*rect; // left, top, right, bottom
        let hw = data::<FrameData>(this).editor.load(Ordering::Relaxed);
        size_editor(hw, r[2] - r[0], r[3] - r[1]);
        Obj(view).call1(10, rect as usize); // onSize
    }
    K_OK
}

// --- IMessage ---

#[repr(C)]
struct MessageVt {
    qi: Qi,
    add_ref: Ref,
    release: Ref,
    get_message_id: unsafe extern "system" fn(*mut Head) -> *const u8,
    set_message_id: unsafe extern "system" fn(*mut Head, *const u8),
    get_attributes: unsafe extern "system" fn(*mut Head) -> *mut Head,
}

static MESSAGE_VT: MessageVt = MessageVt { qi, add_ref, release, get_message_id, set_message_id, get_attributes };

#[derive(Default)]
struct MsgState {
    /// with a terminating zero
    id: Vec<u8>,
    attrs: Option<SendPtr>,
}

impl Drop for MsgState {
    fn drop(&mut self) {
        if let Some(a) = self.attrs.take() {
            unsafe { release(a.0) };
        }
    }
}

fn new_message() -> *mut Head {
    new_shell(&MESSAGE_VT as *const _ as *const c_void, &IID_MESSAGE, Mutex::new(MsgState::default()), true)
}

unsafe extern "system" fn get_message_id(this: *mut Head) -> *const u8 {
    let m = unsafe { data::<Mutex<MsgState>>(this) }.lock().unwrap_or_else(|e| e.into_inner());
    if m.id.is_empty() { std::ptr::null() } else { m.id.as_ptr() }
}

unsafe extern "system" fn set_message_id(this: *mut Head, id: *const u8) {
    let mut s = unsafe { c_str_at(id, 1024) }.into_bytes();
    s.push(0);
    unsafe { data::<Mutex<MsgState>>(this) }.lock().unwrap_or_else(|e| e.into_inner()).id = s;
}

unsafe extern "system" fn get_attributes(this: *mut Head) -> *mut Head {
    let mut m = unsafe { data::<Mutex<MsgState>>(this) }.lock().unwrap_or_else(|e| e.into_inner());
    // without AddRef — as in the SDK
    m.attrs.get_or_insert_with(|| SendPtr(new_attrs())).0
}

// --- IAttributeList ---

#[repr(C)]
struct AttrVt {
    qi: Qi,
    add_ref: Ref,
    release: Ref,
    set_int: unsafe extern "system" fn(*mut Head, *const u8, i64) -> i32,
    get_int: unsafe extern "system" fn(*mut Head, *const u8, *mut i64) -> i32,
    set_float: unsafe extern "system" fn(*mut Head, *const u8, f64) -> i32,
    get_float: unsafe extern "system" fn(*mut Head, *const u8, *mut f64) -> i32,
    set_string: unsafe extern "system" fn(*mut Head, *const u8, *const u16) -> i32,
    get_string: unsafe extern "system" fn(*mut Head, *const u8, *mut u16, u32) -> i32,
    set_binary: unsafe extern "system" fn(*mut Head, *const u8, *const u8, u32) -> i32,
    get_binary: unsafe extern "system" fn(*mut Head, *const u8, *mut *const u8, *mut u32) -> i32,
}

static ATTR_VT: AttrVt = AttrVt { qi, add_ref, release, set_int, get_int, set_float, get_float, set_string, get_string, set_binary, get_binary };

enum Val {
    Int(i64),
    Float(f64),
    Str(Vec<u16>),
    Bin(Vec<u8>),
}

type Attrs = Mutex<HashMap<String, Val>>;

fn new_attrs() -> *mut Head {
    new_shell(&ATTR_VT as *const _ as *const c_void, &IID_ATTRIBUTE_LIST, Attrs::default(), true)
}

unsafe fn attr_set(this: *mut Head, id: *const u8, v: Val) -> i32 {
    let k = unsafe { c_str_at(id, 256) };
    if k.is_empty() {
        return K_INVALID_ARG;
    }
    unsafe { data::<Attrs>(this) }.lock().unwrap_or_else(|e| e.into_inner()).insert(k, v);
    K_OK
}

/// Runs f on the value when there is one of the right kind; K_FALSE otherwise.
unsafe fn attr_get(this: *mut Head, id: *const u8, f: impl FnOnce(&Val) -> bool) -> i32 {
    let k = unsafe { c_str_at(id, 256) };
    let m = unsafe { data::<Attrs>(this) }.lock().unwrap_or_else(|e| e.into_inner());
    match m.get(&k) {
        Some(v) if f(v) => K_OK,
        _ => K_FALSE,
    }
}

unsafe extern "system" fn set_int(this: *mut Head, id: *const u8, value: i64) -> i32 {
    unsafe { attr_set(this, id, Val::Int(value)) }
}

unsafe extern "system" fn get_int(this: *mut Head, id: *const u8, out: *mut i64) -> i32 {
    if out.is_null() {
        return K_FALSE;
    }
    unsafe {
        attr_get(this, id, |v| match v {
            Val::Int(i) => {
                *out = *i;
                true
            }
            _ => false,
        })
    }
}

unsafe extern "system" fn set_float(this: *mut Head, id: *const u8, value: f64) -> i32 {
    unsafe { attr_set(this, id, Val::Float(value)) }
}

unsafe extern "system" fn get_float(this: *mut Head, id: *const u8, out: *mut f64) -> i32 {
    if out.is_null() {
        return K_FALSE;
    }
    unsafe {
        attr_get(this, id, |v| match v {
            Val::Float(f) => {
                *out = *f;
                true
            }
            _ => false,
        })
    }
}

unsafe extern "system" fn set_string(this: *mut Head, id: *const u8, s: *const u16) -> i32 {
    let mut v = Vec::new();
    if !s.is_null() {
        for i in 0..1usize << 20 {
            let c = unsafe { *s.add(i) };
            if c == 0 {
                break;
            }
            v.push(c);
        }
    }
    unsafe { attr_set(this, id, Val::Str(v)) }
}

unsafe extern "system" fn get_string(this: *mut Head, id: *const u8, out: *mut u16, size: u32) -> i32 {
    let n = size as usize / 2;
    if out.is_null() || n < 1 {
        return K_FALSE;
    }
    unsafe {
        attr_get(this, id, |v| match v {
            Val::Str(s) => {
                let dst = std::slice::from_raw_parts_mut(out, n);
                let k = s.len().min(n - 1);
                dst[..k].copy_from_slice(&s[..k]);
                dst[k] = 0;
                true
            }
            _ => false,
        })
    }
}

unsafe extern "system" fn set_binary(this: *mut Head, id: *const u8, d: *const u8, size: u32) -> i32 {
    let b = if d.is_null() || size == 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(d, size as usize) }.to_vec() };
    unsafe { attr_set(this, id, Val::Bin(b)) }
}

unsafe extern "system" fn get_binary(this: *mut Head, id: *const u8, d: *mut *const u8, size: *mut u32) -> i32 {
    if d.is_null() || size.is_null() {
        return K_FALSE;
    }
    unsafe {
        attr_get(this, id, |v| match v {
            Val::Bin(b) => {
                // lives as long as the attribute list
                *d = if b.is_empty() { std::ptr::null() } else { b.as_ptr() };
                *size = b.len() as u32;
                true
            }
            _ => false,
        })
    }
}

// --- IBStream ---

#[repr(C)]
struct StreamVt {
    qi: Qi,
    add_ref: Ref,
    release: Ref,
    read: unsafe extern "system" fn(*mut Head, *mut u8, i32, *mut i32) -> i32,
    write: unsafe extern "system" fn(*mut Head, *const u8, i32, *mut i32) -> i32,
    seek: unsafe extern "system" fn(*mut Head, i64, i32, *mut i64) -> i32,
    tell: unsafe extern "system" fn(*mut Head, *mut i64) -> i32,
}

static STREAM_VT: StreamVt = StreamVt { qi, add_ref, release, read: stream_read, write: stream_write, seek: stream_seek, tell: stream_tell };

#[derive(Default)]
struct StreamState {
    b: Vec<u8>,
    pos: i64,
}

type Stream = Mutex<StreamState>;

fn new_stream(b: Vec<u8>) -> *mut Head {
    new_shell(&STREAM_VT as *const _ as *const c_void, &IID_BSTREAM, Mutex::new(StreamState { b, pos: 0 }), true)
}

unsafe fn stream_bytes(s: *mut Head) -> Vec<u8> {
    unsafe { data::<Stream>(s) }.lock().unwrap_or_else(|e| e.into_inner()).b.clone()
}

unsafe extern "system" fn stream_read(this: *mut Head, buf: *mut u8, n: i32, n_read: *mut i32) -> i32 {
    let mut st = unsafe { data::<Stream>(this) }.lock().unwrap_or_else(|e| e.into_inner());
    let avail = st.b.len() as i64 - st.pos;
    let want = (n as i64).min(avail).max(0);
    if want > 0 && !buf.is_null() {
        let p = st.pos as usize;
        unsafe { std::ptr::copy_nonoverlapping(st.b[p..].as_ptr(), buf, want as usize) };
        st.pos += want;
    }
    if !n_read.is_null() {
        unsafe { *n_read = want as i32 };
    }
    K_OK
}

unsafe extern "system" fn stream_write(this: *mut Head, buf: *const u8, n: i32, n_written: *mut i32) -> i32 {
    let cnt = n as i64;
    if cnt < 0 || (cnt > 0 && buf.is_null()) || cnt > 256 << 20 {
        return K_INVALID_ARG;
    }
    let mut g = unsafe { data::<Stream>(this) }.lock().unwrap_or_else(|e| e.into_inner());
    let st = &mut *g;
    let end = (st.pos + cnt) as usize;
    if end > st.b.len() {
        st.b.resize(end, 0);
    }
    if cnt > 0 {
        let p = st.pos as usize;
        st.b[p..end].copy_from_slice(unsafe { std::slice::from_raw_parts(buf, cnt as usize) });
    }
    st.pos += cnt;
    if !n_written.is_null() {
        unsafe { *n_written = cnt as i32 };
    }
    K_OK
}

unsafe extern "system" fn stream_seek(this: *mut Head, pos: i64, mode: i32, result: *mut i64) -> i32 {
    let mut st = unsafe { data::<Stream>(this) }.lock().unwrap_or_else(|e| e.into_inner());
    let p = match mode {
        1 => pos + st.pos,           // from the current position
        2 => pos + st.b.len() as i64, // from the end
        _ => pos,
    };
    st.pos = p.clamp(0, st.b.len() as i64);
    if !result.is_null() {
        unsafe { *result = st.pos };
    }
    K_OK
}

unsafe extern "system" fn stream_tell(this: *mut Head, out: *mut i64) -> i32 {
    if out.is_null() {
        return K_INVALID_ARG;
    }
    unsafe { *out = data::<Stream>(this).lock().unwrap_or_else(|e| e.into_inner()).pos };
    K_OK
}

// --- IParameterChanges / IParamValueQueue (audio thread; plain data in the objects) ---

#[repr(C)]
struct ChangesVt {
    qi: Qi,
    add_ref: Ref,
    release: Ref,
    get_parameter_count: unsafe extern "system" fn(*mut Head) -> i32,
    get_parameter_data: unsafe extern "system" fn(*mut Head, i32) -> *mut Head,
    add_parameter_data: unsafe extern "system" fn(*mut Head, *const u32, *mut i32) -> *mut Head,
}

static CHANGES_VT: ChangesVt = ChangesVt { qi, add_ref, release, get_parameter_count, get_parameter_data, add_parameter_data };

struct ChangesData {
    count: AtomicI32,
    /// MAX_PARAMS queues in a row
    queues: AtomicPtr<Shell<QueueData>>,
}

unsafe extern "system" fn get_parameter_count(this: *mut Head) -> i32 {
    unsafe { data::<ChangesData>(this) }.count.load(Ordering::Relaxed)
}

unsafe extern "system" fn get_parameter_data(this: *mut Head, index: i32) -> *mut Head {
    let d = unsafe { data::<ChangesData>(this) };
    if index < 0 || index >= d.count.load(Ordering::Relaxed) {
        return std::ptr::null_mut();
    }
    unsafe { d.queues.load(Ordering::Relaxed).add(index as usize) as *mut Head }
}

unsafe extern "system" fn add_parameter_data(_this: *mut Head, _id: *const u32, _index: *mut i32) -> *mut Head {
    std::ptr::null_mut()
}

#[repr(C)]
struct QueueVt {
    qi: Qi,
    add_ref: Ref,
    release: Ref,
    get_parameter_id: unsafe extern "system" fn(*mut Head) -> u32,
    get_point_count: unsafe extern "system" fn(*mut Head) -> i32,
    get_point: unsafe extern "system" fn(*mut Head, i32, *mut i32, *mut f64) -> i32,
    add_point: unsafe extern "system" fn(*mut Head, i32, f64, *mut i32) -> i32,
}

static QUEUE_VT: QueueVt = QueueVt { qi, add_ref, release, get_parameter_id, get_point_count, get_point, add_point };

#[derive(Default)]
struct QueueData {
    id: AtomicU32,
    /// f64 bits
    value: AtomicU64,
}

unsafe extern "system" fn get_parameter_id(this: *mut Head) -> u32 {
    unsafe { data::<QueueData>(this) }.id.load(Ordering::Relaxed)
}

unsafe extern "system" fn get_point_count(_this: *mut Head) -> i32 {
    1
}

unsafe extern "system" fn get_point(this: *mut Head, index: i32, offset: *mut i32, value: *mut f64) -> i32 {
    if index != 0 || offset.is_null() || value.is_null() {
        return K_FALSE;
    }
    unsafe {
        *offset = 0;
        *value = f64::from_bits(data::<QueueData>(this).value.load(Ordering::Relaxed));
    }
    K_OK
}

unsafe extern "system" fn add_point(_this: *mut Head, _offset: i32, _value: f64, _index: *mut i32) -> i32 {
    K_FALSE
}

/// Reads a C string byte by byte (not past the terminating zero — memory beyond may be unmapped).
unsafe fn c_str_at(p: *const u8, max: usize) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut b = Vec::new();
    for i in 0..max {
        let c = unsafe { *p.add(i) };
        if c == 0 {
            break;
        }
        b.push(c);
    }
    String::from_utf8_lossy(&b).into_owned()
}

// ---------- the plugin ----------

#[repr(C)]
struct AudioBusBuffers {
    num_channels: i32,
    silence_flags: u64,
    buffers: *mut *mut f32,
}

#[repr(C)]
struct ProcessData {
    process_mode: i32,
    sample_size: i32,
    num_samples: i32,
    num_inputs: i32,
    num_outputs: i32,
    inputs: *mut AudioBusBuffers,
    outputs: *mut AudioBusBuffers,
    in_changes: *mut Head,
    out_changes: *mut c_void,
    in_events: *mut c_void,
    out_events: *mut c_void,
    ctx: *mut ProcessContext,
}

/// ProcessContext (only the fields the host fills).
#[repr(C)]
#[derive(Default)]
struct ProcessContext {
    state: u32,
    sample_rate: f64,
    project_time_samples: i64,
    system_time: i64,
    cont_time_samples: i64,
    project_time_music: f64,
    bar_position_music: f64,
    cycle_start_music: f64,
    cycle_end_music: f64,
    tempo: f64,
    time_sig_num: i32,
    time_sig_den: i32,
    rest: [i32; 10],
}

#[repr(C)]
struct ProcessSetup {
    process_mode: i32,
    sample_size: i32,
    max_samples: i32,
    sample_rate: f64,
}

/// State used by the audio thread only (and by the UI thread while the audio thread does not use
/// the plugin: setup, configuration).
struct Audio {
    /// owned here: ProcessData points at them
    _in_bus: Vec<AudioBusBuffers>,
    _out_bus: Vec<AudioBusBuffers>,
    /// per-bus arrays of channel pointers and the channel memory
    _ptrs: Vec<Vec<*mut f32>>,
    _chans: Vec<Vec<f32>>,
    in_main: Vec<*mut f32>,
    out_main: Vec<*mut f32>,
    aux_in: Vec<*mut f32>,
    changes: Box<Shell<ChangesData>>,
    queues: Box<[Shell<QueueData>]>,
    pdata: Box<ProcessData>,
    ctx: Box<ProcessContext>,
    samples: i64,
}

pub(super) struct V3 {
    lib: HMODULE,
    factory: Obj,
    comp: Obj,
    ctrl: Obj,
    proc_: Obj,
    comp_cp: Obj,
    ctrl_cp: Obj,
    same_ctrl: bool,
    handler: *mut Head,
    frame: *mut Head,
    exit_dll: Option<unsafe extern "system" fn() -> isize>,
    audio: std::cell::UnsafeCell<Option<Audio>>,
    rate: AtomicU32,
    active: AtomicBool,
    processing: AtomicBool,
    view: AtomicPtr<c_void>,
    pend: Arc<Mutex<Vec<(u32, f64)>>>,
    dirty: Arc<AtomicBool>,
    unloaded: AtomicBool,
}

// Plugin objects are used from the UI thread and the audio thread as the VST3 threading rules allow.
unsafe impl Send for V3 {}
unsafe impl Sync for V3 {}

/// The DLL inside a bundle (a Name.vst3 folder) or the file itself.
pub(super) fn binary(path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    let md = std::fs::metadata(p).map_err(|e| e.to_string())?;
    if !md.is_dir() {
        return Ok(p.to_path_buf());
    }
    let dir = p.join("Contents").join("x86_64-win");
    if let Some(name) = p.file_name() {
        let f = dir.join(name);
        if f.exists() {
            return Ok(f);
        }
    }
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("vst3")) {
                return Ok(e.path());
            }
        }
    }
    Err("not a VST3 plugin".into())
}

pub(super) struct Loaded {
    pub v3: V3,
    pub name: String,
    pub flags: i32,
}

/// UI thread only.
pub(super) fn load(path: &str, rate: u32, state: &[u8], dirty: Arc<AtomicBool>, editor: Arc<AtomicIsize>) -> Result<Loaded, String> {
    let file = binary(path)?;
    if !is_pe64(&file)? {
        return Err("32-bit".into());
    }
    let lib = unsafe { LoadLibraryExW(&HSTRING::from(file.as_os_str()), None, LOAD_WITH_ALTERED_SEARCH_PATH) }.map_err(|e| format!("LoadLibrary: {e}"))?;
    let mut v = V3 {
        lib,
        factory: Obj::NULL,
        comp: Obj::NULL,
        ctrl: Obj::NULL,
        proc_: Obj::NULL,
        comp_cp: Obj::NULL,
        ctrl_cp: Obj::NULL,
        same_ctrl: false,
        handler: std::ptr::null_mut(),
        frame: std::ptr::null_mut(),
        exit_dll: None,
        audio: std::cell::UnsafeCell::new(None),
        rate: AtomicU32::new(rate),
        active: AtomicBool::new(false),
        processing: AtomicBool::new(false),
        view: AtomicPtr::new(std::ptr::null_mut()),
        pend: Arc::new(Mutex::new(Vec::with_capacity(MAX_PARAMS))),
        dirty,
        unloaded: AtomicBool::new(false),
    };
    match unsafe { v.init(path, rate, state, editor) } {
        Ok((name, flags)) => Ok(Loaded { v3: v, name, flags }),
        Err(e) => {
            v.unload();
            Err(e)
        }
    }
}

impl V3 {
    fn sym(&self, name: &[u8]) -> Option<unsafe extern "system" fn() -> isize> {
        unsafe { GetProcAddress(self.lib, PCSTR(name.as_ptr())) }
    }

    unsafe fn init(&mut self, path: &str, rate: u32, state: &[u8], editor: Arc<AtomicIsize>) -> Result<(String, i32), String> {
        let host = HOST.0 as usize;
        unsafe {
            if let Some(f) = self.sym(b"InitDll\0") {
                let ok: unsafe extern "system" fn() -> u8 = std::mem::transmute(f);
                if ok() == 0 {
                    return Err("InitDll failed".into());
                }
                self.exit_dll = self.sym(b"ExitDll\0");
            }
            let Some(gf) = self.sym(b"GetPluginFactory\0") else { return Err("not a VST3 plugin".into()) };
            let gf: unsafe extern "system" fn() -> *mut c_void = std::mem::transmute(gf);
            self.factory = Obj(gf());
            if self.factory.is_null() {
                return Err("not a VST3 plugin".into());
            }

            // an effect class: "Audio Module Class", not an instrument
            let mut cid = [0u8; 16];
            let mut found = None;
            let f2 = self.factory.qi(&IID_FACTORY2);
            let n = self.factory.call0(4);
            for i in 0..n.max(0) as usize {
                if !f2.is_null() {
                    let mut ci = [0u8; 440];
                    if f2.call2(7, i, ci.as_mut_ptr() as usize) >= 0 && c_str(&ci[20..52]) == "Audio Module Class" && !c_str(&ci[120..248]).contains("Instrument") {
                        cid.copy_from_slice(&ci[..16]);
                        found = Some(c_str(&ci[52..116]));
                        break;
                    }
                    continue;
                }
                let mut ci = [0u8; 116];
                if self.factory.call2(5, i, ci.as_mut_ptr() as usize) >= 0 && c_str(&ci[20..52]) == "Audio Module Class" {
                    cid.copy_from_slice(&ci[..16]);
                    found = Some(c_str(&ci[52..116]));
                    break;
                }
            }
            f2.release();
            let Some(mut name) = found else { return Err("not an effect".into()) };
            if name.is_empty() {
                name = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            }

            let mut comp = std::ptr::null_mut::<c_void>();
            if self.factory.call3(6, cid.as_ptr() as usize, IID_COMPONENT.as_ptr() as usize, &mut comp as *mut _ as usize) < 0 || comp.is_null() {
                return Err("cannot create plugin".into());
            }
            let comp = Obj(comp);
            if comp.call1(3, host) < 0 {
                // initialize
                comp.release();
                return Err("cannot create plugin".into());
            }
            self.comp = comp;
            // the controller: the same object or a separate class
            self.ctrl = comp.qi(&IID_EDIT_CONTROLLER);
            if !self.ctrl.is_null() {
                self.same_ctrl = true;
            } else {
                let mut ccid = [0u8; 16];
                if comp.call1(5, ccid.as_mut_ptr() as usize) >= 0 && ccid != [0u8; 16] {
                    let mut ctrl = std::ptr::null_mut::<c_void>();
                    if self.factory.call3(6, ccid.as_ptr() as usize, IID_EDIT_CONTROLLER.as_ptr() as usize, &mut ctrl as *mut _ as usize) >= 0 && !ctrl.is_null() {
                        let c = Obj(ctrl);
                        if c.call1(3, host) < 0 {
                            c.release();
                        } else {
                            self.ctrl = c;
                        }
                    }
                }
                if !self.ctrl.is_null() {
                    self.comp_cp = comp.qi(&IID_CONNECTION_POINT);
                    self.ctrl_cp = self.ctrl.qi(&IID_CONNECTION_POINT);
                    if !self.comp_cp.is_null() && !self.ctrl_cp.is_null() {
                        self.comp_cp.call1(3, self.ctrl_cp.0 as usize);
                        self.ctrl_cp.call1(3, self.comp_cp.0 as usize);
                    }
                }
            }
            let mut flags = 0;
            if !self.ctrl.is_null() {
                self.handler = new_shell(&HANDLER_VT as *const _ as *const c_void, &IID_COMP_HANDLER, HandlerData { pend: self.pend.clone(), dirty: self.dirty.clone() }, false);
                self.frame = new_shell(&FRAME_VT as *const _ as *const c_void, &IID_PLUG_FRAME, FrameData { editor }, false);
                self.ctrl.call1(16, self.handler as usize); // setComponentHandler
                flags |= EFF_FLAGS_HAS_EDITOR;
            }
            self.proc_ = comp.qi(&IID_AUDIO_PROCESSOR);
            if self.proc_.is_null() {
                return Err("not an effect".into());
            }
            if self.proc_.call1(5, 0) != K_OK {
                // canProcessSampleSize(kSample32)
                return Err("unsupported (no 32-bit float)".into());
            }
            if !state.is_empty() {
                self.restore_state(state);
            }
            self.setup_buses()?;
            self.configure(rate);
            if !self.active.load(Ordering::Relaxed) {
                return Err("cannot activate plugin".into());
            }
            let a = (*self.audio.get()).as_ref().map_or((0, 0), |a| (a.in_main.len(), a.out_main.len()));
            logf!("vst3: loaded {name} ({path}), in {} ch, out {} ch, editor {}", a.0, a.1, !self.ctrl.is_null());
            Ok((name, flags))
        }
    }

    /// Stereo (or mono) on the main buses; memory for the channels of all buses.
    unsafe fn setup_buses(&mut self) -> Result<(), String> {
        unsafe {
            let n_in = self.comp.call2(7, K_AUDIO, K_INPUT);
            let n_out = self.comp.call2(7, K_AUDIO, K_OUTPUT);
            if !(1..=16).contains(&n_in) || !(1..=16).contains(&n_out) {
                return Err("not an effect".into());
            }
            let (n_in, n_out) = (n_in as usize, n_out as usize);
            let mut ins = vec![0u64; n_in];
            let mut outs = vec![0u64; n_out];
            let p = self.proc_;
            let read = |ins: &mut [u64], outs: &mut [u64]| {
                for (i, a) in ins.iter_mut().enumerate() {
                    p.call3(4, K_INPUT, i, a as *mut u64 as usize);
                }
                for (i, a) in outs.iter_mut().enumerate() {
                    p.call3(4, K_OUTPUT, i, a as *mut u64 as usize);
                }
            };
            read(&mut ins, &mut outs);
            for arr in [SPEAKER_STEREO, SPEAKER_MONO] {
                ins[0] = arr;
                outs[0] = arr;
                if p.call4(3, ins.as_mut_ptr() as usize, n_in, outs.as_mut_ptr() as usize, n_out) == K_OK {
                    break;
                }
            }
            read(&mut ins, &mut outs); // what the plugin really took
            self.comp.call4(10, K_AUDIO, K_INPUT, 0, 1); // activateBus of the main buses
            self.comp.call4(10, K_AUDIO, K_OUTPUT, 0, 1);

            let mut ptrs: Vec<Vec<*mut f32>> = Vec::new();
            let mut chans: Vec<Vec<f32>> = Vec::new();
            let (mut in_main, mut out_main, mut aux_in) = (Vec::new(), Vec::new(), Vec::new());
            let mut bus = |arrs: &[u64], main: &mut Vec<*mut f32>, mut aux: Option<&mut Vec<*mut f32>>| -> Vec<AudioBusBuffers> {
                let mut out = Vec::with_capacity(arrs.len());
                for (i, &a) in arrs.iter().enumerate() {
                    let nc = a.count_ones() as usize;
                    let mut pa: Vec<*mut f32> = Vec::with_capacity(nc);
                    for _ in 0..nc {
                        let mut ch = vec![0f32; VST_BLOCK];
                        let cp = ch.as_mut_ptr();
                        chans.push(ch);
                        pa.push(cp);
                        if i == 0 {
                            main.push(cp);
                        } else if let Some(x) = aux.as_deref_mut() {
                            x.push(cp);
                        }
                    }
                    let bp = if nc == 0 { std::ptr::null_mut() } else { pa.as_mut_ptr() };
                    ptrs.push(pa);
                    out.push(AudioBusBuffers { num_channels: nc as i32, silence_flags: 0, buffers: bp });
                }
                out
            };
            let mut in_bus = bus(&ins, &mut in_main, Some(&mut aux_in));
            let mut out_bus = bus(&outs, &mut out_main, None);
            if in_main.is_empty() || out_main.is_empty() {
                return Err("not an effect".into());
            }
            // parameter changes: the IParameterChanges object and the queue array
            let mut queues: Box<[Shell<QueueData>]> = (0..MAX_PARAMS)
                .map(|_| Shell { head: Head { vtbl: &QUEUE_VT as *const _ as *const c_void, iid: &IID_PARAM_QUEUE, refs: AtomicI32::new(1), free: None }, data: QueueData::default() })
                .collect();
            let mut changes = Box::new(Shell {
                head: Head { vtbl: &CHANGES_VT as *const _ as *const c_void, iid: &IID_PARAM_CHANGES, refs: AtomicI32::new(1), free: None },
                data: ChangesData { count: AtomicI32::new(0), queues: AtomicPtr::new(queues.as_mut_ptr()) },
            });
            // ProcessContext: tempo and time signature (some plugins need them), time — a running counter
            let mut ctx = Box::new(ProcessContext {
                state: 0x400 | 0x2000 | 0x20000, // kTempoValid | kTimeSigValid | kContTimeValid
                tempo: 120.0,
                time_sig_num: 4,
                time_sig_den: 4,
                ..Default::default()
            });
            let pdata = Box::new(ProcessData {
                process_mode: 0,
                sample_size: 0,
                num_samples: 0,
                num_inputs: n_in as i32,
                num_outputs: n_out as i32,
                inputs: in_bus.as_mut_ptr(),
                outputs: out_bus.as_mut_ptr(),
                in_changes: &mut changes.head as *mut Head,
                out_changes: std::ptr::null_mut(),
                in_events: std::ptr::null_mut(),
                out_events: std::ptr::null_mut(),
                ctx: &mut *ctx as *mut ProcessContext,
            });
            *self.audio.get() = Some(Audio { _in_bus: in_bus, _out_bus: out_bus, _ptrs: ptrs, _chans: chans, in_main, out_main, aux_in, changes, queues, pdata, ctx, samples: 0 });
            Ok(())
        }
    }

    /// The sample rate (UI thread; the audio thread does not use the plugin meanwhile).
    pub(super) fn configure(&self, rate: u32) {
        let active = self.active.load(Ordering::Relaxed);
        if self.dead() || (active && self.rate.load(Ordering::Relaxed) == rate) {
            return;
        }
        unsafe {
            if self.processing.swap(false, Ordering::AcqRel) {
                self.proc_.call1(8, 0);
            }
            if active {
                self.comp.call1(11, 0);
                self.active.store(false, Ordering::Relaxed);
            }
            let setup = ProcessSetup { process_mode: 0, sample_size: 0, max_samples: VST_BLOCK as i32, sample_rate: rate as f64 }; // kRealtime, kSample32
            if self.proc_.call1(7, &setup as *const _ as usize) < 0 {
                logf!("vst3: setupProcessing failed ({rate} Hz)");
            }
            if let Some(a) = (*self.audio.get()).as_mut() {
                a.ctx.sample_rate = rate as f64;
            }
            self.rate.store(rate, Ordering::Relaxed);
            if self.comp.call1(11, 1) >= 0 {
                // setActive(true)
                self.active.store(true, Ordering::Relaxed);
            } else {
                logf!("vst3: setActive failed");
            }
        }
    }

    /// Processes n ≤ VST_BLOCK frames in place (microphone thread).
    pub(super) fn run(&self, buf: &mut [f32], n: usize, ch: usize) {
        if !self.active.load(Ordering::Relaxed) || n == 0 {
            return;
        }
        unsafe {
            let Some(a) = (*self.audio.get()).as_mut() else { return };
            if !self.processing.load(Ordering::Acquire) {
                self.proc_.call1(8, 1); // setProcessing(true) — on the audio thread
                self.processing.store(true, Ordering::Release);
            }
            for (c, &p) in a.in_main.iter().enumerate() {
                let dst = std::slice::from_raw_parts_mut(p, n);
                let src = c.min(ch - 1);
                for (k, d) in dst.iter_mut().enumerate() {
                    *d = buf[k * ch + src];
                }
            }
            for &p in &a.aux_in {
                std::slice::from_raw_parts_mut(p, n).fill(0.0);
            }
            let mut cnt = 0;
            // changes from the plugin window; when busy they are picked up in the next block
            if let Ok(mut pend) = self.pend.try_lock() {
                for &(id, v) in pend.iter() {
                    let q = &a.queues[cnt].data;
                    q.id.store(id, Ordering::Relaxed);
                    q.value.store(v.to_bits(), Ordering::Relaxed);
                    cnt += 1;
                }
                pend.clear();
            }
            a.changes.data.count.store(cnt as i32, Ordering::Relaxed);
            a.pdata.num_samples = n as i32;
            a.ctx.cont_time_samples = a.samples;
            self.proc_.call1(9, &mut *a.pdata as *mut ProcessData as usize); // process
            a.samples += n as i64;
            for c in 0..ch {
                let src = std::slice::from_raw_parts(a.out_main[c.min(a.out_main.len() - 1)], n);
                for (k, &s) in src.iter().enumerate() {
                    buf[k * ch + c] = s;
                }
            }
        }
    }

    /// State: "VST3" + length + component state + length + controller state.
    pub(super) fn save_state(&self) -> Option<Vec<u8>> {
        if self.dead() {
            return None;
        }
        let get = |obj: Obj, idx: usize| -> Vec<u8> {
            if obj.is_null() {
                return Vec::new();
            }
            unsafe {
                let s = new_stream(Vec::new());
                let r = if obj.call1(idx, s as usize) < 0 { Vec::new() } else { stream_bytes(s) };
                release(s);
                r
            }
        };
        let cs = get(self.comp, 13);
        let ks = get(self.ctrl, 7);
        if cs.is_empty() && ks.is_empty() {
            return None;
        }
        let mut out = b"VST3".to_vec();
        out.extend((cs.len() as u32).to_le_bytes());
        out.extend(&cs);
        out.extend((ks.len() as u32).to_le_bytes());
        out.extend(&ks);
        Some(out)
    }

    fn restore_state(&self, b: &[u8]) {
        if b.len() < 12 || &b[..4] != b"VST3" {
            return;
        }
        let n = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
        if 8 + n + 4 > b.len() {
            return;
        }
        let cs = &b[8..8 + n];
        let rest = &b[8 + n..];
        let m = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        if 4 + m > rest.len() {
            return;
        }
        let ks = &rest[4..4 + m];
        let put = |obj: Obj, idx: usize, d: &[u8]| unsafe {
            let s = new_stream(d.to_vec());
            obj.call1(idx, s as usize);
            release(s);
        };
        if !cs.is_empty() {
            put(self.comp, 12, cs); // component.setState
            if !self.ctrl.is_null() {
                put(self.ctrl, 5, cs); // controller.setComponentState
            }
        }
        if !ks.is_empty() && !self.ctrl.is_null() {
            put(self.ctrl, 6, ks); // controller.setState
        }
    }

    // ---------- window ----------

    pub(super) fn attach_view(&self, hw: isize) -> Result<(i32, i32), String> {
        if self.ctrl.is_null() || self.dead() {
            return Err("no editor".into());
        }
        unsafe {
            let view = self.ctrl.call1p(17, b"editor\0".as_ptr() as usize); // createView
            if view.is_null() {
                return Err("no editor".into());
            }
            let hwnd_type = b"HWND\0".as_ptr() as usize;
            if view.call1(3, hwnd_type) != K_OK {
                view.release();
                return Err("no editor".into());
            }
            view.call1(12, self.frame as usize); // setFrame
            if view.call2(4, hw as usize, hwnd_type) != K_OK {
                view.call1(12, 0);
                view.release();
                return Err("attach failed".into());
            }
            self.view.store(view.0, Ordering::Relaxed);
            let mut r = [0i32; 4];
            if view.call1(9, r.as_mut_ptr() as usize) == K_OK {
                return Ok((r[2] - r[0], r[3] - r[1]));
            }
            Ok((0, 0))
        }
    }

    pub(super) fn detach_view(&self) {
        let view = Obj(self.view.swap(std::ptr::null_mut(), Ordering::Relaxed));
        if view.is_null() {
            return;
        }
        unsafe {
            view.call0(5); // removed
            view.call1(12, 0);
            view.release();
        }
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// UI thread; the audio thread no longer uses the plugin. Runs once; the object is dead after.
    pub(super) fn unload(&self) {
        if self.unloaded.swap(true, Ordering::AcqRel) {
            return;
        }
        self.detach_view();
        unsafe {
            if self.processing.swap(false, Ordering::AcqRel) {
                self.proc_.call1(8, 0);
            }
            if self.active.swap(false, Ordering::AcqRel) {
                self.comp.call1(11, 0);
            }
            if !self.comp_cp.is_null() && !self.ctrl_cp.is_null() {
                self.comp_cp.call1(4, self.ctrl_cp.0 as usize);
                self.ctrl_cp.call1(4, self.comp_cp.0 as usize);
            }
            self.comp_cp.release();
            self.ctrl_cp.release();
            if !self.ctrl.is_null() {
                self.ctrl.call1(16, 0);
                if !self.same_ctrl {
                    self.ctrl.call0(4); // terminate
                }
                self.ctrl.release();
            }
            self.proc_.release();
            if !self.comp.is_null() {
                self.comp.call0(4); // terminate
                self.comp.release();
            }
            self.factory.release();
            if let Some(f) = self.exit_dll {
                f();
            }
            if !self.lib.is_invalid() {
                let _ = FreeLibrary(self.lib);
            }
            drop_persistent::<HandlerData>(self.handler);
            drop_persistent::<FrameData>(self.frame);
            *self.audio.get() = None;
        }
    }

    fn dead(&self) -> bool {
        self.unloaded.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    /// The VST3 IID macro gives the same bytes as a Windows GUID.
    #[test]
    fn iid_byte_order() {
        let g = windows::core::GUID::from_values(0xE831FF31, 0xF2D5, 0x4301, [0x92, 0x8E, 0xBB, 0xEE, 0x25, 0x69, 0x78, 0x02]);
        let b: [u8; 16] = unsafe { std::mem::transmute(g) };
        assert_eq!(super::IID_COMPONENT, b);
    }
}