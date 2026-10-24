//! The audio engine: app sessions, game/chat balance, app and microphone volume, output devices.
//! Runs on a thread of its own with COM (MTA); other threads hand it work through `run`.

mod activity;
mod btbatt;
pub mod com;
mod duck;
pub mod echo;
mod output;
pub mod outprio;
mod procmeter;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicBool, AtomicI32, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use windows::Win32::{
    Media::Audio::{AudioSessionStateActive, AudioSessionStateExpired, eCapture, eRender},
    System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
};

use crate::{appinfo, config, logf, server, state::DuckJson, util};

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct MicInfo {
    pub ok: bool,
    pub name: String,
    pub level: i32,
    pub mute: bool,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct OutInfo {
    pub id: String,
    /// as in Windows: "Headphones (Arctis 7+)"
    pub name: String,
    /// the device name: "Arctis 7+"
    pub short: String,
    /// "24g", "bt", "wire", "virtual", "off" (wireless headset switched off), ""
    pub conn: String,
    /// this is the headset the app works with
    pub headset: bool,
    /// battery of Bluetooth headphones as Windows knows it, -1 — unknown
    pub batt: i32,
    /// the Windows volume of this output, 0..100, and its mute
    pub vol: i32,
    pub mute: bool,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct OutDevJson {
    pub id: String,
    pub name: String,
    /// on | disabled | unplugged | absent (disconnected) | hsoff (headset switched off)
    pub state: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
    /// a virtual cable (listed after physical devices where the order is not the user's)
    #[serde(rename = "virtual", skip_serializing_if = "std::ops::Not::not")]
    pub is_virtual: bool,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct WinCallJson {
    /// a call (communications stream) is on
    pub active: bool,
    pub who: String,
    /// what Windows does to other sounds: 0 mutes, 1 -80 %, 2 -50 %, 3 nothing
    pub pref: i32,
}

/// An app records from a virtual cable that microphone processing normally feeds.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct CableJson {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub who: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub out: String,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct AudioDev {
    pub id: String,
    pub name: String,
    #[serde(rename = "virtual")]
    pub is_virtual: bool,
    /// for a cable: the recording device at its other end
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pair: String,
    /// for a cable: the common part of the name — devices of one cable
    #[serde(skip_serializing_if = "String::is_empty")]
    pub group: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
    /// recording device behind "" (as in Windows)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub auto: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessInfo {
    /// as is (with case)
    pub exe: String,
    pub playing: bool,
    /// the user's volume (without ChatMix), 0..1
    pub vol: f32,
}

// ---------- state shared with other threads ----------

#[derive(Default)]
struct Shared {
    mic: MicInfo,
    out: OutInfo,
    outs: Vec<OutDevJson>,
    sess: Vec<SessInfo>,
    /// pinned apps that run without an audio session
    running: HashSet<String>,
    /// apps audible right now (while the window is shown)
    loud: HashSet<String>,
    cable: CableJson,
    /// new app volumes from the window (applied in apply)
    app_pend: HashMap<String, f32>,
}

static SHARED: LazyLock<Mutex<Shared>> = LazyLock::new(|| Mutex::new(Shared::default()));

fn shared() -> std::sync::MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|p| p.into_inner())
}

pub(crate) static DUCK_ACTIVE: AtomicBool = AtomicBool::new(false);
pub(crate) static DUCK_WATCHING: AtomicBool = AtomicBool::new(false);
static APP_ROUTE_OK: AtomicBool = AtomicBool::new(false);
/// The headset has a separate "Chat" device: ChatMix is mixed in the headset itself.
pub static HW_CHATMIX: AtomicBool = AtomicBool::new(false);

pub fn sessions() -> Vec<SessInfo> {
    shared().sess.clone()
}

pub fn audible(exe: &str) -> bool {
    shared().loud.contains(&exe.to_lowercase())
}

pub fn is_running(exe: &str) -> bool {
    shared().running.contains(&exe.to_lowercase())
}

pub fn mic_state() -> MicInfo {
    shared().mic.clone()
}

pub fn out_state() -> OutInfo {
    shared().out.clone()
}

pub fn outs_state() -> Vec<OutDevJson> {
    shared().outs.clone()
}

pub fn cable_state() -> CableJson {
    shared().cable.clone()
}

pub fn duck_state() -> DuckJson {
    DuckJson { watching: DUCK_WATCHING.load(Ordering::Relaxed), active: DUCK_ACTIVE.load(Ordering::Relaxed) }
}

