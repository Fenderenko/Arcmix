//! NVIDIA Audio Effects SDK denoiser: the "quality" noise suppression of the microphone on RTX
//! graphics cards, next to RNNoise. The SDK is not part of Arcmix: it is NVIDIA's own install
//! (NVAFX_SDK_DIR, by default Program Files\NVIDIA Corporation\NVIDIA Audio Effects SDK) with
//! NVAudioEffects.dll, its CUDA / TensorRT libraries and models\denoiser_48k.trtpkg.
//! 48 kHz, frames of 10 ms; the effect loads in the background (TensorRT, a few seconds), RNNoise
//! works meanwhile. A frame runs on the GPU synchronously, in the capture thread of the microphone,
//! which the output thread reads through a FIFO with its own margin.

use std::{
    ffi::{CString, c_char, c_void},
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

use serde::Serialize;
use windows::{
    Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LoadLibraryExW},
    core::{HSTRING, PCSTR},
};

use crate::{logf, rnnoise::RateConv, server};

const DLL: &str = "NVAudioEffects.dll";
const MODEL: &str = "models\\denoiser_48k.trtpkg";
const RATE: u32 = 48000;

/// The SDK folder: NVAFX_SDK_DIR (set by NVIDIA's installer) or the default install path.
pub fn sdk_dir() -> Option<PathBuf> {
    let env = std::env::var_os("NVAFX_SDK_DIR").map(PathBuf::from);
    let def = std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join("NVIDIA Corporation").join("NVIDIA Audio Effects SDK"));
    [env, def].into_iter().flatten().find(|d| d.join(DLL).is_file() && d.join(MODEL).is_file())
}

/// The SDK with the 48 kHz denoiser model is installed.
pub fn available() -> bool {
    sdk_dir().is_some()
}

// ---------- the library ----------

type Status = i32; // NvAFX_Status: 0 — success
type Handle = *mut c_void;

struct Api {
    create: unsafe extern "C" fn(*const c_char, *mut Handle) -> Status,
    destroy: unsafe extern "C" fn(Handle) -> Status,
    set_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> Status,
    set_u32: unsafe extern "C" fn(Handle, *const c_char, u32) -> Status,
    set_float: unsafe extern "C" fn(Handle, *const c_char, f32) -> Status,
    get_u32: unsafe extern "C" fn(Handle, *const c_char, *mut u32) -> Status,
    load: unsafe extern "C" fn(Handle) -> Status,
    run: unsafe extern "C" fn(Handle, *const *const f32, *const *mut f32, u32, u32) -> Status,
}

/// NVAudioEffects.dll, loaded once and kept for the life of the process.
fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| {
        let dir = sdk_dir()?;
        // its CUDA and TensorRT libraries lie next to it
        let lib = unsafe { LoadLibraryExW(&HSTRING::from(dir.join(DLL).as_os_str()), None, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) };
        let lib = match lib {
            Ok(l) => l,
            Err(e) => {
                logf!("nvafx: {DLL} not loaded: {e}");
                return None;
            }
        };
        macro_rules! sym {
            ($name:literal) => {{
                let p = unsafe { GetProcAddress(lib, PCSTR(concat!($name, "\0").as_ptr())) };
                match p {
                    Some(f) => unsafe { std::mem::transmute(f) },
                    None => {
                        logf!("nvafx: {} missing", $name);
                        return None;
                    }
                }
            }};
        }
        Some(Api {
            create: sym!("NvAFX_CreateEffect"),
            destroy: sym!("NvAFX_DestroyEffect"),
            set_string: sym!("NvAFX_SetString"),
            set_u32: sym!("NvAFX_SetU32"),
            set_float: sym!("NvAFX_SetFloat"),
            get_u32: sym!("NvAFX_GetU32"),
            load: sym!("NvAFX_Load"),
            run: sym!("NvAFX_Run"),
        })
    })
    .as_ref()
}

fn cs(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

/// A loaded denoiser effect, handed from the loading thread to the capture thread once.
pub struct Model {
    h: Handle,
    api: &'static Api,
    /// samples of one frame at 48 kHz
    frame: usize,
}

// SAFETY: the SDK handle has no thread affinity (the effect uses the primary CUDA context of the
// default GPU); it is used by one thread at a time: built on the loading thread, then moved whole.
unsafe impl Send for Model {}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe { (self.api.destroy)(self.h) };
    }
}

