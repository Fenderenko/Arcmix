//! Sound processing for routing and the microphone. No Windows dependencies — covered by tests.
//!
//! - `StreamFifo`: a buffer between two devices with different clocks. Reading uses a variable
//!   resampling ratio (cubic interpolation) tuned so the fill stays near the target: this is both
//!   the 44.1↔48 kHz conversion and drift compensation (a cable and a headset never run exactly
//!   in sync).
//! - `EqChain`: a software 10-band equalizer (RBJ peaking biquads).
//! - `MicProc`: microphone processing — low cut, RNNoise denoiser, plugins, noise gate, limiter.

use std::{
    ffi::c_void,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicPtr, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use crate::rnnoise::Denoiser;

// ---------- sample formats ----------

/// Device sample format.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SampleFmt {
    pub channels: usize,
    pub rate: u32,
    /// 16, 24, 32
    pub bits: u32,
    /// IEEE float (32 bits)
    pub float: bool,
    /// bytes per frame
    pub block: usize,
}

impl SampleFmt {
    fn bytes_per_sample(&self) -> usize {
        let mut bps = (self.bits / 8) as usize;
        if self.block > 0 && self.channels > 0 && self.block / self.channels > bps {
            bps = self.block / self.channels; // e.g. 24 bits in a 32-bit container
        }
        bps
    }

    /// Device frames → f32 (channel interleaving kept). dst must hold frames*channels samples.
    pub fn to_float(&self, dst: &mut [f32], src: &[u8], frames: usize) {
        let n = frames * self.channels;
        let bps = self.bytes_per_sample();
        for (i, d) in dst[..n].iter_mut().enumerate() {
            let o = i * bps;
            if o + bps > src.len() {
                *d = 0.0;
                continue;
            }
            let b = &src[o..o + bps];
            *d = match bps {
                4 if self.float => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                2 => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
                3 => ((u32::from(b[0]) << 8 | u32::from(b[1]) << 16 | u32::from(b[2]) << 24) as i32 >> 8) as f32 / 8388608.0,
                4 => {
                    let v = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                    if self.bits == 24 { (v >> 8) as f32 / 8388608.0 } else { (v as f64 / 2147483648.0) as f32 }
                }
                _ => 0.0,
            };
        }
    }

    /// f32 → device format (clamped to ±1).
    pub fn from_float(&self, dst: &mut [u8], src: &[f32], frames: usize) {
        let n = (frames * self.channels).min(src.len());
        let bps = self.bytes_per_sample();
        for (i, &s) in src[..n].iter().enumerate() {
            let o = i * bps;
            if o + bps > dst.len() {
                return;
            }
            let v = s.clamp(-1.0, 1.0);
            let d = &mut dst[o..o + bps];
            match bps {
                4 if self.float => d.copy_from_slice(&v.to_le_bytes()),
                2 => d.copy_from_slice(&((v as f64 * 32767.0).round() as i16).to_le_bytes()),
                3 => d.copy_from_slice(&((v as f64 * 8388607.0).round() as i32).to_le_bytes()[..3]),
                4 => {
                    let s = if self.bits == 24 {
                        ((v as f64 * 8388607.0).round() as i32) << 8
                    } else {
                        (v as f64 * 2147483647.0).round().clamp(-2147483648.0, 2147483647.0) as i32
                    };
                    d.copy_from_slice(&s.to_le_bytes());
                }
                _ => {}
            }
        }
    }
}

/// in_ch → out_ch. Mono goes to all channels, multichannel to mono is the mean of the first two,
/// otherwise the first channels are taken (the rest is silence). dst must hold frames*out_ch.
pub fn map_channels(dst: &mut [f32], src: &[f32], frames: usize, in_ch: usize, out_ch: usize) {
    for fr in 0..frames {
        let inp = &src[fr * in_ch..fr * in_ch + in_ch];
        let out = &mut dst[fr * out_ch..fr * out_ch + out_ch];
        if in_ch == out_ch {
            out.copy_from_slice(inp);
        } else if in_ch == 1 {
            out.fill(inp[0]);
        } else if out_ch == 1 {
            out[0] = (inp[0] + inp[1]) * 0.5;
        } else {
            for (c, o) in out.iter_mut().enumerate() {
                *o = if c < in_ch { inp[c] } else { 0.0 };
            }
        }
    }
}

