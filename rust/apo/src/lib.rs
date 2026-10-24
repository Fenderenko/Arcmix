//! The Arcmix module for the Windows audio engine (an APO, loaded by audiodg.exe): the equalizer of
//! everything that plays on an output device — no cables, no delay. Installed by the app (elevated) as
//! a system effect of chosen devices; the curves come from the app through files (shared.rs).
//!
//! Real-time rules: APOProcess never allocates, locks or waits; the curve is copied under a seqlock and
//! the filter coefficients are recomputed only when it changes. Panics abort (Cargo.toml): nothing
//! unwinds into the audio engine.
//!
//! The engine counts every failed CoCreateInstance, IsInputFormatSupported, IsOutputFormatSupported
//! and LockForProcess and, after 10, switches the device's effects off: the module accepts every
//! uncompressed format and processes 32-bit float only, passing anything else unchanged.

mod shared;

use std::{
    cell::UnsafeCell,
    ffi::c_void,
    io::Write,
    path::PathBuf,
    sync::{
        Once,
        atomic::{AtomicIsize, AtomicPtr, AtomicU32, AtomicUsize, Ordering},
    },
    time::SystemTime,
};

use shared::{BANDS, ENTRIES, Entry, GUID_LEN, Shared};
use windows::{
    Win32::{
        Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_NOINTERFACE, E_OUTOFMEMORY, E_POINTER, HANDLE, S_FALSE, S_OK, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::{
            Apo::{
                APO_CONNECTION_DESCRIPTOR, APO_CONNECTION_PROPERTY, APO_FLAG, APO_REG_PROPERTIES, APOERR_FORMAT_NOT_SUPPORTED, APOInitSystemEffects, BUFFER_SILENT, IAudioMediaType,
                IAudioProcessingObject, IAudioProcessingObject_Vtbl, IAudioProcessingObjectConfiguration, IAudioProcessingObjectConfiguration_Vtbl, IAudioProcessingObjectRT,
                IAudioProcessingObjectRT_Vtbl, IAudioSystemEffects, IAudioSystemEffects_Vtbl, UNCOMPRESSEDAUDIOFORMAT,
            },
            PKEY_AudioEndpoint_GUID,
        },
        Storage::FileSystem::{FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FindFirstChangeNotificationW, FindNextChangeNotification},
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemAlloc, CoTaskMemFree, IClassFactory, IClassFactory_Impl, StructuredStorage::PropVariantToStringAlloc},
            SystemInformation::GetTickCount,
            Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects},
        },
    },
    core::{BOOL, GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, Interface, Ref, Result, implement},
};

/// The class of the module (registered by the app's installer under this CLSID).
pub const CLSID_ARCMIX_APO: GUID = GUID::from_u128(0xa7c3e2f1_5b4d_4e8a_9c21_3f6d8b0a1e57);
/// Channels processed (more are passed unchanged).
const MAXCH: usize = 8;
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
/// APO_FLAG_FRAMESPERSECOND_MUST_MATCH | APO_FLAG_BITSPERSAMPLE_MUST_MATCH; not in place: a hosted effect may
/// not support it, separate buffers every effect does
const FLAGS: i32 = 0x4 | 0x8;

// ---------- files and errors ----------

/// %ProgramData%\Arcmix: the curves, the liveness and the log.
fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into())).join(shared::DIR)
}

/// Errors only, into %ProgramData%\Arcmix\apo.log (the installer opens that folder to the audio
/// engine). Never written from APOProcess; stops growing at 64 KB.
fn log(msg: std::fmt::Arguments) {
    let path = data_dir().join("apo.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 65536) {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} pid {}: {msg}", unsafe { GetTickCount() }, std::process::id());
    }
}

macro_rules! logm {
    ($($a:tt)*) => { log(format_args!($($a)*)) };
}

// ---------- the curves and the liveness ----------

static SHARED: AtomicPtr<Shared> = AtomicPtr::new(std::ptr::null_mut());
/// instances between LockForProcess and UnlockForProcess
static LOCKED: AtomicUsize = AtomicUsize::new(0);
/// wakes the file thread (an auto-reset event): processing started
static WAKE: AtomicIsize = AtomicIsize::new(0);
static START: Once = Once::new();

/// The module's copy, once made (for the processing thread: it never makes it).
fn shared_now() -> Option<&'static Shared> {
    let p = SHARED.load(Ordering::Acquire);
    (!p.is_null()).then(|| unsafe { &*p })
}

/// Makes the module's copy (zeroed: no curves, no instances) and starts the file thread, once per
/// process; kept for the life of the process.
fn shared() -> Option<&'static Shared> {
    START.call_once(|| {
        // SAFETY: all-zero bytes are a valid Shared (atomics and plain numbers)
        let p = unsafe { std::alloc::alloc_zeroed(std::alloc::Layout::new::<Shared>()) } as *mut Shared;
        if p.is_null() {
            return;
        }
        SHARED.store(p, Ordering::Release);
        if let Ok(ev) = unsafe { CreateEventW(None, false, false, None) } {
            WAKE.store(ev.0 as isize, Ordering::Release);
        }
        let sh = unsafe { &*p };
        if std::thread::Builder::new().name("arcmix-apo-files".into()).spawn(move || files(sh)).is_err() {
            logm!("files: no thread");
        }
    });
    shared_now()
}