pub fn app_route_ok() -> bool {
    APP_ROUTE_OK.load(Ordering::Relaxed)
}

pub use activity::{win_call_state, win_ducking_pref};
pub use duck::duck_meter;

// ---------- the engine thread ----------

type Task = Box<dyn FnOnce(&mut Engine) + Send>;

enum Msg {
    /// new balance levels (the latest are in LEVELS)
    Set,
    /// apply the current values again (routing changed)
    Refresh,
    Restore(Sender<()>),
    Task(Task),
    /// Windows reported a device change (handled a bit later: one plug gives several reports)
    DevChanged,
}

static TX: Mutex<Option<Sender<Msg>>> = Mutex::new(None);
static LEVELS: AtomicI32 = AtomicI32::new(100 << 8 | 100);
static SET_PENDING: AtomicBool = AtomicBool::new(false);

fn send(m: Msg) -> bool {
    TX.lock().unwrap().as_ref().is_some_and(|t| t.send(m).is_ok())
}

/// Runs f on the audio engine thread.
pub fn run(f: impl FnOnce(&mut Engine) + Send + 'static) {
    send(Msg::Task(Box::new(f)));
}

pub fn start() {
    let (tx, rx) = mpsc::channel();
    *TX.lock().unwrap() = Some(tx);
    std::thread::Builder::new().name("audio".into()).spawn(move || engine_loop(rx)).ok();
}

/// Game and chat levels of the balance, 0..100. Calls in a row give one apply (the newest values).
pub fn set(game: i32, chat: i32) {
    LEVELS.store((game.clamp(0, 100) << 8) | chat.clamp(0, 100), Ordering::Relaxed);
    if !SET_PENDING.swap(true, Ordering::AcqRel) {
        send(Msg::Set);
    }
}

pub fn reapply() {
    send(Msg::Refresh);
}

/// The app volume (the user's level ChatMix then works on).
pub fn set_app_volume(exe: &str, v: f32) {
    shared().app_pend.insert(exe.to_lowercase(), v.clamp(0.0, 1.0));
    reapply();
}

pub fn set_mic_level(level: i32) {
    run(move |e| e.set_mic_level(level));
}

pub fn set_mic_mute(mute: bool) {
    run(move |e| e.set_mic_mute(mute));
}

pub fn refresh_mic() {
    run(|e| e.refresh_mic());
}

pub fn refresh_out() {
    run(|e| e.refresh_out());
}

/// (from the window) Makes a device Windows' default output.
pub fn set_output(id: &str) {
    let id = id.to_string();
    run(move |e| {
        if com::set_default_endpoint(&id) {
            logf!("output: switched by user");
        }
        e.refresh_out();
    });
}

/// (from the window) The Windows volume and / or mute of the current output.
pub fn set_out_volume(vol: Option<i32>, mute: Option<bool>) {
    run(move |e| {
        let Some(en) = e.en.get().cloned() else { return };
        if let Some(epv) = unsafe { en.GetDefaultAudioEndpoint(eRender, windows::Win32::Media::Audio::eConsole) }.ok().and_then(|d| com::endpoint_volume(&d)) {
            unsafe {
                if let Some(v) = vol {
                    let _ = epv.SetMasterVolumeLevelScalar(v.clamp(0, 100) as f32 / 100.0, std::ptr::null());
                }
                if let Some(m) = mute {
                    let _ = epv.SetMute(m, std::ptr::null());
                }
            }
        }
        e.refresh_out();
    });
}

/// (from the window) The chosen microphone becomes Windows' default too (all roles), so apps that
/// use the default get it as well; the app then follows Windows (""). A virtual cable stays chosen
/// by its id: "" never stands for a cable (com::auto_mic). While Windows' default recording device is
/// a virtual cable (apps listen to the processed microphone there), it is kept: a real microphone is
/// then chosen for the app only.
pub fn set_input(id: &str) {
    let id = id.to_string();
    run(move |e| {
        let virt = e.en.get().cloned().and_then(|en| com::get_device(&en, &id)).is_some_and(|d| outprio::is_virtual_name(&com::friendly_name(&d)));
        let keep = !virt && e.en.get().is_some_and(|en| cable_is_default_mic(en));
        let ok = !keep && com::set_default_endpoint(&id);
        if ok {
            logf!("input: switched by user");
        }
        config::set(|c| c.mic_dev = if ok && !virt { String::new() } else { id.clone() });
        e.refresh_mic();
        std::thread::spawn(|| crate::router::sync(&config::get())); // processing and "listen to yourself" take this microphone
        server::notify_subs();
    });
}

