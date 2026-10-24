//! "Listen to yourself": the microphone in the headphones live, or a 5 s recording and playback.
//! While microphone processing runs, what the others hear is heard (after the denoiser, plugins,
//! gate and limiter); otherwise the microphone as is.
//! Also the microphone level meter of the main screen.

use std::{
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde::Serialize;

use super::{PipeIn, lock, mic_running, wasapi};
use crate::{
    audio::com,
    config,
    dsp::{SampleFmt, store_max, to_db},
    server, util,
};

const OFF: i32 = 0;
const LIVE: i32 = 1;
const RECORD: i32 = 2;
const PLAY: i32 = 3;
/// there is a recording, nothing plays
const READY: i32 = 4;
const REC_SEC: usize = 5;

static MODE: AtomicI32 = AtomicI32::new(OFF);

#[derive(Default)]
struct Rec {
    rec: Vec<f32>,
    len: usize,
    max: usize,
    ch: usize,
    play_pos: usize,
}

static REC: LazyLock<Mutex<Rec>> = LazyLock::new(Default::default);

/// The monitor input fed by the microphone processing thread; TAP_GEN grows on every change.
static TAP: Mutex<Option<Arc<PipeIn>>> = Mutex::new(None);
static TAP_GEN: AtomicU32 = AtomicU32::new(0);

pub(super) fn set_tap(t: Option<Arc<PipeIn>>) {
    *lock(&TAP) = t;
    TAP_GEN.fetch_add(1, Ordering::Release);
}

pub(super) fn clear_tap(t: &Arc<PipeIn>) {
    let mut g = lock(&TAP);
    if g.as_ref().is_some_and(|x| Arc::ptr_eq(x, t)) {
        *g = None;
        TAP_GEN.fetch_add(1, Ordering::Release);
    }
}

/// (microphone processing thread) The current tap, re-read only when it changes.
#[derive(Default)]
pub(super) struct TapCache {
    gen_: u32,
    t: Option<Arc<PipeIn>>,
}

impl TapCache {
    pub(super) fn get(&mut self) -> Option<&Arc<PipeIn>> {
        let g = TAP_GEN.load(Ordering::Acquire);
        if g != self.gen_ {
            self.gen_ = g;
            self.t = lock(&TAP).clone();
        }
        self.t.as_ref()
    }
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct MonState {
    /// "", live, record, play, ready
    pub mode: String,
    pub progress: f64,
    pub has: bool,
    /// the processed sound is heard
    pub tap: bool,
}

pub fn monitor_status() -> MonState {
    let tap = mic_running();
    let r = lock(&REC);
    let mode = MODE.load(Ordering::Relaxed);
    let mut s = MonState { tap, mode: ["", "live", "record", "play", "ready"][mode as usize].to_string(), has: r.len > 0 && mode != RECORD, progress: 0.0 };
    match mode {
        RECORD if r.max > 0 => s.progress = r.len as f64 / r.max as f64,
        PLAY if r.len > 0 => s.progress = r.play_pos as f64 / r.len as f64,
        _ => {}
    }
    s
}

fn resync() {
    server::notify_subs();
    std::thread::spawn(|| util::safely("router", || super::sync(&config::get())));
}

/// live | stop | record | play
pub fn monitor_set(op: &str) {
    {
        let mut r = lock(&REC);
        match op {
            "live" => MODE.store(LIVE, Ordering::Relaxed),
            "record" => {
                (r.len, r.max, r.play_pos) = (0, 0, 0);
                MODE.store(RECORD, Ordering::Relaxed);
            }
            "play" => {
                if r.len > 0 {
                    r.play_pos = 0;
                    MODE.store(PLAY, Ordering::Relaxed);
                }
            }
            _ => MODE.store(if r.len > 0 { READY } else { OFF }, Ordering::Relaxed),
        }
    }
    resync();
}

pub fn monitor_wanted() -> bool {
    matches!(MODE.load(Ordering::Relaxed), LIVE | RECORD | PLAY)
}

/// (output thread) inp — the microphone sound in the output format, buf — what plays in the
/// headphones.
pub(super) fn mix(buf: &mut [f32], inp: &[f32], of: &SampleFmt) {
    match MODE.load(Ordering::Relaxed) {
        LIVE => buf.copy_from_slice(&inp[..buf.len()]),
        RECORD => {
            let done = {
                let mut g = lock(&REC);
                let r = &mut *g;
                if r.max == 0 {
                    r.ch = of.channels;
                    r.max = REC_SEC * of.rate as usize * of.channels;
                    r.rec.resize(r.max, 0.0);
                }
                let k = inp.len().min(r.max - r.len);
                r.rec[r.len..r.len + k].copy_from_slice(&inp[..k]);
                r.len += k;
                let done = r.len >= r.max;
                if done {
                    MODE.store(READY, Ordering::Relaxed);
                }
                done
            };
            buf.fill(0.0);
            if done {
                resync();
            }
        }
        PLAY => {
            let (k, done) = {
                let mut g = lock(&REC);
                let r = &mut *g;
                let mut k = 0;
                if r.ch == of.channels {
                    // the same device — played as recorded
                    k = buf.len().min(r.len - r.play_pos);
                    buf[..k].copy_from_slice(&r.rec[r.play_pos..r.play_pos + k]);
                    r.play_pos += k;
                } else {
                    r.play_pos = r.len;
                }
                let done = r.play_pos >= r.len;
                if done {
                    MODE.store(READY, Ordering::Relaxed);
                }
                (k, done)
            };
            buf[k..].fill(0.0);
            if done {
                resync();
            }
        }
        _ => buf.fill(0.0),
    }
}

// ---------- microphone level meter (main screen) ----------

struct Peak {
    until: Option<Instant>,
    running: bool,
}

static PEAK: Mutex<Peak> = Mutex::new(Peak { until: None, running: false });
/// f64 bits
static PEAK_VAL: AtomicU64 = AtomicU64::new(0);
static PEAK_ERR: AtomicBool = AtomicBool::new(false);

/// Peak since the last call (dBFS). While the window asks, a light microphone capture is kept.
pub fn mic_peak_db() -> (f64, bool) {
    if crate::fly::HIDDEN.load(Ordering::Relaxed) {
        // a late request after the window was hidden does not open the microphone again
        return (-120.0, true);
    }
    let start = {
        let mut p = lock(&PEAK);
        p.until = Some(Instant::now() + Duration::from_millis(1500));
        let start = !p.running;
        p.running = true;
        start
    };
    if start {
        std::thread::Builder::new().name("mic meter".into()).spawn(|| {
            util::safely("mic meter", peak_loop);
            lock(&PEAK).running = false;
        }).ok();
    }
    (to_db(f64::from_bits(PEAK_VAL.swap(0, Ordering::Relaxed))), !PEAK_ERR.load(Ordering::Relaxed))
}

/// The window was hidden — the meter capture ends right away, not in 1.5 s (while it runs,
/// Windows shows "the microphone is in use" in the tray).
pub fn mic_peak_stop() {
    lock(&PEAK).until = None;
}

fn alive() -> bool {
    lock(&PEAK).until.is_some_and(|t| Instant::now() < t)
}

fn peak_loop() {
    let _at = wasapi::audio_thread();
    let mut en = com::Enum::default();
    let Some(enm) = en.get().cloned() else { return };
    let dev = config::with(|c| c.mic_dev.clone());
    let id = if dev.is_empty() { com::auto_mic_id(&enm) } else { dev.clone() };
    let c = match wasapi::open(&enm, &id, false, false) {
        Ok(c) => c,
        Err(_) => {
            PEAK_ERR.store(true, Ordering::Relaxed);
            while alive() {
                // no new attempts while the window stays open
                std::thread::sleep(Duration::from_millis(200));
            }
            PEAK_ERR.store(false, Ordering::Relaxed);
            return;
        }
    };
    if c.start(0).is_err() {
        PEAK_ERR.store(true, Ordering::Relaxed);
        return;
    }
    PEAK_ERR.store(false, Ordering::Relaxed);
    let mut tmp = Vec::new();
    while alive() && config::with(|c| c.mic_dev == dev) {
        c.wait(50); // the microphone wakes the thread when a packet is ready
        let mut pk = 0f64;
        let r = c.capture_all(&mut tmp, |buf, _| {
            for &v in buf.iter() {
                pk = pk.max((v as f64).abs());
            }
        });
        if r.is_err() {
            return;
        }
        store_max(&PEAK_VAL, pk);
    }
}