fn wake_files() {
    let ev = WAKE.load(Ordering::Acquire);
    if ev != 0 {
        let _ = unsafe { SetEvent(HANDLE(ev as *mut c_void)) };
    }
}

/// The file thread: takes curves.bin whenever it changes (a change notification on the folder) and,
/// while sound is processed, writes live.bin every 2 s; with nothing to process it sleeps.
fn files(sh: &'static Shared) {
    let dir = data_dir();
    let (curves, live) = (dir.join(shared::CURVES), dir.join(shared::LIVE));
    let mut stamp: Option<SystemTime> = None;
    take_curves(sh, &curves, &mut stamp);
    let note = unsafe { FindFirstChangeNotificationW(&HSTRING::from(dir.as_os_str()), false, FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME) };
    if let Err(e) = &note {
        logm!("files: no change notification ({e}), reading the curves every 2 s");
    }
    let mut handles = Vec::new();
    let wake = WAKE.load(Ordering::Acquire);
    if wake != 0 {
        handles.push(HANDLE(wake as *mut c_void));
    }
    let note_at = note.as_ref().ok().map(|n| {
        handles.push(*n);
        handles.len() - 1
    });
    loop {
        let busy = LOCKED.load(Ordering::Acquire) > 0;
        let timeout = if busy || note_at.is_none() || wake == 0 { 2000 } else { INFINITE };
        let r = if handles.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(2000));
            WAIT_TIMEOUT.0
        } else {
            unsafe { WaitForMultipleObjects(&handles, false, timeout) }.0
        };
        if r == WAIT_FAILED.0 {
            // never spin
            std::thread::sleep(std::time::Duration::from_millis(2000));
        }
        if note_at.is_some_and(|i| r == WAIT_OBJECT_0.0 + i as u32) {
            // the folder changed: our own live.bin as well, so only the curves are looked at
            if let Ok(n) = &note {
                let _ = unsafe { FindNextChangeNotification(*n) };
            }
            take_curves(sh, &curves, &mut stamp);
            continue;
        }
        if note_at.is_none() {
            take_curves(sh, &curves, &mut stamp);
        }
        if LOCKED.load(Ordering::Acquire) > 0 {
            let items: Vec<(u64, u32)> = sh.seen.iter().map(|s| (s.key.load(Ordering::Acquire), s.tick.load(Ordering::Acquire))).filter(|(k, _)| *k != 0).collect();
            let _ = std::fs::write(&live, shared::encode_live(&items));
        }
    }
}

/// Reads curves.bin if it changed since the last time and publishes it under the seqlock.
fn take_curves(sh: &Shared, path: &std::path::Path, stamp: &mut Option<SystemTime>) {
    let Ok(m) = std::fs::metadata(path).and_then(|m| m.modified()) else { return };
    if *stamp == Some(m) {
        return;
    }
    let Some(entries) = std::fs::read(path).ok().and_then(|b| shared::decode_curves(&b)) else { return };
    *stamp = Some(m);
    let seq = sh.seq.load(Ordering::Acquire);
    sh.seq.store(seq.wrapping_add(1) | 1, Ordering::Release); // odd: being written
    let base = sh.entries.as_ptr() as *mut Entry;
    for (i, e) in entries.iter().enumerate() {
        unsafe { std::ptr::write_volatile(base.add(i), *e) };
    }
    sh.count.store(entries.len() as u32, Ordering::Release);
    sh.seq.store((seq.wrapping_add(1) | 1).wrapping_add(1), Ordering::Release); // even: done
}

// ---------- the filters ----------