/// (hotkey) Windows' default output moves to the next (step 1) or previous (-1) active physical
/// device, in the order of the device lists, round; the on-screen indicator shows its name. Virtual
/// cables are left out. For input only the app's own microphone (processing, "listen to yourself")
/// moves: Windows' recording device stays as it is (apps listen to the processed one through a cable).
pub fn next_device(input: bool, step: i32) {
    run(move |e| {
        let Some(en) = e.en.get().cloned() else { return };
        let all = com::list_flow(&en, if input { eCapture } else { eRender });
        let own = input;
        let cur = if own {
            let m = config::get().mic_dev;
            if m.is_empty() { com::auto_mic(&all).unwrap_or_default() } else { m }
        } else {
            all.iter().find(|d| d.default).map(|d| d.id.clone()).unwrap_or_default()
        };
        let l: Vec<_> = all.into_iter().filter(|d| !d.is_virtual).collect();
        let n = l.len().max(1) as i32;
        let i = l.iter().position(|d| d.id == cur).map_or(if step < 0 { n - 1 } else { 0 }, |i| (i as i32 + step).rem_euclid(n)) as usize;
        let Some(d) = l.get(i).filter(|d| d.id != cur) else { return };
        if own {
            logf!("input: app microphone switched by hotkey (Windows' default stays)");
            let (id, name) = (d.id.clone(), d.name.clone());
            config::set(|c| c.mic_dev = id);
            util::run_on_ui(move || crate::osd::show(Some(crate::osd::ICON_MIC), &name));
            e.refresh_mic();
            std::thread::spawn(|| crate::router::sync(&config::get()));
            server::notify_subs();
            return;
        }
        if com::set_default_endpoint(&d.id) {
            logf!("{}: switched by hotkey", if input { "input" } else { "output" });
            let name = d.name.clone();
            util::run_on_ui(move || crate::osd::show(Some(if input { crate::osd::ICON_MIC } else { crate::osd::ICON_SPEAKER }), &name));
        }
        if input {
            e.refresh_mic();
        } else {
            e.refresh_out();
        }
    });
}

/// Windows' default recording device is a virtual cable: apps listen to the processed microphone.
fn cable_is_default_mic(en: &windows::Win32::Media::Audio::IMMDeviceEnumerator) -> bool {
    com::list_flow(en, eCapture).iter().any(|d| d.default && d.is_virtual)
}

/// (from the window) up | down | order (ids — the new order) | on | off | forget
pub fn out_dev_op(op: &str, id: &str, ids: Vec<String>) {
    let (op, id) = (op.to_string(), id.to_string());
    run(move |e| e.out_dev_op(&op, &id, ids));
}

/// The window was shown or hidden.
pub fn set_activity(on: bool) {
    run(move |e| e.set_activity(on));
}

/// Output priority was just switched on — apply the order right away.
pub fn auto_switch_now() {
    let on = {
        let h = crate::headset::lock();
        h.connected && h.powered
    };
    run(move |e| {
        e.refresh_outs(true);
        e.switch_mic(on);
    });
}

/// The headset was switched on or off (UI thread): "headset off" in the list, output order and
/// the headset microphone follow.
pub fn headset_power(on: bool) {
    run(move |e| {
        e.refresh_out();
        e.refresh_outs(false);
        e.switch_mic(on);
    });
}

/// Active playback and recording devices (waits for the engine up to 3 s).
pub fn list_devices() -> (Vec<AudioDev>, Vec<AudioDev>) {
    let (tx, rx) = mpsc::channel();
    run(move |e| {
        let r = match e.en.get() {
            Some(en) => com::list_audio_devices(en),
            None => (Vec::new(), Vec::new()),
        };
        let _ = tx.send(r);
    });
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or_default()
}

/// Puts app volumes back and stops (on exit).
pub fn restore_and_stop() {
    let (tx, rx) = mpsc::channel();
    if send(Msg::Restore(tx)) {
        let _ = rx.recv_timeout(Duration::from_millis(1500));
    }
}

pub(crate) fn notify_dev_changed() {
    send(Msg::DevChanged);
}