impl Model {
    /// Creates and loads the denoiser (blocks for seconds: TensorRT builds its engine).
    fn load() -> Result<Model, String> {
        let api = api().ok_or("NVIDIA Audio Effects SDK not found")?;
        let dir = sdk_dir().ok_or("NVIDIA Audio Effects SDK not found")?;
        let mut h: Handle = std::ptr::null_mut();
        let r = unsafe { (api.create)(cs("denoiser").as_ptr(), &mut h) };
        if r != 0 || h.is_null() {
            return Err(format!("NvAFX_CreateEffect: {r}"));
        }
        // destroyed on any error below
        let m = Model { h, api, frame: (RATE / 100) as usize };
        let path = cs(&dir.join(MODEL).to_string_lossy());
        let r = unsafe { (api.set_string)(h, cs("model_path").as_ptr(), path.as_ptr()) };
        if r != 0 {
            return Err(format!("NvAFX_SetString(model_path): {r}"));
        }
        // optional on some SDK versions: the defaults are the same
        unsafe {
            (api.set_u32)(h, cs("input_sample_rate").as_ptr(), RATE);
            (api.set_u32)(h, cs("num_streams").as_ptr(), 1);
            (api.set_float)(h, cs("intensity_ratio").as_ptr(), 1.0);
        }
        let r = unsafe { (api.load)(h) };
        if r != 0 {
            return Err(format!("NvAFX_Load: {r}"));
        }
        let get = |name: &str| {
            let mut v = 0u32;
            (unsafe { (api.get_u32)(h, cs(name).as_ptr(), &mut v) } == 0).then_some(v)
        };
        let rate = get("input_sample_rate").unwrap_or(RATE);
        let frame = get("num_input_samples_per_frame").unwrap_or(RATE / 100) as usize;
        if rate != RATE || frame == 0 || frame > 4800 {
            return Err(format!("unexpected format: {rate} Hz, frame {frame}"));
        }
        let mut m = m;
        m.frame = frame;
        Ok(m)
    }

    /// One frame, mono, at 48 kHz.
    fn run(&mut self, inp: &[f32], out: &mut [f32]) -> bool {
        let (i, o) = ([inp.as_ptr()], [out.as_mut_ptr()]);
        unsafe { (self.api.run)(self.h, i.as_ptr(), o.as_ptr(), self.frame as u32, 1) == 0 }
    }
}

// ---------- loading, off the audio thread ----------

/// 0 — not tried, 1 — loading, 2 — loaded once, 3 — failed (RNNoise stays)
static LOAD: AtomicU8 = AtomicU8::new(0);
static LOADING: AtomicBool = AtomicBool::new(false);
/// A loaded effect waiting for the microphone stream to take it.
static SLOT: Mutex<Option<Model>> = Mutex::new(None);

/// For the window: "missing" (no SDK) | "loading" | "ready" | "failed".
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct NvStatus {
    pub state: &'static str,
}

pub fn status() -> NvStatus {
    let state = match LOAD.load(Ordering::Relaxed) {
        _ if !available() => "missing",
        1 => "loading",
        3 => "failed",
        _ => "ready",
    };
    NvStatus { state }
}

/// Loads the effect in the background unless one waits already or loading failed before.
pub fn preload() {
    if !available() || LOAD.load(Ordering::Relaxed) == 3 || SLOT.lock().map(|s| s.is_some()).unwrap_or(true) || LOADING.swap(true, Ordering::AcqRel) {
        return;
    }
    LOAD.store(1, Ordering::Relaxed);
    server::notify_subs();
    std::thread::Builder::new()
        .name("nvafx load".into())
        .spawn(|| {
            let t = std::time::Instant::now();
            match Model::load() {
                Ok(m) => {
                    logf!("nvafx: denoiser loaded in {:.1} s", t.elapsed().as_secs_f64());
                    if let Ok(mut s) = SLOT.lock() {
                        *s = Some(m);
                    }
                    LOAD.store(2, Ordering::Relaxed);
                }
                Err(e) => {
                    logf!("nvafx: denoiser not loaded: {e}");
                    LOAD.store(3, Ordering::Relaxed);
                }
            }
            LOADING.store(false, Ordering::Release);
            server::notify_subs();
        })
        .ok();
}

/// Frees a loaded effect nobody took (the GPU memory), e.g. once the denoiser is switched off.
pub fn release() {
    if let Ok(mut s) = SLOT.try_lock() {
        s.take();
    }
}