#[derive(Clone, Copy, Default)]
struct Coef {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

/// RBJ peaking filter (Audio EQ Cookbook), normalised; None where it would do nothing.
fn peaking(fs: f64, f0: f64, gain_db: f64, q: f64) -> Option<Coef> {
    // the values come from a file: anything not finite or beyond ±30 dB is left out
    if !gain_db.is_finite() || gain_db.abs() > 30.0 || gain_db.abs() < 0.01 || !(f0 > 10.0 && f0 < fs * 0.49) || !(q > 0.05 && q < 50.0) {
        return None;
    }
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = std::f64::consts::TAU * f0 / fs;
    let (s, c) = w0.sin_cos();
    let alpha = s / (2.0 * q);
    let a0 = 1.0 + alpha / a;
    Some(Coef { b0: (1.0 + alpha * a) / a0, b1: -2.0 * c / a0, b2: (1.0 - alpha * a) / a0, a1: -2.0 * c / a0, a2: (1.0 - alpha / a) / a0 })
}

struct St {
    guid: [u16; GUID_LEN],
    key: u64,
    ch: usize,
    rate: f64,
    locked: bool,
    /// 32-bit float: processed; anything else passes unchanged
    float: bool,
    /// bytes of one frame (for copying between separate buffers)
    frame_bytes: usize,
    /// the seqlock value of the curve in use (odd: none yet)
    seq: u32,
    on: bool,
    pre: f32,
    n: usize,
    co: [Coef; BANDS],
    /// transposed direct form II state per channel and filter
    z: [[[f64; 2]; BANDS]; MAXCH],
    slot: usize,
    stamp: u32,
    /// the effect the module took the slot from: it runs first, the curve after it
    inner: Option<Inner>,
}

/// The effect a driver (or Windows) had in the slot, hosted by the module.
struct Inner {
    apo: IAudioProcessingObject,
    rt: IAudioProcessingObjectRT,
    cfg: IAudioProcessingObjectConfiguration,
}

impl Inner {
    /// The effect wrapped on this endpoint (wrap.txt), created and initialised with the engine's data.
    fn open(endpoint: &str, size: u32, data: *const u8) -> Option<Inner> {
        let text = std::fs::read_to_string(data_dir().join(shared::WRAP)).ok()?;
        let s = shared::wrap_lookup(&text, endpoint)?;
        let clsid = GUID::try_from(s.trim_matches(|c| c == '{' || c == '}')).ok()?;
        if clsid == CLSID_ARCMIX_APO {
            return None;
        }
        let made = (|| -> Result<Inner> {
            let apo: IAudioProcessingObject = unsafe { CoCreateInstance(&clsid, None::<&IUnknown>, CLSCTX_INPROC_SERVER) }?;
            let inner = Inner { rt: apo.cast()?, cfg: apo.cast()?, apo };
            // the raw call: the engine's block goes on as it is (null with size 0 included)
            unsafe { (inner.apo.vtable().Initialize)(inner.apo.as_raw(), size, data) }.ok()?;
            Ok(inner)
        })();
        made.inspect_err(|e| logm!("wrap: {s} failed ({e}), the curve works alone")).ok()
    }
}

impl St {
    fn new() -> St {
        St { guid: [0; GUID_LEN], key: 0, ch: 2, rate: 48000.0, locked: false, float: false, frame_bytes: 0, seq: 1, on: false, pre: 1.0, n: 0, co: [Coef::default(); BANDS], z: [[[0.0; 2]; BANDS]; MAXCH], slot: ENTRIES, stamp: 0, inner: None }
    }

    /// Takes a new curve from the app when there is one (seqlock).
    fn refresh(&mut self, sh: &Shared) {
        let s1 = sh.seq.load(Ordering::Acquire);
        if s1 & 1 == 1 || s1 == self.seq || self.key == 0 {
            return;
        }
        let count = (sh.count.load(Ordering::Acquire) as usize).min(ENTRIES);
        let mut found: Option<Entry> = None;
        for i in 0..count {
            // a volatile copy: the app may be writing (the seqlock tells)
            let e = unsafe { std::ptr::read_volatile(&sh.entries[i] as *const Entry) };
            if shared::key_of(&e.guid) == self.key {
                found = Some(e);
                break;
            }
        }
        if sh.seq.load(Ordering::Acquire) != s1 {
            return; // written meanwhile: next buffer
        }
        self.seq = s1;
        let Some(e) = found.filter(|e| e.on != 0) else {
            self.on = false;
            return;
        };
        let mut n = 0;
        for i in 0..(e.n as usize).min(BANDS) {
            if let Some(c) = peaking(self.rate, e.freq[i] as f64, e.gain_db[i] as f64, e.q[i] as f64) {
                self.co[n] = c;
                n += 1;
            }
        }
        // a changed set of filters starts from rest; the same set keeps its state (no click while dragging)
        if n != self.n {
            self.z = [[[0.0; 2]; BANDS]; MAXCH];
        }
        self.n = n;
        let pre = if e.preamp_db.is_finite() { e.preamp_db.clamp(-30.0, 0.0) } else { 0.0 };
        self.pre = 10f32.powf(pre / 20.0);
        self.on = true;
    }

