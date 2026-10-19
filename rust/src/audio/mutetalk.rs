//! "Your mic is muted": while the app's microphone is muted, its level meter is polled, and speech
//! brings the on-screen indicator (at most once per COOLDOWN). Windows' mute leaves the endpoint
//! meter working (checked live on the Arctis 7+); a mute switch on the headset itself gives silence
//! and is not caught. The meter moves only while some app records the microphone, which is exactly
//! when the reminder matters.

use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use windows::Win32::{
    Media::Audio::Endpoints::IAudioMeterInformation,
    System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoInitializeEx},
};

use super::com;
use crate::{i18n, osd, util};

const POLL: Duration = Duration::from_millis(200);
/// Speech peaks seen muted were 0.2–0.4, the noise floor about 0.003.
const LEVEL: f32 = 0.05;
/// Loud polls out of the last WINDOW that count as talking (~1.6 s).
const WINDOW: usize = 8;
const HITS: usize = 3;
const COOLDOWN: Duration = Duration::from_secs(15);

/// The muted microphone to watch; "" — none.
static WATCH: Mutex<String> = Mutex::new(String::new());
static RUNNING: AtomicBool = AtomicBool::new(false);

/// (audio thread) The muted microphone to watch, or "" to stop; cheap to call on every refresh.
pub fn set(dev_id: &str) {
    {
        let mut w = WATCH.lock().unwrap();
        if *w == dev_id {
            return;
        }
        *w = dev_id.to_string();
    }
    if !dev_id.is_empty() && !RUNNING.swap(true, Ordering::AcqRel) {
        std::thread::Builder::new().name("mute talk".into()).spawn(|| util::safely("mute talk", run)).ok();
    }
}

fn run() {
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mut en = com::Enum::default();
    loop {
        watch(&mut en);
        RUNNING.store(false, Ordering::Release);
        // a new microphone may have come while this loop was ending
        if WATCH.lock().unwrap().is_empty() || RUNNING.swap(true, Ordering::AcqRel) {
            return;
        }
    }
}

fn watch(en: &mut com::Enum) {
    let (mut cur, mut meter) = (String::new(), None::<IAudioMeterInformation>);
    let (mut hits, mut i) = ([false; WINDOW], 0usize);
    let mut shown: Option<Instant> = None;
    loop {
        let want = WATCH.lock().unwrap().clone();
        if want.is_empty() {
            return;
        }
        if want != cur {
            meter = en.get().and_then(|e| com::get_device(e, &want)).and_then(|d| unsafe { d.Activate(CLSCTX_ALL, None) }.ok());
            cur = want;
            hits = [false; WINDOW];
        }
        std::thread::sleep(POLL);
        let pk = meter.as_ref().and_then(|m| unsafe { m.GetPeakValue() }.ok()).unwrap_or(0.0);
        hits[i % WINDOW] = pk > LEVEL;
        i += 1;
        if hits.iter().filter(|h| **h).count() >= HITS && shown.is_none_or(|t| t.elapsed() > COOLDOWN) {
            shown = Some(Instant::now());
            hits = [false; WINDOW];
            util::run_on_ui(|| osd::show_text(&i18n::t("Мікрофон вимкнено — вас не чути")));
        }
    }
}
