//! Sound routing (experimental).
//!
//! "route": apps of the "Game", "Chat" and "Media" categories play into virtual cables; the app reads the
//! cables as a loopback, applies the category's software equalizer if needed, mixes and outputs
//! to the headphones. ChatMix keeps working through app volumes (on any device).
//!
//! "mic": microphone → low cut, RNNoise denoiser, plugins, noise gate, limiter → a virtual cable;
//! in Discord or a game the other end of that cable is chosen as the microphone.

mod monitor;
pub mod wasapi;

use std::{
    collections::HashMap,
    sync::{
        Arc, Condvar, LazyLock, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use serde::Serialize;
use windows::Win32::Media::Audio::eRender;

use crate::{
    audio::{self, com, outprio::is_virtual_name},
    config::{self, Config},
    devices::STD10,
    dsp::{EqChain, MicCtl, MicProc, SampleFmt, StreamFifo, map_channels, scratch},
    logf, util,
};

pub use monitor::{MonState, mic_peak_db, mic_peak_stop, monitor_set, monitor_status, monitor_wanted};

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct PipeStatus {
    pub on: bool,
    pub running: bool,
    pub latency: i32,
    /// microphone processing sleeps: no app records from its cable (micFx.sleep)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub asleep: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub err: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hint: String,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// State of a "listen to yourself" input without a device of its own: the sound comes from the
/// microphone processing thread.
#[derive(Default)]
struct TapState {
    /// the output format
    of: SampleFmt,
    ch: usize,
    fifo: Option<Arc<StreamFifo>>,
    mapped: Vec<f32>,
}

#[derive(Default)]
struct PipeIn {
    id: String,
    loopback: bool,
    tap: bool,
    /// set by the input thread; fifo_gen grows on every change (the output thread re-reads it then)
    fifo: Mutex<Option<Arc<StreamFifo>>>,
    fifo_gen: AtomicU32,
    /// the software equalizer (dB per std10 band), None — without it
    eq_want: Mutex<Option<Arc<Vec<f64>>>>,
    eq_gen: AtomicU32,
    mp: Mutex<Option<Arc<MicCtl>>>,
    in_rate: AtomicU32,
    tap_st: Mutex<TapState>,
}

impl PipeIn {
    fn new(id: &str, loopback: bool, tap: bool) -> Arc<Self> {
        Arc::new(PipeIn { id: id.to_string(), loopback, tap, ..Default::default() })
    }

    fn set_fifo(&self, f: Arc<StreamFifo>) {
        *lock(&self.fifo) = Some(f);
        self.fifo_gen.fetch_add(1, Ordering::Release);
    }

    /// (microphone processing thread) The processed sound for "listen to yourself".
    fn feed(&self, buf: &[f32], frames: usize, ch: usize, rate: u32) {
        let mut g = lock(&self.tap_st);
        let ts = &mut *g;
        let of = ts.of;
        if of.channels == 0 {
            return;
        }
        if ts.fifo.is_none() || self.in_rate.load(Ordering::Relaxed) != rate || ts.ch != ch {
            let f = Arc::new(StreamFifo::new(of.channels, rate, of.rate, 10.0));
            self.in_rate.store(rate, Ordering::Relaxed);
            ts.ch = ch;
            ts.fifo = Some(f.clone());
            self.set_fifo(f);
        }
        let m = scratch(&mut ts.mapped, frames * of.channels);
        map_channels(m, buf, frames, ch, of.channels);
        if let Some(f) = &ts.fifo {
            f.push(m);
        }
    }
}

struct Pipeline {
    /// "route" | "mic" | "monitor"
    kind: &'static str,
    sig: String,
    out_id: String,
    ins: Vec<Arc<PipeIn>>,

    /// for the microphone (f64 bits)
    gate: AtomicU64,
    ceil: AtomicU64,
    denoise: AtomicBool,
    quality: AtomicBool,
    /// compressor threshold, ratio (≤ 1 — off), gain; the order of the stages (dsp::order_code)
    comp: Mutex<(f64, f64, f64)>,
    order: AtomicU32,
    /// routing: the software equalizer of the whole mix ("all"), after the categories' own
    eq_want: Mutex<Option<Arc<Vec<f64>>>>,
    eq_gen: AtomicU32,

    stop: Mutex<bool>,
    stop_cv: Condvar,
    stopping: AtomicBool,
    running: AtomicBool,
    err: Mutex<String>,
    lat_ms: AtomicU64,
    started: Instant,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl Pipeline {
    fn new(kind: &'static str, sig: String, out_id: String, ins: Vec<Arc<PipeIn>>) -> Self {
        Pipeline {
            kind,
            sig,
            out_id,
            ins,
            gate: AtomicU64::new(0),
            ceil: AtomicU64::new(0),
            denoise: AtomicBool::new(false),
            quality: AtomicBool::new(false),
            comp: Mutex::new((0.0, 1.0, 0.0)),
            order: AtomicU32::new(0),
            eq_want: Mutex::new(None),
            eq_gen: AtomicU32::new(0),
            stop: Mutex::new(false),
            stop_cv: Condvar::new(),
            stopping: AtomicBool::new(false),
            running: AtomicBool::new(false),
            err: Mutex::new(String::new()),
            lat_ms: AtomicU64::new(0),
            started: Instant::now(),
            handle: Mutex::new(None),
        }
    }

    fn halt(&self) {
        self.stopping.store(true, Ordering::Release);
        *lock(&self.stop) = true;
        self.stop_cv.notify_all();
    }

    fn stopped(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    /// Sleeps up to d, returns early when the pipeline is halted.
    fn wait_stop(&self, d: Duration) {
        let g = lock(&self.stop);
        if !*g {
            let _ = self.stop_cv.wait_timeout(g, d);
        }
    }

    fn fail(&self, msg: &str) {
        {
            let mut e = lock(&self.err);
            if e.is_empty() {
                *e = msg.to_string();
                if pipe_log_fail(self.kind, msg) {
                    logf!("{}: {msg}", self.kind);
                }
            }
        }
        self.halt();
    }

    fn finished(&self) -> bool {
        lock(&self.handle).as_ref().is_none_or(|h| h.is_finished())
    }

    /// Waits for the threads to end (up to max; None — without a limit).
    fn join(&self, max: Option<Duration>) {
        let t = Instant::now();
        while !self.finished() {
            if max.is_some_and(|m| t.elapsed() >= m) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some(h) = lock(&self.handle).take() {
            let _ = h.join();
        }
    }
}

fn soft_clip(b: &mut [f32]) {
    for v in b.iter_mut() {
        if *v > 0.95 {
            *v = 0.95 + 0.05 * ((*v as f64 - 0.95) / 0.05).tanh() as f32;
        } else if *v < -0.95 {
            *v = -0.95 - 0.05 * ((-*v as f64 - 0.95) / 0.05).tanh() as f32;
        }
    }
}

fn eq_active(w: &[f64]) -> bool {
    w.iter().any(|&g| g != 0.0)
}

fn run_output(p: Arc<Pipeline>) {
    let _at = wasapi::audio_thread();
    if util::safely_value(p.kind, || output_main(&p)).is_none() {
        p.fail("internal error");
        panicked(&p);
    }
    p.running.store(false, Ordering::Relaxed);
}

/// Panics of the microphone pipeline within PANIC_WINDOW; at PANIC_LIMIT the plugins go off
/// (vst::safe_mode), so the pipeline restarts without them instead of failing again and again.
static MIC_PANICS: Mutex<Vec<Instant>> = Mutex::new(Vec::new());
const PANIC_LIMIT: usize = 3;
const PANIC_WINDOW: Duration = Duration::from_secs(600);

fn panicked(p: &Pipeline) {
    if p.kind != "mic" {
        return;
    }
    let n = {
        let mut v = lock(&MIC_PANICS);
        v.retain(|t| t.elapsed() < PANIC_WINDOW);
        v.push(Instant::now());
        v.len()
    };
    if n >= PANIC_LIMIT {
        lock(&MIC_PANICS).clear();
        crate::vst::safe_mode("pipeline failures");
    }
}

fn output_main(p: &Arc<Pipeline>) {
    let mut en = com::Enum::default();
    let Some(enm) = en.get().cloned() else {
        p.fail("no audio device enumerator");
        return;
    };
    let mut out_id = p.out_id.clone();
    if out_id.is_empty() {
        out_id = com::default_id(&enm, eRender);
    }
    if p.ins.iter().any(|i| i.loopback && i.id == out_id) {
        p.fail("output device is also an input (feedback loop)");
        return;
    }
    let out = match wasapi::open(&enm, &out_id, true, false) {
        Ok(c) => c,
        Err(e) => {
            p.fail(&format!("output: {e}"));
            return;
        }
    };
    let of = out.fmt;
    std::thread::scope(|s| {
        for inp in &p.ins {
            let (p, inp) = (p.clone(), inp.clone());
            s.spawn(move || {
                let _at = wasapi::audio_thread();
                if util::safely_value(p.kind, || run_input(&p, &inp, of)).is_none() {
                    p.fail("internal error");
                    panicked(&p);
                }
            });
        }
        if util::safely_value(p.kind, || render_loop(p, &out)).is_none() {
            p.fail("internal error");
            panicked(p);
        }
        p.halt();
    });
}

fn render_loop(p: &Pipeline, out: &wasapi::Client) {
    let kind = p.kind;
    let of = out.fmt;
    // Minimal latency: the output buffer holds one Windows engine period and a small margin (not
    // the whole buffer). If the thread falls behind, the margin grows by itself.
    let mut margin = of.rate * 3 / 1000;
    let mut target_pad = out.buf_frames.min(out.period + margin);
    if !pipe_log_quiet(kind) {
        logf!("{kind}: output {} ch, {} Hz, {} bit, buffer {} frames, period {}, keep {target_pad}", of.channels, of.rate, of.bits, out.buf_frames, out.period);
    }
    if let Err(e) = out.start(target_pad) {
        p.fail(&e);
        return;
    }
    p.running.store(true, Ordering::Relaxed);
    if kind == "route" {
        // apps may be sent into the cables now
        ROUTE_LIVE.store(true, Ordering::Relaxed);
        audio::reapply();
    }
    struct Live(bool);
    impl Drop for Live {
        fn drop(&mut self) {
            if self.0 {
                ROUTE_LIVE.store(false, Ordering::Relaxed);
                audio::reapply(); // apps back to the usual output
            }
        }
    }
    let _live = Live(kind == "route");

    let n = p.ins.len();
    let started = Instant::now();
    let mut ok_logged = false;
    let (mut tmp, mut mix) = (Vec::new(), Vec::new());
    let mut fifos: Vec<Option<Arc<StreamFifo>>> = vec![None; n];
    let mut fifo_gens = vec![0u32; n];
    let mut eqs: Vec<Option<EqChain>> = (0..n).map(|_| None).collect();
    let mut eq_gens = vec![0u32; n];
    let (mut master, mut master_gen) = (None::<EqChain>, 0u32);
    let mut last_lat = Instant::now();
    while !p.stopped() {
        if !ok_logged && started.elapsed() > Duration::from_secs(3) {
            pipe_log_ok(kind); // runs steadily — the next error gets logged again
            ok_logged = true;
        }
        out.wait(100);
        let pad = match out.padding() {
            Ok(v) => v,
            Err(e) => {
                p.fail(&wasapi::hr_err("output", &e));
                return;
            }
        };
        // the first seconds do not count: threads are still "warming up", the margin would grow for nothing
        if pad < of.rate / 1000 && target_pad < out.buf_frames && started.elapsed() > Duration::from_secs(3) {
            // the sound nearly ran out — 2 ms more margin
            margin += of.rate * 2 / 1000;
            target_pad = out.buf_frames.min(out.period + margin);
            logf!("{kind}: output nearly ran dry, keeping {target_pad} frames now");
        }
        if pad >= target_pad {
            continue;
        }
        for (i, inp) in p.ins.iter().enumerate() {
            let g = inp.fifo_gen.load(Ordering::Acquire);
            if g != fifo_gens[i] {
                fifo_gens[i] = g;
                fifos[i] = lock(&inp.fifo).clone();
            }
            let g = inp.eq_gen.load(Ordering::Acquire);
            if g != eq_gens[i] {
                eq_gens[i] = g;
                let w = lock(&inp.eq_want).clone();
                eqs[i] = w.filter(|w| eq_active(w)).map(|w| EqChain::new(of.rate as f64, &STD10, &w, of.channels));
            }
        }
        let g = p.eq_gen.load(Ordering::Acquire);
        if g != master_gen {
            master_gen = g;
            let w = lock(&p.eq_want).clone();
            master = w.filter(|w| eq_active(w)).map(|w| EqChain::new(of.rate as f64, &STD10, &w, of.channels));
        }
        let r = out.render_frames(target_pad - pad, &mut tmp, |buf, n| {
            buf.fill(0.0);
            let m = scratch(&mut mix, buf.len());
            if kind == "monitor" {
                // "listen to yourself"
                if fifos[0].as_ref().is_none_or(|f| f.pull(m, n) == 0) {
                    m.fill(0.0);
                }
                monitor::mix(buf, m, &of);
                return;
            }
            for (i, f) in fifos.iter().enumerate() {
                let Some(f) = f else { continue };
                if f.pull(m, n) == 0 {
                    continue;
                }
                if let Some(e) = &mut eqs[i] {
                    e.process(m, of.channels);
                }
                for (b, v) in buf.iter_mut().zip(m.iter()) {
                    *b += *v;
                }
            }
            if let Some(e) = &mut master {
                e.process(buf, of.channels);
            }
            soft_clip(buf);
        });
        if let Err(e) = r {
            p.fail(&wasapi::hr_err("output", &e));
            return;
        }
        if last_lat.elapsed() > Duration::from_millis(500) {
            last_lat = Instant::now();
            let lat = pad as f64 / of.rate as f64 * 1000.0;
            let mut worst = 0f64;
            for (i, inp) in p.ins.iter().enumerate() {
                if let Some(f) = &fifos[i] {
                    worst = worst.max(f.latency_ms(inp.in_rate.load(Ordering::Relaxed).max(1)));
                }
            }
            p.lat_ms.store((lat + worst + 10.0).to_bits(), Ordering::Relaxed); // + a capture packet ~10 ms
        }
    }
}

fn run_input(p: &Pipeline, inp: &Arc<PipeIn>, of: SampleFmt) {
    if inp.tap {
        // the sound comes from the microphone processing thread
        lock(&inp.tap_st).of = of;
        monitor::set_tap(Some(inp.clone()));
        while !p.stopped() {
            p.wait_stop(Duration::from_secs(1));
        }
        monitor::clear_tap(inp);
        return;
    }
    let mut en = com::Enum::default();
    let Some(enm) = en.get().cloned() else {
        p.fail("input: no audio device enumerator");
        return;
    };
    let mut id = inp.id.clone();
    if id.is_empty() && !inp.loopback {
        id = com::auto_mic_id(&enm);
    }
    let c = match wasapi::open(&enm, &id, false, inp.loopback) {
        Ok(c) => c,
        Err(e) => {
            p.fail(&format!("input: {e}"));
            return;
        }
    };
    if p.kind == "mic" && is_virtual_name(&com::friendly_name(&c.dev)) {
        // the cable we write into ourselves — an echo
        p.fail("mic input is a virtual cable");
        return;
    }
    let cf = c.fmt;
    logf!("{}: input {} ch, {} Hz, {} bit (loopback {})", p.kind, cf.channels, cf.rate, cf.bits, inp.loopback);
    let f = Arc::new(StreamFifo::new(of.channels, cf.rate, of.rate, 10.0));
    inp.in_rate.store(cf.rate, Ordering::Relaxed);
    let mut mp = None;
    if p.kind == "mic" {
        let m = MicProc::new(cf.rate as f64, cf.channels);
        m.ctl.set_gate(f64::from_bits(p.gate.load(Ordering::Relaxed)));
        m.ctl.set_ceil(f64::from_bits(p.ceil.load(Ordering::Relaxed)));
        m.ctl.set_denoise(p.denoise.load(Ordering::Relaxed));
        m.ctl.set_quality(p.quality.load(Ordering::Relaxed));
        let (cd, cr, cg) = *lock(&p.comp);
        m.ctl.set_comp(cd, cr, cg);
        m.ctl.set_order(p.order.load(Ordering::Relaxed));
        *lock(&inp.mp) = Some(m.ctl.clone());
        let ctl = m.ctl.clone();
        util::run_on_ui(move || crate::vst::attach(&ctl)); // plugins at this rate
        mp = Some(m);
    }
    inp.set_fifo(f.clone());
    if let Err(e) = c.start(0) {
        p.fail(&e);
        return;
    }
    let (mut tmp, mut mapped) = (Vec::new(), Vec::new());
    let mut tap = monitor::TapCache::default();
    let evented = c.evented();
    while !p.stopped() {
        // a microphone wakes the thread when a packet is ready (~10 ms); loopback is polled
        if evented {
            c.wait(50);
        } else {
            std::thread::sleep(Duration::from_millis(4));
        }
        let r = c.capture_all(&mut tmp, |buf, frames| {
            if let Some(m) = &mut mp {
                m.process(buf);
                if let Some(t) = tap.get() {
                    // "listen to yourself" — what the others hear
                    t.feed(buf, frames, cf.channels, cf.rate);
                }
            }
            let mo = scratch(&mut mapped, frames * of.channels);
            map_channels(mo, buf, frames, cf.channels, of.channels);
            f.push(mo);
        });
        if let Err(e) = r {
            p.fail(&wasapi::hr_err("input", &e));
            return;
        }
    }
}

// ---------- control ----------

#[derive(Default)]
struct Mgr {
    route: Option<Arc<Pipeline>>,
    mic: Option<Arc<Pipeline>>,
    /// "listen to yourself"
    mon: Option<Arc<Pipeline>>,
    retry_at: HashMap<&'static str, Instant>,
    /// failed starts in a row: the pause between attempts grows up to a minute
    tries: HashMap<&'static str, u32>,
}

static MGR: LazyLock<Mutex<Mgr>> = LazyLock::new(Default::default);
/// Routing is running — apps can be sent into the cables.
static ROUTE_LIVE: AtomicBool = AtomicBool::new(false);
/// Microphone processing sleeps (micFx.sleep): no app records from its cable.
static MIC_ASLEEP: AtomicBool = AtomicBool::new(false);
/// The Microphone tab shows the processing levels (server meters "fx"): it runs meanwhile.
static FX_WATCH: AtomicU32 = AtomicU32::new(0);
/// A sync before the next 2 s: someone started or stopped recording from the cable.
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// Wakes the router thread for a sync now.
pub fn wake() {
    *lock(&WAKE.0) = true;
    WAKE.1.notify_one();
}

fn wait_wake(d: Duration) {
    let g = lock(&WAKE.0);
    let (mut g, _) = WAKE.1.wait_timeout_while(g, d, |w| !*w).unwrap_or_else(|p| p.into_inner());
    *g = false;
}

/// (server) While it lives, the window shows the levels of microphone processing: it does not sleep.
pub struct FxWatch;

impl FxWatch {
    pub fn new() -> Self {
        FX_WATCH.fetch_add(1, Ordering::Relaxed);
        wake();
        FxWatch
    }
}

impl Drop for FxWatch {
    fn drop(&mut self) {
        FX_WATCH.fetch_sub(1, Ordering::Relaxed);
        wake();
    }
}

/// The same error repeating on every attempt is logged once.
#[derive(Default)]
struct PipeLog {
    last: HashMap<&'static str, String>,
    n: HashMap<&'static str, u32>,
}

static PIPE_LOG: LazyLock<Mutex<PipeLog>> = LazyLock::new(Default::default);

fn pipe_log_fail(kind: &'static str, msg: &str) -> bool {
    let mut l = lock(&PIPE_LOG);
    *l.n.entry(kind).or_default() += 1;
    if l.last.get(kind).is_some_and(|m| m == msg) {
        return false;
    }
    l.last.insert(kind, msg.to_string());
    true
}

fn pipe_log_ok(kind: &'static str) {
    let mut l = lock(&PIPE_LOG);
    if let Some(n) = l.n.get(kind).copied().filter(|&n| n > 0) {
        logf!("{kind}: running again after {n} failed attempts");
    }
    l.last.remove(kind);
    l.n.remove(kind);
}

fn pipe_log_quiet(kind: &str) -> bool {
    lock(&PIPE_LOG).last.get(kind).is_some_and(|m| !m.is_empty())
}

pub fn route_live() -> bool {
    ROUTE_LIVE.load(Ordering::Relaxed)
}

/// Brings pipelines in line with the settings every 2 s (after a device error a pipeline is
/// restarted with a pause).
pub fn start() {
    std::thread::Builder::new()
        .name("router".into())
        .spawn(|| {
            loop {
                wait_wake(Duration::from_secs(2));
                util::safely("router", || sync(&config::get()));
            }
        })
        .ok();
}

fn soft_bands(c: &Config, cat: &str) -> Option<Arc<Vec<f64>>> {
    c.soft_eq.get(cat).filter(|s| s.bands.len() == 10 && s.preset != "flat").map(|s| Arc::new(s.bands.clone()))
}

fn same_bands(a: &Option<Arc<Vec<f64>>>, b: &Option<Arc<Vec<f64>>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a == b,
        (None, None) => true,
        _ => false,
    }
}

/// Brings the running pipelines in line with the settings.
pub fn sync(c: &Config) {
    if util::main_hwnd().is_invalid() {
        return; // the app has not started yet (or tests)
    }
    crate::apo::sync(c);
    crate::apomod::sync(c);
    let mut g = lock(&MGR);
    let r = &mut *g;

    // ----- routing -----
    let rc = &c.route;
    let cables = ["game", "chat", "media"].map(|cat| (c.route_cable(cat), cat));
    let want_route = c.route_live() && cables.iter().any(|(id, _)| !id.is_empty()) && c.on("route");
    let sig = format!("{}|{}|{}|{}", cables[0].0, cables[1].0, cables[2].0, rc.out);
    let cur = r.route.take();
    r.route = r.keep(cur, "route", want_route, &sig, || {
        let mut ins: Vec<Arc<PipeIn>> = Vec::new();
        for (id, _) in cables {
            if !id.is_empty() && !ins.iter().any(|x| x.id == id) {
                ins.push(PipeIn::new(id, true, false));
            }
        }
        Pipeline::new("route", sig.clone(), rc.out.clone(), ins)
    });
    if let Some(p) = &r.route {
        // one cable per category; a cable picked twice keeps the first category's equalizer
        let mut eq_for: HashMap<&str, Option<Arc<Vec<f64>>>> = HashMap::new();
        for (id, cat) in cables {
            if !id.is_empty() && !eq_for.contains_key(id) {
                eq_for.insert(id, soft_bands(c, cat));
            }
        }
        for inp in &p.ins {
            let w = eq_for.get(inp.id.as_str()).cloned().flatten();
            let mut cur = lock(&inp.eq_want);
            if !same_bands(&cur, &w) {
                *cur = w;
                inp.eq_gen.fetch_add(1, Ordering::Release);
            }
        }
        // the equalizer of everything that plays through the cables — unless Equalizer APO applies it
        // on the device they play to (it would be heard twice)
        let o = crate::audio::out_state();
        let (dest, name) = if rc.out.is_empty() || rc.out == o.id {
            (o.id.clone(), o.name.clone())
        } else {
            (rc.out.clone(), crate::audio::list_devices().0.into_iter().find(|d| d.id == rc.out).map(|d| d.name).unwrap_or_default())
        };
        let w = if crate::apo::applies_on(c, &dest, &name) || crate::apomod::applies_on(c, &dest, &name) { None } else { soft_bands(c, "all") };
        let mut cur = lock(&p.eq_want);
        if !same_bands(&cur, &w) {
            *cur = w;
            p.eq_gen.fetch_add(1, Ordering::Release);
        }
    }
    ROUTE_LIVE.store(r.route.as_ref().is_some_and(|p| p.running.load(Ordering::Relaxed)), Ordering::Relaxed);

    // ----- microphone -----
    let mc = &c.mic_fx;
    // microphone processing runs while either the denoiser or VST plugins are on (in "Features");
    // a category whose cable is this one is not routed (Config::route_cable)
    let want_mic = c.mic_fx_live() && !mc.out.is_empty() && (c.on("denoise") || c.on("vst"));
    // while it runs, the cable is Windows' default microphone (unless the app keeps off Windows' sound)
    crate::audio::sync_fx_mic(want_mic && c.win_ctl, &mc.out);
    let denoise = mc.denoise && c.on("denoise");
    let quality = want_mic && denoise && mc.denoise_kind == "nv" && crate::nvafx::available();
    crate::nvafx::set(&mc.nv_mode, mc.nv_strength);
    let comp = if mc.comp { (mc.comp_db, mc.comp_ratio, mc.comp_gain) } else { (0.0, 1.0, 0.0) };
    let order = crate::dsp::order_code(&mc.order);
    if quality {
        crate::nvafx::preload(); // the effect waits for the stream, loaded off the audio thread
    } else {
        crate::nvafx::release();
    }
    // asleep (micFx.sleep): no app records from the cable, and neither the Microphone tab's levels nor
    // "listen to yourself" need the processed sound; the plugins and the NVIDIA effect stay loaded
    let sleepy = want_mic && mc.sleep;
    crate::audio::watch_cable(if sleepy { &mc.out } else { "" });
    let asleep = sleepy && crate::audio::cable_idle() && FX_WATCH.load(Ordering::Relaxed) == 0 && !(monitor_wanted() && c.on("monitor"));
    if MIC_ASLEEP.swap(asleep, Ordering::Relaxed) != asleep {
        crate::server::notify_subs();
    }
    // the same microphone whose volume and mute the app controls
    let msig = format!("{}|{}", c.mic_dev, mc.out);
    let cur = r.mic.take();
    r.mic = r.keep(cur, "mic", want_mic && !asleep, &msig, || {
        let p = Pipeline::new("mic", msig.clone(), mc.out.clone(), vec![PipeIn::new(&c.mic_dev, false, false)]);
        // before the threads start: otherwise the first seconds the gate would be at 0 dB
        p.gate.store(mc.gate.to_bits(), Ordering::Relaxed);
        p.ceil.store(mc.ceil.to_bits(), Ordering::Relaxed);
        p.denoise.store(denoise, Ordering::Relaxed);
        p.quality.store(quality, Ordering::Relaxed);
        *lock(&p.comp) = comp;
        p.order.store(order, Ordering::Relaxed);
        p
    });
    if let Some(p) = &r.mic {
        p.gate.store(mc.gate.to_bits(), Ordering::Relaxed);
        p.ceil.store(mc.ceil.to_bits(), Ordering::Relaxed);
        p.denoise.store(denoise, Ordering::Relaxed);
        p.quality.store(quality, Ordering::Relaxed);
        *lock(&p.comp) = comp;
        p.order.store(order, Ordering::Relaxed);
        if let Some(ctl) = lock(&p.ins[0].mp).as_ref() {
            ctl.set_gate(mc.gate);
            ctl.set_ceil(mc.ceil);
            ctl.set_denoise(denoise);
            ctl.set_quality(quality);
            ctl.set_comp(comp.0, comp.1, comp.2);
            ctl.set_order(order);
        }
    }

    // ----- "listen to yourself" -----
    let tap = r.mic.as_ref().is_some_and(|p| p.running.load(Ordering::Relaxed));
    let src = if tap { "tap".to_string() } else { format!("dev:{}", c.mic_dev) };
    let cur = r.mon.take();
    r.mon = r.keep(cur, "monitor", monitor_wanted() && c.on("monitor"), &src, || Pipeline::new("monitor", src.clone(), String::new(), vec![PipeIn::new(&c.mic_dev, false, tap)]));
}

impl Mgr {
    /// Keeps, stops or (re)starts a pipeline.
    fn keep(&mut self, cur: Option<Arc<Pipeline>>, kind: &'static str, want: bool, sig: &str, mk: impl FnOnce() -> Pipeline) -> Option<Arc<Pipeline>> {
        let mut cur = cur;
        if let Some(p) = &cur
            && (!want || p.sig != sig)
        {
            p.halt();
            p.join(None);
            logf!("{kind}: stopped");
            cur = None;
        }
        if !want {
            self.retry_at.remove(kind);
            self.tries.remove(kind);
            return None;
        }
        if let Some(p) = &cur
            && p.running.load(Ordering::Relaxed)
            && p.started.elapsed() > Duration::from_secs(10)
        {
            self.tries.insert(kind, 0);
        }
        if let Some(p) = &cur
            && p.finished()
        {
            // stopped on an error — a restart after a pause
            match self.retry_at.get(kind).copied() {
                None => {
                    let t = self.tries.entry(kind).or_default();
                    *t += 1;
                    let pause = Duration::from_secs(5 * (*t).min(12) as u64);
                    self.retry_at.insert(kind, Instant::now() + pause);
                    return cur;
                }
                Some(t) if Instant::now() < t => return cur,
                Some(_) => {
                    self.retry_at.remove(kind);
                    p.join(None);
                    cur = None;
                }
            }
        }
        if cur.is_none() {
            let p = Arc::new(mk());
            if !pipe_log_quiet(kind) {
                logf!("{kind}: starting");
            }
            let p2 = p.clone();
            match std::thread::Builder::new().name(kind.into()).spawn(move || run_output(p2)) {
                Ok(h) => *lock(&p.handle) = Some(h),
                Err(e) => p.fail(&format!("thread: {e}")),
            }
            cur = Some(p);
        }
        cur
    }
}

pub fn mic_running() -> bool {
    lock(&MGR).mic.as_ref().is_some_and(|p| p.running.load(Ordering::Relaxed))
}

pub fn status(c: &Config) -> (PipeStatus, PipeStatus) {
    let g = lock(&MGR);
    let st = |p: &Option<Arc<Pipeline>>, on: bool| {
        let mut s = PipeStatus { on, ..Default::default() };
        if let Some(p) = p {
            s.running = p.running.load(Ordering::Relaxed);
            s.latency = f64::from_bits(p.lat_ms.load(Ordering::Relaxed)).round() as i32;
            s.err = lock(&p.err).clone();
        }
        s
    };
    let (mut route, mut mic) = (st(&g.route, c.route_live()), st(&g.mic, c.mic_fx_live()));
    mic.asleep = mic.on && MIC_ASLEEP.load(Ordering::Relaxed);
    route.hint = wasapi::err_hint(&route.err).into();
    mic.hint = wasapi::err_hint(&mic.err).into();
    (route, mic)
}

/// Microphone levels for the meter in the settings: in, out (dBFS), gate open, running.
pub fn mic_meter() -> (f64, f64, bool, bool) {
    let ctl = {
        let g = lock(&MGR);
        g.mic.as_ref().and_then(|p| lock(&p.ins[0].mp).clone())
    };
    match ctl {
        Some(c) => {
            let (i, o, open) = c.meter();
            (i, o, open, true)
        }
        None => (-120.0, -120.0, false, false),
    }
}

pub fn shutdown() {
    let mut g = lock(&MGR);
    for p in [g.route.take(), g.mic.take(), g.mon.take()].into_iter().flatten() {
        p.halt();
        p.join(Some(Duration::from_millis(1500)));
    }
    ROUTE_LIVE.store(false, Ordering::Relaxed);
}