    /// Marks this endpoint as processed (about twice a second).
    fn alive(&mut self, sh: &Shared) {
        let now = unsafe { GetTickCount() };
        if now.wrapping_sub(self.stamp) < 500 || self.key == 0 {
            return;
        }
        self.stamp = now;
        if self.slot >= ENTRIES {
            for (i, s) in sh.seen.iter().enumerate() {
                let k = s.key.load(Ordering::Acquire);
                if k == self.key || (k == 0 && s.key.compare_exchange(0, self.key, Ordering::AcqRel, Ordering::Acquire).is_ok()) {
                    self.slot = i;
                    break;
                }
            }
        }
        if let Some(s) = sh.seen.get(self.slot) {
            s.tick.store(now, Ordering::Release);
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        let ch = self.ch.max(1);
        let pch = ch.min(MAXCH);
        let (pre, n) = (self.pre, self.n);
        for frame in buf.chunks_exact_mut(ch) {
            for (c, s) in frame.iter_mut().take(pch).enumerate() {
                let mut x = (*s * pre) as f64;
                for (co, z) in self.co[..n].iter().zip(self.z[c][..n].iter_mut()) {
                    let y = co.b0 * x + z[0];
                    z[0] = co.b1 * x - co.a1 * y + z[1];
                    z[1] = co.b2 * x - co.a2 * y;
                    x = y;
                }
                // denormals in a tail of silence cost much CPU
                *s = if x.abs() < 1e-20 { 0.0 } else { x as f32 };
            }
        }
    }
}

// ---------- formats ----------

/// A format for the log: subtype, container bytes, channels, rate.
fn describe(t: &IAudioMediaType) -> String {
    let mut f = UNCOMPRESSEDAUDIOFORMAT::default();
    match unsafe { t.GetUncompressedAudioFormat(&mut f) } {
        Ok(()) => format!("subtype {:08x}, {} bytes, {} ch, {} Hz", f.guidFormatType.data1, f.dwBytesPerSampleContainer, f.dwSamplesPerFrame, f.fFramesPerSecond),
        Err(e) => format!("not uncompressed ({e})"),
    }
}

/// An uncompressed format with at least one channel: accepted (processed only when float, see is_float).
fn plain_format(t: &IAudioMediaType) -> Option<UNCOMPRESSEDAUDIOFORMAT> {
    let mut f = UNCOMPRESSEDAUDIOFORMAT::default();
    unsafe { t.GetUncompressedAudioFormat(&mut f) }.ok()?;
    (f.dwSamplesPerFrame >= 1 && f.dwBytesPerSampleContainer >= 1).then_some(f)
}

/// 32-bit float PCM: the only format the module processes.
fn is_float(f: &UNCOMPRESSEDAUDIOFORMAT) -> bool {
    f.guidFormatType == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT && f.dwBytesPerSampleContainer == 4
}

// ---------- the APO: a COM object written out by hand ----------

/// The audio engine creates a system effect inside its own wrapper (COM aggregation): the effect's
/// interfaces pass QueryInterface, AddRef and Release to the controlling unknown — the wrapper, or the
/// object's own inner unknown when it is created alone. `#[implement]` cannot be aggregated, and an
/// effect that refuses is a failed CoCreateInstance for the engine.
#[repr(C)]
struct Obj {
    apo: &'static IAudioProcessingObject_Vtbl,
    rt: &'static IAudioProcessingObjectRT_Vtbl,
    cfg: &'static IAudioProcessingObjectConfiguration_Vtbl,
    sfx: &'static IAudioSystemEffects_Vtbl,
    /// the non-delegating unknown: it alone owns the object
    inner: &'static IUnknown_Vtbl,
    /// the controlling unknown, not referenced (it owns this object)
    outer: *mut c_void,
    refs: AtomicU32,
    st: UnsafeCell<St>,
}

const OFF_APO: usize = std::mem::offset_of!(Obj, apo);
const OFF_RT: usize = std::mem::offset_of!(Obj, rt);
const OFF_CFG: usize = std::mem::offset_of!(Obj, cfg);
const OFF_SFX: usize = std::mem::offset_of!(Obj, sfx);
const OFF_INNER: usize = std::mem::offset_of!(Obj, inner);

impl Obj {
    /// The object behind an interface pointer at that field offset.
    unsafe fn of<'a>(this: *mut c_void, off: usize) -> &'a Obj {
        unsafe { &*((this as *const u8).sub(off) as *const Obj) }
    }

    #[allow(clippy::mut_from_ref)]
    fn st(&self) -> &mut St {
        // SAFETY: the engine calls configuration and processing of one instance from one thread at a time
        unsafe { &mut *self.st.get() }
    }