/// A session ChatMix controls right now. `base` — the level the user set (in the Windows mixer or
/// the app itself); ChatMix multiplies it instead of replacing it, and puts it back.
struct SessVol {
    base: f32,
    last: f32,
    /// the factor `last` was set with
    fac: f32,
    seen: bool,
}

pub struct Engine {
    pub(crate) en: com::Enum,
    game: i32,
    chat: i32,
    ticks: u64,
    /// key — the session instance id
    vols: HashMap<String, SessVol>,
    /// pid → device the app was routed to ("" — the default)
    routed: HashMap<u32, String>,
    routing: Option<com::AppRouting>,
    dk: duck::Ducker,
    pm: procmeter::ProcMeters,
    act: activity::Activity,
    wincall: activity::WinCall,
    devs: output::DevWatch,
    sig: String,
    mic_hold_log: Option<Instant>,
    bt_batt: btbatt::Cache,
    /// the current output's volume control with the watch on it (output.rs)
    out_vol: Option<(String, windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume, windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolumeCallback)>,
    vol_restore_log: HashMap<String, Instant>,
    endpoints_log: String,
}

pub(crate) static SELF_PID: LazyLock<u32> = LazyLock::new(std::process::id);
/// The app's own sound among sessions ("listen to yourself", microphone sounds).
pub(crate) const SELF_EXE: &str = "arcmix.exe";

/// Apps that play the already mixed sound of others (Windows effects, Sonar): their level is the
/// level of all sound, not their own.
fn mixes_others(exe: &str) -> bool {
    exe == "audiodg.exe" || exe == "steelseriessonar.exe"
}

