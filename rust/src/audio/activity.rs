//! What really sounds, and what only keeps an open stream.
//!
//! Windows calls a session "active" as soon as an app opened an audio stream — even with silence in
//! it (Discord in an empty voice channel). So the "playing" mark in the window follows the session
//! level meter, and only while the window is shown.
//!
//! Also here: Windows' own ducking of sounds during calls ("Sound → Communications") — easy to mix
//! up with ChatMix ducking — and an app recording from a silent virtual cable.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicI32, Ordering},
    },
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Media::Audio::{
            AudioSessionStateActive, Endpoints::IAudioMeterInformation, IAudioSessionControl2, IAudioSessionManager2, IAudioVolumeDuckNotification, IAudioVolumeDuckNotification_Impl,
            eCapture, eRender,
        },
        System::Com::{CLSCTX_ALL, CoTaskMemFree},
    },
    core::{Interface, PCWSTR, implement},
};

use super::{
    CableJson, Engine, SELF_PID, com,
    echo::{ECHO_CORR, LvlHist, echo_of},
    shared,
};
use crate::{appinfo, config::Config, logf, server, util};

pub const ACT_TICK: Duration = Duration::from_millis(50);
/// ≈ −56 dB: quieter counts as silence
const ACT_MIN_PEAK: f64 = 0.0015;
const ACT_HOLD: Duration = Duration::from_millis(1500);

pub(super) struct ActMeter {
    exe: String,
    key: String,
    cat: String,
    pid: u32,
    meter: IAudioMeterInformation,
}

#[derive(Default)]
pub(super) struct Activity {
    meters: Vec<ActMeter>,
    /// session level history — not to call a copy of foreign sound "playing"
    hist: HashMap<String, LvlHist>,
    /// when the app was last audible
    last: HashMap<String, Instant>,
    /// the next step (None while the window is hidden)
    pub next: Option<Instant>,
    db: Vec<f64>,
    direct: Vec<bool>,
}

impl Activity {
    pub fn on(&self) -> bool {
        self.next.is_some()
    }
}

impl Engine {
    pub(super) fn act_begin(&mut self) {
        self.act.meters.clear();
    }

    /// (in apply) The session level meter, if the window is shown.
    pub(super) fn act_collect(&mut self, exe: &str, key: &str, cat: &str, pid: u32, s: &com::Session) {
        if !self.act.on() {
            return;
        }
        if let Ok(m) = s.ctl.cast::<IAudioMeterInformation>() {
            self.act.meters.push(ActMeter { exe: exe.into(), key: key.into(), cat: cat.into(), pid, meter: m });
        }
    }

    /// The window was shown or hidden.
    pub(super) fn set_activity(&mut self, on: bool) {
        if on == self.act.on() {
            return;
        }
        if on {
            self.act.next = Some(Instant::now() + ACT_TICK);
            self.act.hist.clear();
            self.apply(false); // collect the meters right away
            self.refresh_cable();
            return;
        }
        self.act.next = None;
        self.act_begin();
        shared().loud.clear();
        self.refresh_cable();
    }