    fn at(&self, off: usize) -> *mut c_void {
        unsafe { (self as *const Obj as *const u8).add(off) as *mut c_void }
    }
}

impl Drop for Obj {
    // released while still locked: the file thread must not count it as processing
    fn drop(&mut self) {
        if self.st.get_mut().locked {
            LOCKED.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

unsafe fn unknown_vtbl<'a>(p: *mut c_void) -> &'a IUnknown_Vtbl {
    unsafe { &**(p as *const *const IUnknown_Vtbl) }
}

unsafe fn add_ref(p: *mut c_void) -> u32 {
    unsafe { (unknown_vtbl(p).AddRef)(p) }
}

// the delegating unknown of every interface

unsafe extern "system" fn d_query<const OFF: usize>(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    let outer = unsafe { Obj::of(this, OFF) }.outer;
    unsafe { (unknown_vtbl(outer).QueryInterface)(outer, iid, out) }
}

unsafe extern "system" fn d_add_ref<const OFF: usize>(this: *mut c_void) -> u32 {
    let outer = unsafe { Obj::of(this, OFF) }.outer;
    unsafe { add_ref(outer) }
}

unsafe extern "system" fn d_release<const OFF: usize>(this: *mut c_void) -> u32 {
    let outer = unsafe { Obj::of(this, OFF) }.outer;
    unsafe { (unknown_vtbl(outer).Release)(outer) }
}

const fn delegating<const OFF: usize>() -> IUnknown_Vtbl {
    IUnknown_Vtbl { QueryInterface: d_query::<OFF>, AddRef: d_add_ref::<OFF>, Release: d_release::<OFF> }
}

// the inner (non-delegating) unknown

unsafe extern "system" fn i_query(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    let o = unsafe { Obj::of(this, OFF_INNER) };
    let iid = unsafe { *iid };
    let off = if iid == IUnknown::IID {
        OFF_INNER
    } else if iid == IAudioProcessingObject::IID {
        OFF_APO
    } else if iid == IAudioProcessingObjectRT::IID {
        OFF_RT
    } else if iid == IAudioProcessingObjectConfiguration::IID {
        OFF_CFG
    } else if iid == IAudioSystemEffects::IID {
        OFF_SFX
    } else {
        unsafe { *out = std::ptr::null_mut() };
        return E_NOINTERFACE;
    };
    let p = o.at(off);
    unsafe {
        *out = p;
        // counted where the interface counts: the inner itself for IUnknown, else the controlling unknown
        add_ref(p);
    }
    S_OK
}

unsafe extern "system" fn i_add_ref(this: *mut c_void) -> u32 {
    unsafe { Obj::of(this, OFF_INNER) }.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn i_release(this: *mut c_void) -> u32 {
    let o = unsafe { Obj::of(this, OFF_INNER) };
    let n = o.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        drop(unsafe { Box::from_raw(o as *const Obj as *mut Obj) });
    }
    n
}

static INNER_VTBL: IUnknown_Vtbl = IUnknown_Vtbl { QueryInterface: i_query, AddRef: i_add_ref, Release: i_release };

// IAudioProcessingObject

unsafe extern "system" fn apo_reset(this: *mut c_void) -> HRESULT {
    let st = unsafe { Obj::of(this, OFF_APO) }.st();
    if let Some(i) = &st.inner {
        let _ = unsafe { i.apo.Reset() };
    }
    st.z = [[[0.0; 2]; BANDS]; MAXCH];
    S_OK
}

unsafe extern "system" fn apo_latency(this: *mut c_void, out: *mut i64) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // the curve adds none; the hosted effect may
    let st = unsafe { Obj::of(this, OFF_APO) }.st();
    unsafe { *out = st.inner.as_ref().and_then(|i| i.apo.GetLatency().ok()).unwrap_or(0) };
    S_OK
}

unsafe extern "system" fn apo_registration(_this: *mut c_void, out: *mut *mut APO_REG_PROPERTIES) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe {
        let p = CoTaskMemAlloc(std::mem::size_of::<APO_REG_PROPERTIES>()) as *mut APO_REG_PROPERTIES;
        if p.is_null() {
            return E_OUTOFMEMORY;
        }
        let mut r = APO_REG_PROPERTIES { clsid: CLSID_ARCMIX_APO, Flags: APO_FLAG(FLAGS), u32MajorVersion: 1, u32MinorVersion: 0, u32MinInputConnections: 1, u32MaxInputConnections: 1, u32MinOutputConnections: 1, u32MaxOutputConnections: 1, u32MaxInstances: u32::MAX, u32NumAPOInterfaces: 1, iidAPOInterfaceList: [IAudioProcessingObject::IID], ..Default::default() };
        for (d, c) in r.szFriendlyName.iter_mut().zip("Arcmix".encode_utf16()) {
            *d = c;
        }
        for (d, c) in r.szCopyrightInfo.iter_mut().zip("Fenderenko, GPL-3.0".encode_utf16()) {
            *d = c;
        }
        p.write(r);
        *out = p;
    }
    S_OK
}

unsafe extern "system" fn apo_initialize(this: *mut c_void, size: u32, data: *const u8) -> HRESULT {
    let st = unsafe { Obj::of(this, OFF_APO) }.st();
    // the system-effects form names the endpoint: its GUID picks this device's curve
    if !data.is_null() && size as usize >= std::mem::size_of::<APOInitSystemEffects>() {
        let init = unsafe { &*(data as *const APOInitSystemEffects) };
        if let Some(props) = init.pAPOEndpointProperties.as_ref() {
            if let Ok(v) = unsafe { props.GetValue(&PKEY_AudioEndpoint_GUID) } {
                if let Ok(p) = unsafe { PropVariantToStringAlloc(&v) } {
                    let s = unsafe { p.to_string() }.unwrap_or_default();
                    unsafe { CoTaskMemFree(Some(p.0 as *const c_void)) };
                    st.guid = shared::guid_field(&s);
                    st.key = shared::key_of(&st.guid);
                }
            }
        }
    }
    let endpoint = String::from_utf16_lossy(&st.guid).trim_end_matches('\0').to_string();
    if st.inner.is_none() && !endpoint.is_empty() {
        st.inner = Inner::open(&endpoint, size, data);
    }
    let _ = shared();
    S_OK
}

/// Any uncompressed format is accepted as it is (the requested one, referenced, goes back). With a
/// hosted effect its answer goes back as it is, a suggested format (S_FALSE) included.
unsafe fn format_answer(this: *mut c_void, output: bool, opposite: *mut c_void, requested: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || requested.is_null() {
        return E_POINTER;
    }
    let Some(r) = (unsafe { IAudioMediaType::from_raw_borrowed(&requested) }) else { return E_POINTER };
    if let Some(i) = &unsafe { Obj::of(this, OFF_APO) }.st().inner {
        let vt = i.apo.vtable();
        let f = if output { vt.IsOutputFormatSupported } else { vt.IsInputFormatSupported };
        return unsafe { f(i.apo.as_raw(), opposite, requested, out) };
    }
    if plain_format(r).is_some() {
        unsafe {
            add_ref(requested);
            *out = requested;
        }
        S_OK
    } else {
        unsafe { *out = std::ptr::null_mut() };
        APOERR_FORMAT_NOT_SUPPORTED
    }
}

unsafe extern "system" fn apo_input_format(this: *mut c_void, opposite: *mut c_void, requested: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    unsafe { format_answer(this, false, opposite, requested, out) }
}

unsafe extern "system" fn apo_output_format(this: *mut c_void, opposite: *mut c_void, requested: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    unsafe { format_answer(this, true, opposite, requested, out) }
}

unsafe extern "system" fn apo_channels(this: *mut c_void, out: *mut u32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let st = unsafe { Obj::of(this, OFF_APO) }.st();
    unsafe { *out = st.inner.as_ref().and_then(|i| i.apo.GetInputChannelCount().ok()).unwrap_or(st.ch as u32) };
    S_OK
}

static APO_VTBL: IAudioProcessingObject_Vtbl = IAudioProcessingObject_Vtbl {
    base__: delegating::<OFF_APO>(),
    Reset: apo_reset,
    GetLatency: apo_latency,
    GetRegistrationProperties: apo_registration,
    Initialize: apo_initialize,
    IsInputFormatSupported: apo_input_format,
    IsOutputFormatSupported: apo_output_format,
    GetInputChannelCount: apo_channels,
};

// IAudioProcessingObjectConfiguration

unsafe extern "system" fn cfg_lock(this: *mut c_void, nin: u32, pin: *const *const APO_CONNECTION_DESCRIPTOR, nout: u32, pout: *const *const APO_CONNECTION_DESCRIPTOR) -> HRESULT {
    if nin < 1 || nout < 1 || pin.is_null() || unsafe { (*pin).is_null() } {
        return E_POINTER;
    }
    let st = unsafe { Obj::of(this, OFF_CFG) }.st();
    if let Some(i) = &st.inner {
        let hr = unsafe { (i.cfg.vtable().LockForProcess)(i.cfg.as_raw(), nin, pin, nout, pout) };
        if hr.is_err() {
            // the curve keeps the sound going rather than failing the whole chain
            logm!("wrap: LockForProcess of the hosted effect failed ({hr:?}), the curve works alone");
            st.inner = None;
        }
    }
    // the curve works on the output: with a hosted effect that is its output
    let d = if !pout.is_null() && unsafe { !(*pout).is_null() } { unsafe { &**pout } } else { unsafe { &**pin } };
    let Some(f) = d.pFormat.as_ref().and_then(plain_format) else {
        logm!("LockForProcess: refused, {}", d.pFormat.as_ref().map_or("no format".into(), describe));
        return APOERR_FORMAT_NOT_SUPPORTED;
    };
    let _ = shared();
    st.ch = f.dwSamplesPerFrame as usize;
    st.rate = f.fFramesPerSecond as f64;
    st.float = is_float(&f);
    st.frame_bytes = f.dwSamplesPerFrame as usize * f.dwBytesPerSampleContainer as usize;
    st.z = [[[0.0; 2]; BANDS]; MAXCH];
    st.seq = 1; // the curve is read again for this rate
    if !st.locked {
        st.locked = true;
        LOCKED.fetch_add(1, Ordering::AcqRel);
    }
    wake_files();
    S_OK
}

unsafe extern "system" fn cfg_unlock(this: *mut c_void) -> HRESULT {
    let st = unsafe { Obj::of(this, OFF_CFG) }.st();
    if let Some(i) = &st.inner {
        let _ = unsafe { i.cfg.UnlockForProcess() };
    }
    if st.locked {
        st.locked = false;
        LOCKED.fetch_sub(1, Ordering::AcqRel);
    }
    S_OK
}

static CFG_VTBL: IAudioProcessingObjectConfiguration_Vtbl = IAudioProcessingObjectConfiguration_Vtbl { base__: delegating::<OFF_CFG>(), LockForProcess: cfg_lock, UnlockForProcess: cfg_unlock };

// IAudioProcessingObjectRT

unsafe extern "system" fn rt_process(this: *mut c_void, nin: u32, pin: *const *const APO_CONNECTION_PROPERTY, nout: u32, pout: *mut *mut APO_CONNECTION_PROPERTY) {
    if nin < 1 || nout < 1 || pin.is_null() || pout.is_null() {
        return;
    }
    let st = unsafe { Obj::of(this, OFF_RT) }.st();
    if let Some(inner) = &st.inner {
        // the hosted effect writes the output (frames, flags and samples)
        let (f, raw) = (inner.rt.vtable().APOProcess, inner.rt.as_raw());
        unsafe { f(raw, nin, pin, nout, pout) };
    } else {
        let (i, o) = unsafe { (&**pin, &mut **pout) };
        o.u32ValidFrameCount = i.u32ValidFrameCount;
        o.u32BufferFlags = i.u32BufferFlags;
        if st.locked && i.pBuffer != 0 && o.pBuffer != 0 && i.pBuffer != o.pBuffer {
            unsafe { std::ptr::copy_nonoverlapping(i.pBuffer as *const u8, o.pBuffer as *mut u8, i.u32ValidFrameCount as usize * st.frame_bytes) };
        }
    }
    let o = unsafe { &mut **pout };
    if !st.locked || o.pBuffer == 0 {
        return;
    }
    if let Some(sh) = shared_now() {
        st.refresh(sh);
        st.alive(sh);
    }
    if !st.float || !st.on || o.u32BufferFlags == BUFFER_SILENT {
        return;
    }
    let buf = unsafe { std::slice::from_raw_parts_mut(o.pBuffer as *mut f32, o.u32ValidFrameCount as usize * st.ch) };
    st.process(buf);
}

unsafe extern "system" fn rt_input_frames(this: *mut c_void, n: u32) -> u32 {
    match &unsafe { Obj::of(this, OFF_RT) }.st().inner {
        Some(i) => unsafe { i.rt.CalcInputFrames(n) },
        None => n,
    }
}

unsafe extern "system" fn rt_output_frames(this: *mut c_void, n: u32) -> u32 {
    match &unsafe { Obj::of(this, OFF_RT) }.st().inner {
        Some(i) => unsafe { i.rt.CalcOutputFrames(n) },
        None => n,
    }
}

static RT_VTBL: IAudioProcessingObjectRT_Vtbl = IAudioProcessingObjectRT_Vtbl { base__: delegating::<OFF_RT>(), APOProcess: rt_process, CalcInputFrames: rt_input_frames, CalcOutputFrames: rt_output_frames };

// IAudioSystemEffects: a mark, no methods

static SFX_VTBL: IAudioSystemEffects_Vtbl = IAudioSystemEffects_Vtbl { base__: delegating::<OFF_SFX>() };

/// A new instance answering `riid`; `outer` — the aggregating object, or null.
unsafe fn create(outer: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || riid.is_null() {
        return E_POINTER;
    }
    unsafe { *out = std::ptr::null_mut() };
    // COM: an aggregated object is asked for its inner unknown only
    if !outer.is_null() && unsafe { *riid } != IUnknown::IID {
        return CLASS_E_NOAGGREGATION;
    }
    let o = Box::into_raw(Box::new(Obj { apo: &APO_VTBL, rt: &RT_VTBL, cfg: &CFG_VTBL, sfx: &SFX_VTBL, inner: &INNER_VTBL, outer: std::ptr::null_mut(), refs: AtomicU32::new(1), st: UnsafeCell::new(St::new()) }));
    let inner = unsafe { (*o).at(OFF_INNER) };
    unsafe { (*o).outer = if outer.is_null() { inner } else { outer } };
    let hr = unsafe { i_query(inner, riid, out) };
    unsafe { i_release(inner) }; // the reference of creation: the answer holds its own (none on failure)
    hr
}

// ---------- the COM server ----------

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, riid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        let outer = outer.ok().map_or(std::ptr::null_mut(), |u| u.as_raw());
        let hr = unsafe { create(outer, riid, out) };
        if hr.is_err() {
            logm!("CreateInstance: aggregated {}, {:?} -> {hr:?}", !outer.is_null(), if riid.is_null() { GUID::zeroed() } else { unsafe { *riid } });
        }
        hr.ok()
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// # Safety
/// Called by COM with valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || clsid.is_null() || riid.is_null() {
        return E_POINTER;
    }
    unsafe {
        *out = std::ptr::null_mut();
        if *clsid != CLSID_ARCMIX_APO {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let f: IClassFactory = Factory.into();
        f.query(riid, out)
    }
}

/// The module stays loaded while the audio engine runs.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boost_boosts_and_flat_passes() {
        let mut st = St::new();
        st.ch = 1;
        st.rate = 48000.0;
        st.co[0] = peaking(48000.0, 1000.0, 6.0, 1.41).unwrap();
        st.n = 1;
        st.pre = 1.0;
        let mut buf: Vec<f32> = (0..48000).map(|i| (i as f32 * std::f32::consts::TAU * 1000.0 / 48000.0).sin() * 0.25).collect();
        st.process(&mut buf);
        let peak = buf[24000..].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((peak / 0.25 - 2.0).abs() < 0.05, "+6 dB at the centre: {peak}");
        assert!(peaking(48000.0, 1000.0, 0.0, 1.41).is_none());
    }

