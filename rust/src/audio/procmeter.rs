//! The level of a chat app — straight from its own sound (the way OBS captures one app).
//!
//! The Windows session meter is sometimes useless for Discord: during a call Discord keeps, besides
//! the voices, a capture of everything playing on the headset in the same session, and the meter
//! shows the level of all sound (music, video). Process loopback capture (Windows 10 2004+ /
//! Windows 11) gives only what the app itself plays — the voices. Without it, the session meter
//! stays, as before.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
            IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient, WAVEFORMATEX,
        },
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx, IAgileObject},
            Threading::{CreateEventW, WaitForSingleObject},
        },
    },
    core::{HRESULT, IUnknown, Interface, Ref, implement, w},
};

use crate::logf;

const STARTING: i32 = 0;
const RUNNING: i32 = 1;
const FAILED: i32 = 2;

pub(super) struct ProcMeter {
    stop: AtomicBool,
    state: AtomicI32,
    /// the highest level (f32 bits) since the last read: 0 — ducking, 1 — window
    peak: [AtomicU32; 2],
}

impl ProcMeter {
    /// The level, dB, since the last read (slot 0 — ducking, 1 — the "playing" mark).
    pub fn take(&self, slot: usize) -> f64 {
        let v = f32::from_bits(self.peak[slot].swap(0, Ordering::AcqRel));
        if v > 1e-6 { 20.0 * (v as f64).log10() } else { -120.0 }
    }

    fn put(&self, v: f32) {
        for p in &self.peak {
            let mut old = p.load(Ordering::Acquire);
            while f32::from_bits(old) < v {
                match p.compare_exchange_weak(old, v.to_bits(), Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) => break,
                    Err(x) => old = x,
                }
            }
        }
    }
}

/// Lives on the audio engine thread.
#[derive(Default)]
pub(super) struct ProcMeters {
    m: HashMap<u32, Arc<ProcMeter>>,
    want: HashMap<u32, String>,
    /// when to try again a process that failed
    retry: HashMap<u32, Instant>,
    /// Windows cannot capture a process's sound — no more tries
    off: bool,
    off_logged: bool,
}

/// captured a process's sound at least once
static LOOP_OK: AtomicBool = AtomicBool::new(false);
static LOOP_FAILS: AtomicI32 = AtomicI32::new(0);
/// pids already written to the log
static LOGGED: Mutex<Option<HashSet<u32>>> = Mutex::new(None);

impl ProcMeters {
    pub fn want_pid(&mut self, pid: u32, exe: &str) {
        if !self.want.contains_key(&pid) {
            self.want.insert(pid, exe.to_string());
        }
    }

    /// (end of apply) Start the needed ones, stop the rest; run=false — stop all.
    pub fn sync(&mut self, run: bool) {
        let now = Instant::now();
        let want = &self.want;
        let mut stopped = Vec::new();
        for (pid, m) in &self.m {
            let st = m.state.load(Ordering::Acquire);
            if run && want.contains_key(pid) && st != FAILED {
                continue;
            }
            if st == FAILED {
                self.retry.insert(*pid, now + Duration::from_secs(30));
                // never worked and failed twice already — this Windows cannot do it
                if !LOOP_OK.load(Ordering::Relaxed) && LOOP_FAILS.load(Ordering::Relaxed) >= 2 {
                    self.off = true;
                }
            }
            m.stop.store(true, Ordering::Release);
            stopped.push(*pid);
        }
        for pid in stopped {
            self.m.remove(&pid);
        }
        if run && !self.off {
            for (pid, exe) in &self.want {
                if self.m.contains_key(pid) || self.retry.get(pid).is_some_and(|t| now < *t) {
                    continue;
                }
                let m = Arc::new(ProcMeter { stop: AtomicBool::new(false), state: AtomicI32::new(STARTING), peak: [AtomicU32::new(0), AtomicU32::new(0)] });
                self.m.insert(*pid, m.clone());
                let (pid, exe) = (*pid, exe.clone());
                std::thread::Builder::new().name("procmeter".into()).spawn(move || run_meter(pid, exe, m)).ok();
            }
        }
        if self.off && !self.off_logged {
            self.off_logged = true;
            logf!("duck: this Windows can't capture an app's own sound — using session levels");
        }
        if self.retry.len() > 64 {
            self.retry.clear();
        }
        self.want.clear();
    }