    pub(super) fn act_step(&mut self) {
        let now = Instant::now();
        let a = &mut self.act;
        let n = a.meters.len();
        a.db.resize(n, -120.0);
        a.direct.resize(n, false);
        let mut by_pid: HashMap<u32, f64> = HashMap::new();
        for i in 0..n {
            let m = &a.meters[i];
            a.db[i] = -120.0;
            a.direct[i] = false;
            let pm = self.pm.get(m.pid).filter(|_| m.cat == "chat");
            if let Some(pm) = pm {
                a.db[i] = *by_pid.entry(m.pid).or_insert_with(|| pm.take(1));
                a.direct[i] = true; // the level is from the app's own sound, not the session meter
            } else if let Ok(v) = unsafe { m.meter.GetPeakValue() } {
                a.db[i] = if v > 1e-6 { 20.0 * (v as f64).log10() } else { -120.0 };
            }
            match a.hist.get_mut(&m.key) {
                Some(h) => h.push(a.db[i]),
                None => {
                    let mut h = LvlHist::default();
                    h.push(a.db[i]);
                    a.hist.insert(m.key.clone(), h);
                }
            }
        }
        let min_db = 20.0 * ACT_MIN_PEAK.log10();
        for i in 0..n {
            let m = &a.meters[i];
            if a.db[i] <= min_db {
                continue;
            }
            let mut copied = false;
            if m.cat == "chat" && !a.direct[i] {
                // chat that only repeats another app's sound is not "playing"
                copied = self.dk.mirror.contains(&m.key);
                let hm = a.hist.get(&m.key).copied().unwrap_or_default();
                for o in &a.meters {
                    if copied {
                        break;
                    }
                    if o.cat != "chat" && echo_of(&hm, &a.hist.get(&o.key).copied().unwrap_or_default(), 4) >= ECHO_CORR {
                        copied = true;
                    }
                }
            }
            if !copied {
                a.last.insert(m.exe.clone(), now);
            }
        }
        if a.hist.len() > 128 {
            a.hist.clear();
        }
        a.last.retain(|_, t| now.duration_since(*t) < Duration::from_secs(60));
        // who is audible now; the table changes only when something changed (the window reads it)
        let loud_now: Vec<&String> = a.last.iter().filter(|(_, t)| now.duration_since(**t) < ACT_HOLD).map(|(k, _)| k).collect();
        let changed = {
            let mut sh = shared();
            let changed = loud_now.len() != sh.loud.len() || loud_now.iter().any(|e| !sh.loud.contains(*e));
            if changed {
                sh.loud = loud_now.into_iter().cloned().collect();
            }
            changed
        };
        if changed {
            server::notify_subs();
        }
    }

    /// Which app records from a virtual cable. Checked only while the window is shown — the
    /// warning about a silent cable is visible only there.
    pub(super) fn refresh_cable(&mut self) {
        let mut cj = CableJson::default();
        if crate::fly::SHOWN.load(Ordering::Relaxed) {
            if let Some(en) = self.en.get().cloned() {
                cj = cable_listener(&en, &crate::config::get());
            }
        }
        let changed = {
            let mut sh = shared();
            let ch = sh.cable != cj;
            sh.cable = cj;
            ch
        };
        if changed {
            server::notify_subs();
        }
    }

    /// Subscribes to Windows' call ducking notifications for the default output.
    pub(super) fn watch_win_calls(&mut self) {
        let Some(en) = self.en.get().cloned() else { return };
        let id = com::default_id(&en, eRender);
        let w = &mut self.wincall;
        if id == w.dev && (w.mgr.is_some() || id.is_empty()) {
            return;
        }
        if let (Some(mgr), Some(obj)) = (w.mgr.take(), w.obj.as_ref()) {
            unsafe {
                let _ = mgr.UnregisterDuckNotification(obj);
            }
        }
        w.dev = id.clone();
        let Some(dev) = com::get_device(&en, &id) else { return };
        let Ok(mgr) = (unsafe { dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }) else { return };
        let obj = w.obj.get_or_insert_with(|| DuckNotify.into()).clone();
        // NULL — all notifications
        if let Err(e) = unsafe { mgr.RegisterDuckNotification(PCWSTR::null(), &obj) } {
            logf!("windows: RegisterDuckNotification {e}");
            return;
        }
        w.mgr = Some(mgr);
    }
}