/// (capture thread) The loaded effect, if one waits; otherwise its loading starts and the caller
/// uses RNNoise meanwhile. Never blocks.
pub fn take() -> Option<Model> {
    let m = SLOT.try_lock().ok().and_then(|mut s| s.take());
    if m.is_none() {
        preload();
    }
    m
}

// ---------- the denoiser on a stream ----------

/// The NVIDIA denoiser for any rate and channel count (channels are mixed to mono), like
/// rnnoise::Denoiser: frames of 10 ms at 48 kHz, a queue of one frame ahead.
pub struct NvDenoiser {
    m: Model,
    up: Option<RateConv>,
    down: Option<RateConv>,
    /// 48 kHz, waiting for a full frame
    acc: Vec<f32>,
    /// ready samples at the stream rate
    q: Vec<f32>,
    q_head: usize,
    inp: Vec<f32>,
    out: Vec<f32>,
}

impl NvDenoiser {
    pub fn new(m: Model, fs: u32) -> Self {
        let hop = m.frame;
        let mut pre = hop;
        let (mut up, mut down) = (None, None);
        if fs != RATE {
            up = Some(RateConv::new(fs, RATE));
            down = Some(RateConv::new(RATE, fs));
            pre = (hop * fs as usize).div_ceil(RATE as usize) + 8;
        }
        let mut q = Vec::with_capacity(pre + 4 * hop);
        q.resize(pre, 0.0);
        NvDenoiser { m, up, down, acc: Vec::with_capacity(2 * hop), q, q_head: 0, inp: vec![0.0; hop], out: vec![0.0; hop] }
    }

    fn run_frame(&mut self) {
        let hop = self.m.frame;
        for (d, s) in self.inp.iter_mut().zip(self.acc.drain(..hop)) {
            *d = s;
        }
        if !self.m.run(&self.inp, &mut self.out) {
            self.out.copy_from_slice(&self.inp); // a failed frame passes unchanged
        }
        match &mut self.down {
            None => self.q.extend_from_slice(&self.out),
            Some(d) => {
                for &v in &self.out {
                    d.push(v, &mut self.q);
                }
            }
        }
    }

    /// Processes interleaved frames in place, values within ±1.
    pub fn process(&mut self, buf: &mut [f32], ch: usize) {
        let frames = buf.len() / ch;
        for k in 0..frames {
            let s = buf[k * ch..k * ch + ch].iter().sum::<f32>() / ch as f32;
            match &mut self.up {
                None => self.acc.push(s),
                Some(u) => u.push(s, &mut self.acc),
            }
            while self.acc.len() >= self.m.frame {
                self.run_frame();
            }
            let mut v = 0f32;
            if self.q_head < self.q.len() {
                v = self.q[self.q_head];
                self.q_head += 1;
            }
            buf[k * ch..k * ch + ch].fill(v);
        }
        if self.q_head > 0 {
            self.q.drain(..self.q_head);
            self.q_head = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// The real SDK: load time, speed of a frame, noise taken out of a tone:
    /// cargo test --release -- --ignored nv_live --nocapture
    #[test]
    #[ignore]
    fn nv_live() {
        let Some(dir) = sdk_dir() else {
            println!("no NVIDIA Audio Effects SDK");
            return;
        };
        println!("SDK: {}", dir.display());
        let t0 = Instant::now();
        let m = Model::load().unwrap();
        println!("load {:.1} s, frame {}", t0.elapsed().as_secs_f64(), m.frame);
        let mut d = NvDenoiser::new(m, 48000);
        let (mut seed, n, hop) = (1u32, 600usize, 480usize);
        let (mut sum, mut max) = (0f64, 0f64);
        let (mut e_in, mut e_out) = (0f64, 0f64);
        for f in 0..n {
            // white noise only: a denoiser should take most of it out
            let mut buf: Vec<f32> = (0..hop)
                .map(|_| {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.2
                })
                .collect();
            let ein: f64 = buf.iter().map(|v| (*v as f64).powi(2)).sum();
            let t = Instant::now();
            d.process(&mut buf, 1);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            assert!(buf.iter().all(|v| v.is_finite()));
            if f >= 100 {
                sum += ms;
                max = max.max(ms);
                e_in += ein;
                e_out += buf.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            }
        }
        let avg = sum / (n - 100) as f64;
        println!("per 10 ms frame: avg {avg:.2} ms, max {max:.2} ms; noise {:.1} dB", 10.0 * (e_out / e_in).max(1e-12).log10());
        assert!(e_out < e_in * 0.5, "the noise is suppressed");
    }
}