/// A scratch buffer that grows only when a bigger packet comes (no allocation per buffer).
pub fn scratch(v: &mut Vec<f32>, n: usize) -> &mut [f32] {
    if v.len() < n {
        v.resize(n, 0.0);
    }
    &mut v[..n]
}

// ---------- buffer between devices with rate tuning ----------

struct FifoState {
    ch: usize,
    /// interleaved frames; buf[0..] starts with one frame of history
    buf: Vec<f32>,
    /// fractional read position (frames from the start of buf)
    rd: f64,
    in_rate: f64,
    out_rate: f64,
    /// nominal in_rate/out_rate
    ratio: f64,
    /// drift correction (a fraction)
    adj: f64,
    integ: f64,
    /// smoothed fill, frames
    avg: f64,
    /// target fill, frames (at the input rate)
    target: f64,
    min_t: f64,
    max_t: f64,
    primed: bool,
    /// seconds without underruns (to bring the latency back to the minimum gradually)
    calm: f64,
}

pub struct StreamFifo {
    st: Mutex<FifoState>,
    pub underruns: AtomicI32,
    /// frames received in total (for "there is signal")
    pub last_data: AtomicI64,
}

impl FifoState {
    fn frames(&self) -> usize {
        self.buf.len() / self.ch
    }

    /// Drops frames already read, keeping one frame of history.
    fn compact(&mut self) {
        let drop = self.rd as i64 - 1;
        if drop < 2048 {
            return;
        }
        let d = drop as usize * self.ch;
        self.buf.copy_within(d.., 0);
        self.buf.truncate(self.buf.len() - d);
        self.rd -= drop as f64;
    }
}