/// The first app with an active recording session on the recording end of a virtual cable.
/// Cables used by routing are skipped: apps like OBS record game or chat sound from them.
fn cable_listener(en: &windows::Win32::Media::Audio::IMMDeviceEnumerator, c: &Config) -> CableJson {
    let (render, capture) = com::list_audio_devices(en);
    for r in &render {
        if !r.is_virtual || r.pair.is_empty() || (c.route.on && ["game", "chat", "media"].iter().any(|k| c.route_cable(k) == r.id)) {
            continue;
        }
        for cp in capture.iter().filter(|cp| cp.name == r.pair) {
            let exe = capture_app(en, &cp.id);
            if !exe.is_empty() {
                return CableJson { who: appinfo::display_name(&exe), name: cp.name.clone(), out: r.id.clone() };
            }
        }
    }
    CableJson::default()
}

/// The exe of another app recording from the device id right now ("" — none).
fn capture_app(en: &windows::Win32::Media::Audio::IMMDeviceEnumerator, id: &str) -> String {
    let Some(dev) = com::get_device(en, id) else { return String::new() };
    let mut out = Vec::new();
    com::sessions_of(&dev, id, &mut out);
    for s in out {
        if s.state != AudioSessionStateActive || s.pid == 0 || s.pid == *SELF_PID {
            continue;
        }
        let exe = appinfo::app_of(&util::exe_path_of(s.pid));
        if !exe.is_empty() {
            return exe;
        }
    }
    let _ = eCapture;
    String::new()
}

// ---------- Windows' own ducking during calls ----------

/// calls (communication streams) running
static CALLS: AtomicI32 = AtomicI32::new(0);
/// the app that started the call
static WHO: Mutex<String> = Mutex::new(String::new());

#[derive(Default)]
pub(super) struct WinCall {
    obj: Option<IAudioVolumeDuckNotification>,
    mgr: Option<IAudioSessionManager2>,
    dev: String,
}

/// "…|\Device\HarddiskVolume3\…\Discord.exe%b{…}" → "Discord.exe".
fn session_exe(id: &str) -> String {
    let mut s = id;
    if let Some(i) = s.find('|') {
        s = &s[i + 1..];
    }
    if let Some(i) = s.find("%b") {
        s = &s[..i];
    }
    util::base_name(s).to_string()
}

#[implement(IAudioVolumeDuckNotification)]
struct DuckNotify;

impl IAudioVolumeDuckNotification_Impl for DuckNotify_Impl {
    fn OnVolumeDuckNotification(&self, session: &PCWSTR, count: u32) -> windows::core::Result<()> {
        let who = session_exe(&unsafe { session.to_string() }.unwrap_or_default());
        logf!("windows: call started in {who} ({count} active); Windows ducking preference {} (3 = do nothing)", win_ducking_pref());
        *WHO.lock().unwrap() = who;
        CALLS.store(count.max(1) as i32, Ordering::Relaxed);
        server::notify_subs();
        Ok(())
    }

    fn OnVolumeUnduckNotification(&self, session: &PCWSTR) -> windows::core::Result<()> {
        if CALLS.fetch_sub(1, Ordering::Relaxed) <= 1 {
            CALLS.store(0, Ordering::Relaxed);
        }
        logf!("windows: call ended in {}", session_exe(&unsafe { session.to_string() }.unwrap_or_default()));
        server::notify_subs();
        Ok(())
    }
}

pub fn win_call_state() -> super::WinCallJson {
    let who = WHO.lock().unwrap().clone();
    let who = who.strip_suffix(".exe").or_else(|| who.strip_suffix(".EXE")).unwrap_or(&who).to_string();
    super::WinCallJson { active: CALLS.load(Ordering::Relaxed) > 0, who, pref: win_ducking_pref() }
}

/// What Windows does to other sounds during a call (0 — mutes, 1 — 80 % quieter (default),
/// 2 — 50 %, 3 — nothing).
pub fn win_ducking_pref() -> i32 {
    util::reg_open(r"Software\Microsoft\Multimedia\Audio").and_then(|k| k.dword("UserDuckingPreference")).map(|v| v as i32).unwrap_or(1)
}

#[allow(dead_code)]
fn _unused(_: IAudioSessionControl2) {
    let _ = CoTaskMemFree;
}