    #[test]
    fn bad_values_are_left_out() {
        assert!(peaking(48000.0, 1000.0, f64::NAN, 1.41).is_none());
        assert!(peaking(48000.0, f64::INFINITY, 6.0, 1.41).is_none());
        assert!(peaking(48000.0, 1000.0, 6.0, f64::NAN).is_none());
        assert!(peaking(48000.0, 1000.0, 90.0, 1.41).is_none());
    }

    #[test]
    fn files_round_trip() {
        let mut e = Entry::EMPTY;
        e.guid = shared::guid_field("{b9dd9f19-5dfb-4cba-afc6-8279613240c4}");
        e.on = 1;
        e.n = 2;
        e.gain_db[1] = -3.5;
        let b = shared::encode_curves(&[e, Entry::EMPTY]);
        let d = shared::decode_curves(&b).unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].on, d[0].n, d[0].gain_db[1]), (1, 2, -3.5));
        assert_eq!(shared::key_of(&d[0].guid), shared::key_of(&e.guid));
        assert!(shared::decode_curves(&b[..100]).is_none(), "cut short");
        let l = shared::encode_live(&[(7, 1000), (9, 2000)]);
        assert_eq!(shared::decode_live(&l), vec![(7, 1000), (9, 2000)]);
    }

    /// A stand-in for the engine's wrapper: it hands every interface but IUnknown to the inner object.
    #[repr(C)]
    struct Outer {
        vtbl: &'static IUnknown_Vtbl,
        refs: AtomicU32,
        inner: *mut c_void,
    }

    unsafe extern "system" fn o_query(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        let o = unsafe { &*(this as *const Outer) };
        if unsafe { *iid } == IUnknown::IID {
            unsafe { *out = this };
            o.refs.fetch_add(1, Ordering::AcqRel);
            return S_OK;
        }
        unsafe { (unknown_vtbl(o.inner).QueryInterface)(o.inner, iid, out) }
    }
    unsafe extern "system" fn o_add_ref(this: *mut c_void) -> u32 {
        unsafe { &*(this as *const Outer) }.refs.fetch_add(1, Ordering::AcqRel) + 1
    }
    unsafe extern "system" fn o_release(this: *mut c_void) -> u32 {
        unsafe { &*(this as *const Outer) }.refs.fetch_sub(1, Ordering::AcqRel) - 1
    }
    static OUTER_VTBL: IUnknown_Vtbl = IUnknown_Vtbl { QueryInterface: o_query, AddRef: o_add_ref, Release: o_release };

    #[test]
    fn works_alone() {
        let mut p = std::ptr::null_mut();
        assert_eq!(unsafe { create(std::ptr::null_mut(), &IAudioProcessingObject::IID, &mut p) }, S_OK);
        let apo = unsafe { IAudioProcessingObject::from_raw(p) };
        assert_eq!(unsafe { apo.GetLatency() }.unwrap(), 0);
        let rt: IAudioProcessingObjectRT = apo.cast().unwrap();
        assert_eq!(unsafe { rt.CalcInputFrames(480) }, 480);
        let _: IAudioSystemEffects = rt.cast().unwrap();
        let _: IAudioProcessingObjectConfiguration = apo.cast().unwrap();
        assert!(apo.cast::<windows::Win32::System::Com::IClassFactory>().is_err());
    }

    #[test]
    fn works_aggregated() {
        let outer = Box::into_raw(Box::new(Outer { vtbl: &OUTER_VTBL, refs: AtomicU32::new(1), inner: std::ptr::null_mut() }));
        let mut inner = std::ptr::null_mut();
        let op = outer as *mut c_void;
        assert_eq!(unsafe { create(op, &IAudioProcessingObject::IID, &mut inner) }, CLASS_E_NOAGGREGATION, "only IUnknown when aggregated");
        assert_eq!(unsafe { create(op, &IUnknown::IID, &mut inner) }, S_OK);
        unsafe { (*outer).inner = inner };
        {
            // the engine's view: everything through the wrapper, the wrapper counts
            let unk = unsafe { IUnknown::from_raw_borrowed(&op) }.unwrap();
            let apo: IAudioProcessingObject = unk.cast().unwrap();
            assert_eq!(unsafe { (*outer).refs.load(Ordering::Acquire) }, 2);
            let back: IUnknown = apo.cast().unwrap();
            assert_eq!(back.as_raw(), op, "IUnknown of an interface is the wrapper");
            let cfg: IAudioProcessingObjectConfiguration = apo.cast().unwrap();
            drop((back, cfg));
            assert_eq!(unsafe { apo.GetLatency() }.unwrap(), 0);
        }
        assert_eq!(unsafe { (*outer).refs.load(Ordering::Acquire) }, 1);
        let o = unsafe { Obj::of(inner, OFF_INNER) };
        assert_eq!(o.refs.load(Ordering::Acquire), 1, "the wrapper holds the inner once");
        assert_eq!(unsafe { i_release(inner) }, 0);
        drop(unsafe { Box::from_raw(outer) });
    }

    #[test]
    fn wrap_lines() {
        let t = "{B9DD9F19-5DFB-4CBA-AFC6-8279613240C4} {13AB3EBD-137E-4903-9D89-60BE8277FD17}\r\n{F43B33F8-1C84-4023-BE3C-2F9C34AF1E09} {11111111-1111-1111-1111-111111111111}\r\n";
        assert_eq!(shared::wrap_lookup(t, "{f43b33f8-1c84-4023-be3c-2f9c34af1e09}").as_deref(), Some("{11111111-1111-1111-1111-111111111111}"));
        assert_eq!(shared::wrap_lookup(t, "{00000000-0000-0000-0000-000000000000}"), None);
        assert!(GUID::try_from("13AB3EBD-137E-4903-9D89-60BE8277FD17").is_ok());
    }

    #[test]
    fn keys_ignore_case() {
        let a = shared::guid_field("{b9dd9f19-5dfb-4cba-afc6-8279613240c4}");
        let b = shared::guid_field("{B9DD9F19-5DFB-4CBA-AFC6-8279613240C4}");
        assert_eq!(shared::key_of(&a), shared::key_of(&b));
    }
}