#[inline]
pub fn catmull(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    p1 + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

impl StreamFifo {
    /// in_rate/out_rate — input and output rates; target_ms — the initial target latency.
    pub fn new(ch: usize, in_rate: u32, out_rate: u32, target_ms: f64) -> Self {
        let (inr, outr) = (in_rate as f64, out_rate as f64);
        let min_t = target_ms / 1000.0 * inr;
        let max_t = 60.0 / 1000.0 * inr;
        // capacity for the overflow limit plus compaction slack: pushes never reallocate
        let mut buf = Vec::with_capacity((max_t as usize * 4 + 2048 + 8192) * ch);
        buf.resize(ch, 0.0); // one frame of silence as interpolation history
        StreamFifo {
            st: Mutex::new(FifoState {
                ch,
                buf,
                rd: 1.0,
                in_rate: inr,
                out_rate: outr,
                ratio: inr / outr,
                adj: 0.0,
                integ: 0.0,
                avg: 0.0,
                target: min_t,
                min_t,
                max_t,
                primed: false,
                calm: 0.0,
            }),
            underruns: AtomicI32::new(0),
            last_data: AtomicI64::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FifoState> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Appends frames (already with the right channel count).
    pub fn push(&self, fr: &[f32]) {
        let mut g = self.lock();
        let f = &mut *g;
        f.buf.extend_from_slice(fr);
        self.last_data.fetch_add((fr.len() / f.ch) as i64, Ordering::Relaxed);
        // overflow (the output stopped or lags far behind): keep the target reserve
        let fill = f.frames() as f64 - f.rd;
        if fill > f.max_t * 4.0 {
            f.rd = f.frames() as f64 - f.target;
            f.avg = f.target;
            f.integ = 0.0;
        }
        f.compact();
    }

    /// Writes n frames into out (at least n*ch long). Returns the frames with real sound; the
    /// rest is silence (the input is quiet or still filling up).
    pub fn pull(&self, out: &mut [f32], n: usize) -> usize {
        let mut g = self.lock();
        let f = &mut *g;
        let ch = f.ch;
        let total = f.frames();
        let mut fill = total as f64 - f.rd;
        if !f.primed {
            if fill < f.target + 2.0 {
                out[..n * ch].fill(0.0);
                return 0;
            }
            f.primed = true;
            f.avg = fill;
            f.integ = 0.0;
        }
        let step = f.ratio * (1.0 + f.adj);
        let mut got = 0;
        for k in 0..n {
            let i = f.rd as usize;
            if i + 2 >= total {
                // the buffer ran out: the rest is silence; next time a (slightly bigger) reserve is built up again
                out[k * ch..n * ch].fill(0.0);
                f.primed = false;
                self.underruns.fetch_add(1, Ordering::Relaxed);
                f.target = f.max_t.min(f.target * 1.25 + 0.002 * f.in_rate);
                f.calm = 0.0;
                break;
            }
            let t = (f.rd - i as f64) as f32;
            let b = &f.buf;
            for c in 0..ch {
                out[k * ch + c] = catmull(b[(i - 1) * ch + c], b[i * ch + c], b[(i + 1) * ch + c], b[(i + 2) * ch + c], t);
            }
            f.rd += step;
            got += 1;
        }
        f.compact();
        // tuning: fill above the target — read a bit faster, below — slower
        fill = f.frames() as f64 - f.rd;
        let dt = n as f64 / f.out_rate;
        // after an underrun the reserve grows; 20 s without trouble bring the minimum latency back gradually
        f.calm += dt;
        if f.calm > 20.0 && f.target > f.min_t {
            f.target = f.min_t.max(f.target * 0.85);
            f.calm = 0.0;
        }
        f.avg += (fill - f.avg) * (dt / 0.5).min(1.0); // ~0.5 s smoothing
        let e = (f.avg - f.target) / f.target.max(1.0);
        f.integ += 0.004 * e * dt;
        f.integ = f.integ.clamp(-0.005, 0.005);
        f.adj = (0.002 * e + f.integ).clamp(-0.01, 0.01);
        got
    }

    /// The current buffer latency (ms) at the input rate.
    pub fn latency_ms(&self, in_rate: u32) -> f64 {
        self.lock().avg / in_rate as f64 * 1000.0
    }

    #[cfg(test)]
    fn adj_target(&self) -> (f64, f64) {
        let f = self.lock();
        (f.adj, f.target)
    }
}

// ---------- software equalizer ----------

#[derive(Clone, Copy, Debug, Default)]
pub struct BqCoef {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

/// RBJ peaking filter (Audio EQ Cookbook).
pub fn peaking(fs: f64, f0: f64, gain_db: f64, q: f64) -> BqCoef {
    if gain_db == 0.0 || f0 >= fs * 0.49 {
        return BqCoef { b0: 1.0, ..Default::default() };
    }
    let a = 10f64.powf(gain_db / 40.0);
    let w = 2.0 * std::f64::consts::PI * f0 / fs;
    let al = w.sin() / (2.0 * q);
    let cw = w.cos();
    let a0 = 1.0 + al / a;
    BqCoef { b0: (1.0 + al * a) / a0, b1: -2.0 * cw / a0, b2: (1.0 - al * a) / a0, a1: -2.0 * cw / a0, a2: (1.0 - al / a) / a0 }
}

/// RBJ second-order high-pass filter.
pub fn highpass(fs: f64, f0: f64, q: f64) -> BqCoef {
    let w = 2.0 * std::f64::consts::PI * f0 / fs;
    let al = w.sin() / (2.0 * q);
    let cw = w.cos();
    let a0 = 1.0 + al;
    BqCoef { b0: (1.0 + cw) / 2.0 / a0, b1: -(1.0 + cw) / a0, b2: (1.0 + cw) / 2.0 / a0, a1: -2.0 * cw / a0, a2: (1.0 - al) / a0 }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BqState {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

/// Filters applied one after another to every channel.
pub struct EqChain {
    f: Vec<BqCoef>,
    /// [filter * ch + channel]
    st: Vec<BqState>,
    ch: usize,
}

impl EqChain {
    pub fn new(fs: f64, freqs: &[f64], gains: &[f64], ch: usize) -> Self {
        let f: Vec<BqCoef> = freqs.iter().enumerate().filter(|&(i, _)| i < gains.len() && gains[i] != 0.0).map(|(i, &f0)| peaking(fs, f0, gains[i], 1.41)).collect();
        let st = vec![BqState::default(); f.len() * ch];
        EqChain { f, st, ch }
    }

    pub fn process(&mut self, buf: &mut [f32], ch: usize) {
        if self.f.is_empty() || ch != self.ch {
            return;
        }
        let frames = buf.len() / ch;
        for (fi, q) in self.f.iter().enumerate() {
            for c in 0..ch {
                let mut s = self.st[fi * ch + c];
                for k in 0..frames {
                    let x = buf[k * ch + c] as f64;
                    let y = q.b0 * x + q.b1 * s.x1 + q.b2 * s.x2 - q.a1 * s.y1 - q.a2 * s.y2;
                    (s.x2, s.x1, s.y2, s.y1) = (s.x1, x, s.y1, y);
                    buf[k * ch + c] = y as f32;
                }
                if s.y1.abs() < 1e-20 {
                    // no denormals in silence
                    (s.y1, s.y2) = (0.0, 0.0);
                }
                self.st[fi * ch + c] = s;
            }
        }
    }
}

// ---------- microphone processing ----------

/// External processing (VST plugins) between the low cut and the gate; processes interleaved
/// frames in place. Called on the audio thread only.
pub type ExtFn = dyn Fn(&mut [f32], usize) + Send + Sync;

/// Controls of the microphone processing shared with other threads: thresholds change from the
/// settings, the meter is read by the window, plugins are swapped by the UI thread.
pub struct MicCtl {
    /// f64 bits; ≤ −100 — the gate is off
    gate_db: AtomicU64,
    /// f64 bits; ≥ 0 — the limiter is off
    ceil_db: AtomicU64,
    denoise: AtomicBool,
    /// max level at the input since the last read (f64 bits)
    peak: AtomicU64,
    out_peak: AtomicU64,
    open: AtomicBool,
    /// Box<Box<ExtFn>> as a raw pointer; null — none
    ext: AtomicPtr<c_void>,
    /// 1 — the audio thread is inside ext right now
    busy: AtomicBool,
    /// the sample rate of the stream
    pub rate: u32,
}

impl MicCtl {
    pub fn set_gate(&self, db: f64) {
        self.gate_db.store(db.to_bits(), Ordering::Relaxed);
    }

    pub fn set_ceil(&self, db: f64) {
        self.ceil_db.store(db.to_bits(), Ordering::Relaxed);
    }

    pub fn set_denoise(&self, on: bool) {
        self.denoise.store(on, Ordering::Relaxed);
    }

    /// Peak level at the input and output (dBFS) since the last call, and the gate state.
    pub fn meter(&self) -> (f64, f64, bool) {
        let i = f64::from_bits(self.peak.swap(0, Ordering::Relaxed));
        let o = f64::from_bits(self.out_peak.swap(0, Ordering::Relaxed));
        (to_db(i), to_db(o), self.open.load(Ordering::Relaxed))
    }

    /// Installs a new plugin chain (None — none) and frees the old one once the audio thread has
    /// left it. If the audio thread does not leave it within max_wait, the old chain is leaked
    /// instead of being freed under it.
    pub fn set_ext(&self, e: Option<Box<ExtFn>>, max_wait: Duration) {
        let p = e.map_or(std::ptr::null_mut(), |b| Box::into_raw(Box::new(b)) as *mut c_void);
        let old = self.ext.swap(p, Ordering::SeqCst);
        if old.is_null() {
            return;
        }
        let t = Instant::now();
        while self.busy.load(Ordering::SeqCst) && t.elapsed() < max_wait {
            std::thread::sleep(Duration::from_millis(1));
        }
        if self.busy.load(Ordering::SeqCst) {
            crate::logf!("mic: plugin chain still busy, not freed");
            return;
        }
        drop(unsafe { Box::from_raw(old as *mut Box<ExtFn>) });
    }
}

impl Drop for MicCtl {
    fn drop(&mut self) {
        let p = *self.ext.get_mut();
        if !p.is_null() {
            drop(unsafe { Box::from_raw(p as *mut Box<ExtFn>) });
        }
    }
}

/// Raises the f64 kept in a (as bits) to v.
pub fn store_max(a: &AtomicU64, v: f64) {
    let mut old = a.load(Ordering::Relaxed);
    while f64::from_bits(old) < v {
        match a.compare_exchange_weak(old, v.to_bits(), Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(o) => old = o,
        }
    }
}

pub fn to_db(v: f64) -> f64 {
    if v < 1e-6 { -120.0 } else { 20.0 * v.log10() }
}

/// Microphone processing state (audio thread): low cut (80 Hz), denoiser (RNNoise), plugins,
/// noise gate and limiter.
pub struct MicProc {
    pub ctl: Arc<MicCtl>,
    fs: f64,
    ch: usize,
    hp: BqCoef,
    hp_st: Vec<BqState>,
    /// level detector (before the gate)
    env: f64,
    /// current gate gain 0..1
    gate: f64,
    hold: i64,
    lim_env: f64,
    dn: Option<Box<Denoiser>>,
}

impl MicProc {
    pub fn new(fs: f64, ch: usize) -> Self {
        let ctl = Arc::new(MicCtl {
            gate_db: AtomicU64::new((-100f64).to_bits()),
            ceil_db: AtomicU64::new(0f64.to_bits()),
            denoise: AtomicBool::new(false),
            peak: AtomicU64::new(0),
            out_peak: AtomicU64::new(0),
            open: AtomicBool::new(false),
            ext: AtomicPtr::new(std::ptr::null_mut()),
            busy: AtomicBool::new(false),
            rate: fs as u32,
        });
        MicProc { ctl, fs, ch, hp: highpass(fs, 80.0, 0.707), hp_st: vec![BqState::default(); ch], env: 0.0, gate: 1.0, hold: 0, lim_env: 0.0, dn: None }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        let ch = self.ch;
        let frames = buf.len() / ch;
        let ctl = &*self.ctl;
        let gate_db = f64::from_bits(ctl.gate_db.load(Ordering::Relaxed));
        let ceil_db = f64::from_bits(ctl.ceil_db.load(Ordering::Relaxed));
        let gate_on = gate_db > -100.0;
        let ceil = 10f64.powf(ceil_db.min(0.0) / 20.0);
        let lim_on = ceil_db < 0.0;
        let gate_lin = 10f64.powf(gate_db / 20.0);
        let close_lin = 10f64.powf((gate_db - 6.0) / 20.0); // 6 dB hysteresis
        let fs = self.fs;
        let env_att = 1.0 - (-1.0 / (0.001 * fs)).exp();
        let env_rel = 1.0 - (-1.0 / (0.120 * fs)).exp();
        let open_rate = 1.0 - (-1.0 / (0.002 * fs)).exp();
        let close_rate = 1.0 - (-1.0 / (0.090 * fs)).exp();
        let lim_rel = 1.0 - (-1.0 / (0.080 * fs)).exp();
        let hold_len = (0.25 * fs) as i64;
        let (mut peak_in, mut peak_out) = (0f64, 0f64);
        let mut target = if gate_on && !ctl.open.load(Ordering::Relaxed) { 0.0 } else { 1.0 };
        // 1) low cut
        let hp = self.hp;
        for k in 0..frames {
            for c in 0..ch {
                let s = &mut self.hp_st[c];
                let v = buf[k * ch + c] as f64;
                let y = hp.b0 * v + hp.b1 * s.x1 + hp.b2 * s.x2 - hp.a1 * s.y1 - hp.a2 * s.y2;
                (s.x2, s.x1, s.y2, s.y1) = (s.x1, v, s.y1, y);
                buf[k * ch + c] = y as f32;
            }
        }
        // 2) denoiser: created on first use, its state is dropped once switched off
        if ctl.denoise.load(Ordering::Relaxed) {
            self.dn.get_or_insert_with(|| Box::new(Denoiser::new(fs as u32))).process(buf, ch);
        } else {
            self.dn = None;
        }
        // 3) plugins (if any)
        ctl.busy.store(true, Ordering::SeqCst);
        let e = ctl.ext.load(Ordering::SeqCst);
        if !e.is_null() {
            let f = unsafe { &*(e as *const Box<ExtFn>) };
            f(buf, ch);
        }
        ctl.busy.store(false, Ordering::SeqCst);
        // 4) level detector, gate and limiter
        for k in 0..frames {
            let mut x = 0f64;
            for c in 0..ch {
                x = x.max((buf[k * ch + c] as f64).abs());
            }
            peak_in = peak_in.max(x);
            if x > self.env {
                self.env += (x - self.env) * env_att;
            } else {
                self.env += (x - self.env) * env_rel;
            }
            // noise gate: opens above the threshold, closes below (threshold − 6 dB) after the hold
            if gate_on {
                if self.env >= gate_lin {
                    target = 1.0;
                    self.hold = hold_len;
                    ctl.open.store(true, Ordering::Relaxed);
                } else if self.hold > 0 {
                    self.hold -= 1;
                } else if self.env < close_lin {
                    target = 0.0;
                    ctl.open.store(false, Ordering::Relaxed);
                }
            } else {
                target = 1.0;
                ctl.open.store(true, Ordering::Relaxed);
            }
            if target > self.gate {
                self.gate += (target - self.gate) * open_rate;
            } else {
                self.gate += (target - self.gate) * close_rate;
            }
            let mut g = self.gate;
            // limiter: instant attack, smooth release; a hard ceiling at the end
            if lim_on {
                let pk = x * g;
                if pk > self.lim_env {
                    self.lim_env = pk;
                } else {
                    self.lim_env += (pk - self.lim_env) * lim_rel;
                }
                if self.lim_env > ceil {
                    g *= ceil / self.lim_env;
                }
            }
            for c in 0..ch {
                let mut v = buf[k * ch + c] as f64 * g;
                if lim_on {
                    v = v.clamp(-ceil, ceil);
                }
                buf[k * ch + c] = v as f32;
                peak_out = peak_out.max(v.abs());
            }
        }
        for s in self.hp_st.iter_mut() {
            if s.y1.abs() < 1e-20 {
                (s.y1, s.y2) = (0.0, 0.0);
            }
        }
        store_max(&ctl.peak, peak_in);
        store_max(&ctl.out_peak, peak_out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::STD10;

    pub fn sine(freq: f64, rate: f64, frames: usize, ch: usize, amp: f64, phase: &mut f64) -> Vec<f32> {
        let mut out = vec![0f32; frames * ch];
        for i in 0..frames {
            let v = (amp * phase.sin()) as f32;
            *phase += 2.0 * std::f64::consts::PI * freq / rate;
            out[i * ch..i * ch + ch].fill(v);
        }
        out
    }

    fn peak_of(b: &[f32]) -> f64 {
        b.iter().fold(0f64, |m, &v| m.max((v as f64).abs()))
    }

    /// Signal frequency by zero crossings.
    fn zero_cross_freq(b: &[f32], ch: usize, rate: f64) -> f64 {
        let frames = b.len() / ch;
        let n = (1..frames).filter(|&i| (b[(i - 1) * ch] < 0.0) != (b[i * ch] < 0.0)).count();
        n as f64 / 2.0 / (frames as f64 / rate)
    }

    #[test]
    fn formats() {
        let src = [0.0f32, 0.5, -0.5, 0.999, -1.0];
        for f in [
            SampleFmt { channels: 1, rate: 48000, bits: 32, float: true, block: 4 },
            SampleFmt { channels: 1, rate: 48000, bits: 16, float: false, block: 2 },
            SampleFmt { channels: 1, rate: 48000, bits: 24, float: false, block: 3 },
            SampleFmt { channels: 1, rate: 48000, bits: 24, float: false, block: 4 },
            SampleFmt { channels: 1, rate: 48000, bits: 32, float: false, block: 4 },
        ] {
            let mut b = vec![0u8; src.len() * f.block];
            f.from_float(&mut b, &src, src.len());
            let mut back = vec![0f32; src.len()];
            f.to_float(&mut back, &b, src.len());
            for i in 0..src.len() {
                assert!((back[i] - src[i]).abs() <= 1e-4, "{f:?}: {} → {}", src[i], back[i]);
            }
        }
        let mut st = [0f32; 4];
        map_channels(&mut st, &[1.0, 2.0], 2, 1, 2);
        assert_eq!(st, [1.0, 1.0, 2.0, 2.0], "mono → stereo");
        let mut mo = [0f32; 1];
        map_channels(&mut mo, &[1.0, 3.0], 1, 2, 1);
        assert_eq!(mo[0], 2.0, "stereo → mono");
    }

    /// 44.1 kHz → 48 kHz: frequency and amplitude are kept.
    #[test]
    fn fifo_resample() {
        let f = StreamFifo::new(2, 44100, 48000, 10.0);
        let mut ph = 0.0;
        let mut out = Vec::new();
        for _ in 0..300 {
            f.push(&sine(1000.0, 44100.0, 441, 2, 0.5, &mut ph));
            let mut b = vec![0f32; 480 * 2];
            f.pull(&mut b, 480);
            out.extend_from_slice(&b);
        }
        let tail = &out[out.len() / 2..];
        let fr = zero_cross_freq(tail, 2, 48000.0);
        assert!((fr - 1000.0).abs() <= 3.0, "frequency after resampling {fr:.1} Hz");
        let p = peak_of(tail);
        assert!((p - 0.5).abs() <= 0.01, "amplitude {p:.3}");
        assert_eq!(f.underruns.load(Ordering::Relaxed), 0);
    }

    /// The cable and headset clocks drift apart (±300 ppm), packets come unevenly. Over 15 minutes
    /// the buffer neither overflows nor runs dry, the latency stays low.
    #[test]
    fn fifo_drift() {
        for ppm in [-300.0, 0.0, 300.0] {
            let f = StreamFifo::new(2, 48000, 48000, 10.0);
            let in_rate = 48000.0 * (1.0 + ppm / 1e6);
            let (mut ph, mut acc, mut now) = (0.0, 0.0, 0.0);
            const STEP: f64 = 0.010; // the output takes frames every 10 ms
            let (mut max_lat, mut settle) = (0f64, 0);
            let mut b = vec![0f32; 480 * 2];
            for i in 0..90000 {
                now += STEP;
                // input: 10 ms packets with ±3 ms jitter (as real drivers do)
                let jit = 0.003 * (i as f64 * 0.77).sin();
                while acc + 0.010 <= now + jit {
                    acc += 0.010;
                    let n = ((in_rate * acc).round() - (in_rate * (acc - 0.010)).round()) as usize;
                    f.push(&sine(440.0, in_rate, n, 2, 0.3, &mut ph));
                }
                f.pull(&mut b, 480);
                if i == 500 {
                    settle = f.underruns.load(Ordering::Relaxed);
                }
                if i > 500 {
                    max_lat = max_lat.max(f.latency_ms(48000));
                }
            }
            let u = f.underruns.load(Ordering::Relaxed) - settle;
            assert!(u <= 0, "{ppm:+.0} ppm: {u} underruns after settling");
            assert!(max_lat <= 45.0, "{ppm:+.0} ppm: buffer latency grew to {max_lat:.1} ms");
            let (adj, target) = f.adj_target();
            println!("{ppm:+.0} ppm: adj {adj:.5}, target {:.1} ms, max latency {max_lat:.1} ms, startup underruns {settle}", target / 48.0);
        }
    }

    /// The input is quiet (nothing plays into the cable) — silence at the output without glitches,
    /// then the sound comes back.
    #[test]
    fn fifo_silence_gap() {
        let f = StreamFifo::new(1, 48000, 48000, 10.0);
        let mut ph = 0.0;
        let mut b = vec![0f32; 480];
        for _ in 0..100 {
            f.push(&sine(500.0, 48000.0, 480, 1, 0.5, &mut ph));
            f.pull(&mut b, 480);
        }
        for i in 0..100 {
            f.pull(&mut b, 480);
            assert!(i <= 3 || peak_of(&b) == 0.0, "no silence during the gap");
        }
        let mut got = 0;
        for _ in 0..100 {
            f.push(&sine(500.0, 48000.0, 480, 1, 0.5, &mut ph));
            got += f.pull(&mut b, 480);
        }
        assert!(got >= 90 * 480, "sound did not come back after the gap: {got} frames");
    }

    #[test]
    fn peaking_eq() {
        let mut ph = 0.0;
        let mut inp = sine(1000.0, 48000.0, 48000, 1, 0.25, &mut ph);
        let mut e = EqChain::new(48000.0, &STD10, &[0.0, 0.0, 0.0, 0.0, 0.0, 6.0, 0.0, 0.0, 0.0, 0.0], 1);
        e.process(&mut inp, 1);
        let g = peak_of(&inp[24000..]) / 0.25;
        assert!((20.0 * g.log10() - 6.0).abs() <= 0.3, "gain at 1 kHz: {:.2} dB", 20.0 * g.log10());
        ph = 0.0;
        let mut inp = sine(100.0, 48000.0, 48000, 1, 0.25, &mut ph);
        let mut e = EqChain::new(48000.0, &STD10, &[0.0, 0.0, 0.0, 0.0, 0.0, 6.0, 0.0, 0.0, 0.0, 0.0], 1);
        e.process(&mut inp, 1);
        let g = peak_of(&inp[24000..]) / 0.25;
        assert!((20.0 * g.log10()).abs() <= 0.5, "the 1 kHz band touched 100 Hz: {:.2} dB", 20.0 * g.log10());
    }

    #[test]
    fn mic_gate_and_limiter() {
        const FS: usize = 48000;
        let mut m = MicProc::new(FS as f64, 1);
        m.ctl.set_gate(-40.0);
        m.ctl.set_ceil(-6.0);
        let mut ph = 0.0;
        // quiet noise (−60 dB) — the gate is closed, almost nothing at the output
        let mut quiet = sine(300.0, FS as f64, FS, 1, 0.001, &mut ph);
        m.process(&mut quiet);
        assert!(peak_of(&quiet[FS / 2..]) <= 0.0001, "quiet background passed the gate");
        // voice (−10 dB) — passes
        let mut voice = sine(300.0, FS as f64, FS, 1, 0.3, &mut ph);
        m.process(&mut voice);
        assert!(peak_of(&voice[FS / 4..]) >= 0.25, "voice attenuated");
        // a shout (0 dB) — limited to −6 dB
        let mut loud = sine(300.0, FS as f64, FS, 1, 1.0, &mut ph);
        m.process(&mut loud);
        let p = peak_of(&loud);
        assert!(p <= 0.502, "the limiter let {p:.3} through (ceiling 0.501)");
        let (i, o, open) = m.ctl.meter();
        assert!(i >= -1.0 && o <= -5.9 && open, "meter: in {i:.1}, out {o:.1}, open {open}");
        // gate and ceiling off — the signal is unchanged (except the cut below 80 Hz)
        let mut m2 = MicProc::new(FS as f64, 1);
        ph = 0.0;
        let mut s = sine(1000.0, FS as f64, FS, 1, 0.9, &mut ph);
        m2.process(&mut s);
        let p = peak_of(&s[FS / 2..]);
        assert!((p - 0.9).abs() <= 0.01, "amplitude changed without processing: {p:.3}");
    }

    /// External processing (a plugin) sits between the low cut and the gate.
    #[test]
    fn mic_ext() {
        use std::sync::atomic::AtomicUsize;
        let mut m = MicProc::new(48000.0, 1);
        let calls = Arc::new(AtomicUsize::new(0));
        let c2 = calls.clone();
        m.ctl.set_ext(
            Some(Box::new(move |buf: &mut [f32], _ch: usize| {
                c2.fetch_add(1, Ordering::Relaxed);
                for v in buf.iter_mut() {
                    *v *= 0.5;
                }
            })),
            Duration::from_millis(10),
        );
        let mut ph = 0.0;
        let mut s = sine(1000.0, 48000.0, 48000, 1, 0.8, &mut ph);
        m.process(&mut s);
        let p = peak_of(&s[24000..]);
        assert!((p - 0.4).abs() <= 0.01 && calls.load(Ordering::Relaxed) == 1, "plugin not applied: peak {p:.3}, calls {}", calls.load(Ordering::Relaxed));
        m.ctl.set_ext(None, Duration::from_millis(10));
    }
}