    /// A running meter of the process, or None (then — the session meter).
    pub fn get(&self, pid: u32) -> Option<&Arc<ProcMeter>> {
        self.m.get(&pid).filter(|m| m.state.load(Ordering::Acquire) == RUNNING)
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct Done(Mutex<Option<mpsc::Sender<()>>>);

impl windows::Win32::System::Com::IAgileObject_Impl for Done_Impl {}

impl IActivateAudioInterfaceCompletionHandler_Impl for Done_Impl {
    fn ActivateCompleted(&self, _op: Ref<'_, IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        if let Some(tx) = self.0.lock().unwrap().take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

/// AUDIOCLIENT_ACTIVATION_PARAMS for process loopback, followed by a PROPVARIANT (VT_BLOB) pointing
/// at it. Activation is asynchronous, so the memory is kept until it completes (leaked on timeout:
/// Windows may still read it).
#[repr(C)]
struct ActParams {
    /// AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK
    kind: u32,
    pid: u32,
    /// PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
    mode: u32,
    _pad: u32,
    // PROPVARIANT: vt, reserved ×3, blob.cbSize, (padding), blob.pBlobData
    vt: u16,
    _r: [u16; 3],
    blob_size: u32,
    _p: u32,
    blob: *const u8,
}

/// IAudioClient that captures the sound of process pid (with its child processes).
fn activate(pid: u32) -> Result<IAudioClient, HRESULT> {
    let mut p = Box::new(ActParams { kind: 1, pid, mode: 0, _pad: 0, vt: 65, _r: [0; 3], blob_size: 12, _p: 0, blob: std::ptr::null() });
    p.blob = &p.kind as *const u32 as *const u8;
    let (tx, rx) = mpsc::channel();
    let handler: IActivateAudioInterfaceCompletionHandler = Done(Mutex::new(Some(tx))).into();
    let pv = &p.vt as *const u16 as *const _;
    let op = unsafe { ActivateAudioInterfaceAsync(w!(r"VAD\Process_Loopback"), &IAudioClient::IID, Some(pv), &handler) }.map_err(|e| e.code())?;
    if rx.recv_timeout(Duration::from_secs(5)).is_err() {
        std::mem::forget(p); // not waited for; Windows may still read the parameters
        return Err(HRESULT(-2));
    }
    let mut hr = HRESULT(0);
    let mut unk: Option<IUnknown> = None;
    unsafe { op.GetActivateResult(&mut hr, &mut unk) }.map_err(|e| e.code())?;
    drop(p);
    if hr.is_err() {
        return Err(hr);
    }
    unk.ok_or(HRESULT(-1))?.cast::<IAudioClient>().map_err(|e| e.code())
}

fn run_meter(pid: u32, exe: String, m: Arc<ProcMeter>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let fail = |what: &str, hr: HRESULT| {
        let n = LOOP_FAILS.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 3 || LOOP_OK.load(Ordering::Relaxed) {
            logf!("duck: can't capture {exe} sound directly ({what} 0x{:08x}) — using its session level", hr.0 as u32);
        }
        m.state.store(FAILED, Ordering::Release);
    };
    let client = match activate(pid) {
        Ok(c) => c,
        Err(hr) => return fail("activate", hr),
    };
    // 16 bit, 2 channels, 44.1 kHz — the format Windows suggests for process capture
    let wf = WAVEFORMATEX { wFormatTag: 1, nChannels: 2, nSamplesPerSec: 44100, nAvgBytesPerSec: 44100 * 4, nBlockAlign: 4, wBitsPerSample: 16, cbSize: 0 };
    unsafe {
        if let Err(e) = client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 200_000, 0, &wf, None) {
            return fail("Initialize", e.code());
        }
        let Ok(ev) = CreateEventW(None, false, false, None) else { return fail("CreateEvent", HRESULT(-1)) };
        let r = (|| -> Result<(), (&'static str, HRESULT)> {
            client.SetEventHandle(ev).map_err(|e| ("SetEventHandle", e.code()))?;
            let cc: IAudioCaptureClient = client.GetService().map_err(|e| ("GetService", e.code()))?;
            client.Start().map_err(|e| ("Start", e.code()))?;
            m.state.store(RUNNING, Ordering::Release);
            LOOP_OK.store(true, Ordering::Relaxed);
            if LOGGED.lock().unwrap().get_or_insert_with(HashSet::new).insert(pid) {
                logf!("duck: listening to {exe} sound directly");
            }
            // the capture loop allocates nothing
            while !m.stop.load(Ordering::Acquire) {
                let _ = WaitForSingleObject(ev, 200) == WAIT_OBJECT_0;
                loop {
                    let next = cc.GetNextPacketSize().map_err(|e| ("capture", e.code()))?;
                    if next == 0 {
                        break;
                    }
                    let (mut data, mut frames, mut flags) = (std::ptr::null_mut::<u8>(), 0u32, 0u32);
                    cc.GetBuffer(&mut data, &mut frames, &mut flags, None, None).map_err(|e| ("capture", e.code()))?;
                    if frames > 0 && !data.is_null() && flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 == 0 {
                        let s = std::slice::from_raw_parts(data as *const i16, frames as usize * 2);
                        let pk = s.iter().map(|&v| (v as i32).unsigned_abs()).max().unwrap_or(0);
                        m.put(pk as f32 / 32768.0);
                    }
                    cc.ReleaseBuffer(frames).map_err(|e| ("capture", e.code()))?;
                }
            }
            let _ = client.Stop();
            Ok(())
        })();
        let _ = CloseHandle(ev);
        if let Err((what, hr)) = r {
            fail(what, hr);
        }
    }
}