fn engine_loop(rx: Receiver<Msg>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mut e = Engine {
        en: com::Enum::default(),
        game: 100,
        chat: 100,
        ticks: 0,
        vols: HashMap::new(),
        routed: HashMap::new(),
        routing: None,
        dk: duck::Ducker::default(),
        pm: procmeter::ProcMeters::default(),
        act: activity::Activity::default(),
        wincall: activity::WinCall::default(),
        devs: output::DevWatch::default(),
        sig: String::new(),
        mic_hold_log: None,
        bt_batt: btbatt::Cache::default(),
        out_vol: None,
        vol_restore_log: HashMap::new(),
        endpoints_log: String::new(),
    };
    util::safely("audio start", || {
        e.refresh_mic();
        e.refresh_out();
        e.log_devices();
        e.detect_hw_chatmix();
        let r = com::AppRouting::new(); // check up front whether apps can be routed automatically
        APP_ROUTE_OK.store(r.ok(), Ordering::Relaxed);
        e.routing = Some(r);
        e.watch_win_calls();
        e.watch_devices();
        e.refresh_outs(false);
    });
    let now = Instant::now();
    // after 4 s the headset state is known — then the device order is applied
    let mut start_due = Some(now + Duration::from_millis(4500));
    let mut next_tick = now + Duration::from_secs(3);
    let mut dev_due: Option<Instant> = None;
    loop {
        let now = Instant::now();
        let mut deadline = next_tick;
        for d in [start_due, dev_due, e.dk.next, e.act.next].into_iter().flatten() {
            deadline = deadline.min(d);
        }
        match rx.recv_timeout(deadline.saturating_duration_since(now)) {
            Ok(Msg::Set) => {
                SET_PENDING.store(false, Ordering::Release);
                let l = LEVELS.load(Ordering::Relaxed);
                (e.game, e.chat) = (l >> 8, l & 0xff);
                util::safely("audio", || e.apply(false));
            }
            Ok(Msg::Refresh) => {
                util::safely("audio", || e.apply(false));
            }
            Ok(Msg::Restore(done)) => {
                util::safely("audio restore", || {
                    (e.game, e.chat) = (100, 100);
                    e.apply(true);
                });
                e.pm.sync(false);
                let _ = done.send(());
                return;
            }
            Ok(Msg::Task(f)) => {
                util::safely("audio task", || f(&mut e));
            }
            Ok(Msg::DevChanged) => dev_due = Some(Instant::now() + Duration::from_millis(400)),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        if e.dk.next.is_some_and(|t| now >= t) {
            e.dk.next = Some(now + duck::DUCK_TICK);
            util::safely("duck", || e.duck_step());
        }
        if e.act.next.is_some_and(|t| now >= t) {
            e.act.next = Some(now + activity::ACT_TICK);
            util::safely("activity", || e.act_step());
        }
        if dev_due.is_some_and(|t| now >= t) {
            dev_due = None;
            util::safely("devices", || {
                e.refresh_out();
                e.refresh_outs(false);
            });
        }
        if start_due.is_some_and(|t| now >= t) {
            start_due = None;
            util::safely("devices", || e.refresh_outs(false));
        }
        if now >= next_tick {
            next_tick = now + Duration::from_secs(3);
            util::safely("audio", || {
                e.apply(false);
                e.refresh_mic();
                e.refresh_cable();
                e.refresh_out();
                e.watch_win_calls(); // the default output may have changed
            });
            if e.ticks % 5 == 0 {
                // every 15 s: the enumerator may have been recreated
                util::safely("devices", || e.heal_devices());
                util::safely("devices", || {
                    e.watch_devices();
                    e.refresh_outs(false);
                });
            }
            e.ticks += 1;
            if e.ticks % 10 == 0 {
                // every 30 s: the headset may have been plugged in later
                e.detect_hw_chatmix();
                util::prune_path_cache();
            }
        }
    }
}

impl Engine {
    /// Windows sometimes gives a device a new id (e.g. after VB-CABLE is reinstalled). Then the
    /// chosen cable or microphone "disappears" although the same device is there. The names of the
    /// chosen devices are remembered and, when an id is gone, the device is found by name.
    fn heal_devices(&mut self) {
        let Some(en) = self.en.get() else { return };
        let (render, capture) = com::list_audio_devices(en);
        if render.is_empty() && capture.is_empty() {
            return;
        }
        let c = config::get();
        let mut refs = vec![(&c.mic_fx.out, true), (&c.route.game, true), (&c.route.chat, true), (&c.route.media, true), (&c.route.out, true), (&c.mic_dev, false)];
        refs.extend(c.app_outs.values().map(|id| (id, true)));
        let mut names: BTreeMap<String, String> = BTreeMap::new();
        // old id → new id
        let mut fix: HashMap<String, String> = HashMap::new();
        for (id, is_render) in refs {
            if id.is_empty() {
                continue;
            }
            let list = if is_render { &render } else { &capture };
            if let Some(d) = list.iter().find(|d| d.id == *id) {
                names.insert(id.clone(), d.name.clone());
                continue;
            }
            let Some(name) = c.dev_names.get(id.as_str()).filter(|n| !n.is_empty()) else { continue };
            names.insert(id.clone(), name.clone()); // the device is unplugged — its name is kept
            if let Some(d) = list.iter().find(|d| d.name == *name) {
                fix.insert(id.clone(), d.id.clone());
                names.remove(id.as_str());
                names.insert(d.id.clone(), name.clone());
                logf!("device {name:?} has a new id — using it");
            }
        }
        if names == c.dev_names && fix.is_empty() {
            return;
        }
        let changed = !fix.is_empty();
        config::set(move |c| {
            let ids = [&mut c.mic_fx.out, &mut c.route.game, &mut c.route.chat, &mut c.route.media, &mut c.route.out, &mut c.mic_dev].into_iter().chain(c.app_outs.values_mut());
            for id in ids {
                if let Some(n) = fix.get(id.as_str()) {
                    *id = n.clone();
                }
            }
            c.dev_names = names;
        });
        if changed {
            util::post_ui(crate::WM_CFG_CHG, 0);
        }
    }

    fn apply(&mut self, restore: bool) {
        let (game, chat) = (self.game, self.chat);
        let cfg = config::get();
        for v in self.vols.values_mut() {
            v.seen = false;
        }
        let route_on = !restore && cfg.route.auto && crate::router::route_live();
        let collect = self.duck_begin(&cfg, restore);
        self.act_begin();
        // devices whose sound ducking and the "playing" mark listen to
        let listen = if collect || self.act.on() { self.listen_endpoints(&cfg) } else { HashSet::new() };
        if collect {
            self.duck_endpoints(&listen);
        }
        let pend = std::mem::take(&mut shared().app_pend);
        let mut seen: BTreeMap<String, SessInfo> = BTreeMap::new();
        let mut seen_pid = HashSet::new();
        let mut save_pv: HashMap<String, f32> = HashMap::new();
        let (mut add_routed, mut del_routed) = (Vec::new(), Vec::new());
        let sessions = match self.en.get().map(com::sessions) {
            Some(Some(s)) => s,
            Some(None) => {
                self.en.reset(); // try to create it anew next time
                Vec::new()
            }
            None => Vec::new(),
        };
        for s in &sessions {
            if s.pid == *SELF_PID && collect && listen.contains(&s.dev_id) && !s.sys {
                // own sound ("listen to yourself", mic sounds): untouched, but counted to recognise a
                // chat session that repeats everything playing on the headset
                self.duck_collect("self", &s.key, SELF_EXE, s.pid, s, true);
                continue;
            }
            if s.pid == 0 || s.state == AudioSessionStateExpired || s.pid == *SELF_PID {
                continue;
            }
            let full = util::exe_path_of(s.pid);
            // a helper process counts as its app (Steam Client WebHelper as Steam)
            let exe = appinfo::app_of(&full);
            // audiodg.exe (Windows Audio Device Graph Isolation) plays system streams, not an app's own
            // sound: it is neither listed, nor balanced, ducked or routed.
            if exe.is_empty() || exe == "audiodg.exe" {
                continue;
            }
            let playing = s.state == AudioSessionStateActive;
            let shown = if exe.eq_ignore_ascii_case(util::base_name(&full)) { util::base_name(&full).to_string() } else { exe.clone() };
            seen.entry(exe.clone()).and_modify(|x| x.playing |= playing).or_insert(SessInfo { exe: shown, playing, vol: 0.0 });
            let cat = cfg.category(&exe);
            if seen_pid.insert(s.pid) {
                match self.route_app(s.pid, &exe, cat, &cfg, route_on) {
                    1 => add_routed.push(exe.clone()),
                    -1 => del_routed.push(exe.clone()),
                    _ => {}
                }
            }
            // the level is taken only from the headset (and routing cables); "System sounds" and apps
            // playing the already mixed sound of all others are skipped
            let meter = listen.contains(&s.dev_id) && !s.sys && !mixes_others(&exe);
            if collect {
                self.duck_collect(cat, &s.key, &exe, s.pid, s, meter);
            }
            if playing && meter {
                self.act_collect(&exe, &s.key, cat, s.pid, s);
                if cat == "chat" {
                    self.pm.want_pid(s.pid, &exe);
                }
            }
            let mut factor = 1.0f32; // media: ChatMix does not apply
            match cat {
                "game" => factor = game as f32 / 100.0 * self.dk.cur,
                "chat" => factor = chat as f32 / 100.0,
                _ if self.dk.media => factor = self.dk.cur,
                _ => {}
            }
            if restore {
                factor = 1.0;
            }
            let mut cur = com::get_vol(&s.sav);
            let (mut pv, mut has_pend) = match pend.get(&exe) {
                Some(&v) => (v, true), // app volume set in the window
                None => (0.0, false),
            };
            if has_pend {
                save_pv.insert(exe.clone(), pv);
            } else if let Some(&v) = cfg.app_vols.get(&exe) {
                if !restore && cfg.on("volmem") {
                    // saved volume: Windows (or the app itself) may have reset it — put it back
                    (pv, has_pend) = (v as f32, true);
                }
            }
            let note = |seen: &mut BTreeMap<String, SessInfo>, v: f32| {
                if let Some(x) = seen.get_mut(&exe) {
                    if v > x.vol {
                        x.vol = v;
                    }
                }
            };
            if !self.vols.contains_key(&s.key) {
                if factor >= 0.999 {
                    if has_pend && !restore && (cur - pv).abs() > 0.004 {
                        if !pend.contains_key(&exe) {
                            self.log_vol_restore(&exe, cur, pv);
                        }
                        com::set_vol(&s.sav, pv);
                        cur = pv;
                    }
                    note(&mut seen, cur);
                    continue; // ChatMix leaves this session alone
                }
                let base = if has_pend { pv } else { cur };
                self.vols.insert(s.key.clone(), SessVol { base, last: cur, fac: 1.0, seen: false });
            } else if let Some(sv) = self.vols.get_mut(&s.key) {
                if has_pend {
                    sv.base = pv;
                } else if (cur - sv.last).abs() > 0.01 {
                    // the user changed the volume — it becomes the new base level
                    if sv.fac > 0.05 {
                        sv.base = (cur / sv.fac).min(1.0);
                    } else if factor > 0.05 {
                        sv.base = (cur / factor).min(1.0);
                    }
                }
            }
            let sv = self.vols.get_mut(&s.key).unwrap();
            sv.seen = true;
            let target = sv.base * factor;
            let d = cur - target;
            if d.abs() > 0.004 || (factor >= 0.999 && d != 0.0) {
                com::set_vol(&s.sav, target);
                sv.last = target;
            } else {
                sv.last = cur;
            }
            sv.fac = factor;
            let base = sv.base;
            note(&mut seen, base);
            if factor >= 0.999 {
                self.vols.remove(&s.key); // the user's level is back — no more tracking
            }
        }
        self.vols.retain(|_, v| v.seen); // finished sessions
        self.duck_end(game);
        // a volume set for an app with no sound yet is kept: volume memory puts it in place once it plays
        for (exe, &v) in &pend {
            if !seen.contains_key(exe) {
                save_pv.insert(exe.clone(), v);
            }
        }
        if !save_pv.is_empty() {
            config::set_later(|c| {
                for (k, v) in save_pv {
                    c.app_vols.insert(k, (v as f64 * 1000.0).round() / 1000.0);
                }
            });
        }
        // chat apps' own sound is captured only while there is something to duck or the window is shown
        let want_pm = !restore && (self.dk.next.is_some() || self.act.on());
        self.pm.sync(want_pm);
        self.routed.retain(|pid, _| seen_pid.contains(pid));
        if !add_routed.is_empty() || !del_routed.is_empty() {
            config::set(|c| {
                c.routed_apps.retain(|x| !del_routed.contains(x));
                for x in add_routed {
                    if !c.routed_apps.contains(&x) {
                        c.routed_apps.push(x);
                    }
                }
            });
        }
        let mut list: Vec<SessInfo> = seen.values().cloned().collect();
        list.sort_by_key(|s| s.exe.to_lowercase());
        let mut sig = String::new();
        for s in &list {
            sig += &s.exe;
            if s.playing {
                sig.push('*');
            }
            sig += &format!("{}|", (s.vol * 100.0 + 0.5) as i32);
        }
        // pinned apps without an audio session: whether they run at all (a snapshot of all processes —
        // only while the window is shown: it is visible only there; showing the window recounts)
        let mut running = shared().running.clone();
        if crate::fly::SHOWN.load(Ordering::Relaxed) {
            running.clear();
            let mut procs: Option<HashMap<String, u32>> = None;
            for p in &cfg.pinned {
                if p.is_empty() || seen.contains_key(p) {
                    continue;
                }
                let procs = procs.get_or_insert_with(appinfo::running_procs);
                if let Some(&pid) = procs.get(p) {
                    running.insert(p.clone());
                    if appinfo::path_for(p).is_empty() {
                        appinfo::remember_path(p, &util::exe_path_of(pid)); // for the icon
                    }
                }
            }
        }
        for p in &cfg.pinned {
            if running.contains(p) && !seen.contains_key(p) {
                sig += &format!("~{p}");
            }
        }
        let changed = sig != self.sig;
        self.sig = sig;
        {
            let mut sh = shared();
            sh.sess = list;
            sh.running = running;
        }
        if changed {
            server::notify_subs();
        }
    }

    /// Sends the app to its own output device (chosen in its menu), otherwise into the cable of its
    /// category (when routing runs) or back to the default device. Apps not touched by us are left
    /// alone (the user's manual settings). 1 — routed, -1 — put back, 0 — no change.
    fn route_app(&mut self, pid: u32, exe: &str, cat: &str, cfg: &config::Config, route_on: bool) -> i32 {
        let want = if let Some(dev) = cfg.app_outs.get(exe) {
            dev.clone()
        } else if route_on {
            cfg.route_cable(cat).to_string()
        } else {
            String::new()
        };
        let known = self.routed.get(&pid).cloned();
        if known.is_none() && want.is_empty() && !cfg.routed_apps.iter().any(|x| x == exe) {
            self.routed.insert(pid, String::new());
            return 0;
        }
        if known.as_deref() == Some(want.as_str()) {
            return 0;
        }
        let ok = self.routing.as_ref().is_some_and(|r| r.set(pid, &want));
        self.routed.insert(pid, want.clone()); // even if it failed — not again every three seconds
        if !ok {
            return 0;
        }
        if want.is_empty() {
            logf!("app {exe} → default output");
            -1
        } else {
            logf!("app {exe} → routed");
            1
        }
    }

    fn log_vol_restore(&mut self, exe: &str, was: f32, now: f32) {
        if self.vol_restore_log.get(exe).is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return;
        }
        if self.vol_restore_log.len() > 64 {
            self.vol_restore_log.clear();
        }
        self.vol_restore_log.insert(exe.to_string(), Instant::now());
        logf!("volume: {exe} was {:.0}%, restored to {:.0}%", was * 100.0, now * 100.0);
    }

    // ---------- microphone (Windows recording volume) ----------

    /// The microphone chosen in the app (id), otherwise "" — the one "As in Windows" stands for.
    fn with_mic<R>(&mut self, id: &str, f: impl FnOnce(&windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume, &str, &str) -> R) -> Option<R> {
        let en = self.en.get()?.clone();
        let dev = com::get_device(&en, id).or_else(|| com::get_device(&en, &com::auto_mic_id(&en)))?;
        let name = com::friendly_name(&dev);
        let epv = com::endpoint_volume(&dev)?;
        Some(f(&epv, &name, &com::device_id(&dev)))
    }

    pub(crate) fn refresh_mic(&mut self) {
        let c = config::get();
        let mut mi = MicInfo::default();
        let mut log_held = None;
        let hold_log = self.mic_hold_log;
        let r = self.with_mic(&c.mic_dev, |epv, name, dev_id| unsafe {
            let v = epv.GetMasterVolumeLevelScalar().unwrap_or(0.0);
            let m = epv.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            mi.name = name.to_string();
            mi.level = (v * 100.0).round() as i32;
            mi.mute = m;
            if !c.on("volmem") {
                return;
            }
            // the mic volume was changed by someone else (Windows, auto gain in Discord…) — ours back
            let want = c.mic_levels.get(dev_id).copied().unwrap_or(100);
            if mi.level != want {
                let _ = epv.SetMasterVolumeLevelScalar(want as f32 / 100.0, std::ptr::null());
                if hold_log.is_none_or(|t: Instant| t.elapsed() > Duration::from_secs(60)) {
                    log_held = Some((mi.level, want));
                }
                mi.level = want;
            }
        });
        mi.ok = r.is_some();
        if let Some((was, want)) = log_held {
            self.mic_hold_log = Some(Instant::now());
            logf!("mic: volume was changed to {was}% outside the app — back to {want}%");
        }
        let changed = {
            let mut sh = shared();
            let ch = sh.mic != mi;
            sh.mic = mi;
            ch
        };
        if changed {
            server::notify_subs();
            util::post_ui(crate::WM_MIC_CHG, 0); // the badge on the tray icon
        }
    }

    fn set_mic_level(&mut self, level: i32) {
        let dev = config::get().mic_dev;
        let id = self.with_mic(&dev, |epv, _, dev_id| unsafe {
            let _ = epv.SetMasterVolumeLevelScalar(level as f32 / 100.0, std::ptr::null());
            dev_id.to_string()
        });
        if let Some(id) = id.filter(|i| !i.is_empty()) {
            // remember: exactly this volume is held
            config::set_later(|c| {
                c.mic_levels.insert(id, level);
            });
        }
        self.refresh_mic();
    }

    /// Unmutes the app's microphone right away (on exit with push-to-talk on).
    pub(crate) fn unmute_mic(&mut self) {
        let dev = config::get().mic_dev;
        self.with_mic(&dev, |epv, _, _| unsafe {
            let _ = epv.SetMute(false, std::ptr::null());
        });
    }

    fn set_mic_mute(&mut self, m: bool) {
        let dev = config::get().mic_dev;
        self.with_mic(&dev, |epv, _, _| unsafe {
            let _ = epv.SetMute(m, std::ptr::null());
        });
        self.refresh_mic();
    }

    /// The headset has a separate "Chat" device — ChatMix mixes in the headset, the app does not
    /// need to change app volumes.
    fn detect_hw_chatmix(&mut self) {
        let Some(en) = self.en.get().cloned() else { return };
        let found = com::list_flow(&en, windows::Win32::Media::Audio::eRender).iter().any(|d| {
            let n = d.name.to_lowercase();
            n.contains("chat") && (n.contains("arctis") || n.contains("steelseries"))
        });
        HW_CHATMIX.store(found, Ordering::Relaxed);
    }

    /// Names of the audio devices in the log (for diagnostics).
    fn log_devices(&mut self) {
        let Some(en) = self.en.get().cloned() else { return };
        for (flow, title) in [(windows::Win32::Media::Audio::eRender, "output"), (eCapture, "capture")] {
            for d in com::list_flow(&en, flow) {
                logf!("{title} device: {}", d.name);
            }
        }
    }
}
