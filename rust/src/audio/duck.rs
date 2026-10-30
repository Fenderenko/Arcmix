//! Ducking the game when someone talks in chat.
//!
//! Several dozen times a second the level of "Chat" apps is read (the Windows session level
//! meter — the same as in the volume mixer) and "Game" apps (optionally "Media" too) are smoothly
//! multiplied by the chosen percentage. When chat has been silent for a second, the level before
//! ducking comes back the same way (with ChatMix and manual changes counted).
//!
//! So that a constant quiet background (other people's mic noise, "silence" with hiss) does not
//! keep the game ducked, each chat app has its background level tracked, and only sound well above
//! it counts. Everything runs on the audio engine thread — where ChatMix runs.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Media::Audio::{Endpoints::IAudioMeterInformation, ISimpleAudioVolume, eRender},
        System::Com::CLSCTX_ALL,
    },
    core::Interface,
};

use super::{
    DUCK_ACTIVE, DUCK_WATCHING, Engine, SELF_EXE, com,
    echo::{ECHO_CORR, ECHO_MIN_N, LvlHist, echo_of, mirror_evidence, residual_db},
    outprio::is_virtual_name,
};
use crate::{config::Config, logf, server};

pub const DUCK_TICK: Duration = Duration::from_millis(25);
/// Quieter is not talk (a normal voice in Discord is −30 dB and louder).
const DUCK_MIN_DB: f64 = -42.0;
/// A voice is at least this much above the background.
const DUCK_OVER_FLOOR: f64 = 12.0;
/// The background does not go lower (otherwise any rustle is "voice").
const DUCK_FLOOR_MIN: f64 = -75.0;
/// dB per measurement (≈4 dB/s): a constant sound becomes background in a few seconds.
const DUCK_FLOOR_RISE: f64 = 0.1;
const DUCK_HOLD: Duration = Duration::from_secs(1);

fn attack() -> f32 {
    1.0 - (-25.0f32 / 60.0).exp() // ≈ 0.15 s to 90 %
}

fn release() -> f32 {
    1.0 - (-25.0f32 / 250.0).exp() // ≈ 0.6 s to 90 %
}

pub(super) struct DuckMeter {
    pub exe: String,
    pub key: String,
    pub pid: u32,
    pub meter: IAudioMeterInformation,
    /// output device of the session
    pub dev: String,
    /// for other apps: their level with their volume
    pub sav: Option<ISimpleAudioVolume>,
}

pub(super) struct DuckGame {
    key: String,
    sav: ISimpleAudioVolume,
    /// game (ChatMix applies too), otherwise media
    game: bool,
    /// the app, for the log
    exe: String,
}

pub(super) struct Ducker {
    /// chat apps
    meters: Vec<DuckMeter>,
    /// other apps sounding now: to recognise a copy of their sound in chat
    pub others: Vec<DuckMeter>,
    hist: HashMap<String, LvlHist>,
    games: Vec<DuckGame>,
    /// background level of every chat session, dB (lives between applies)
    floor: HashMap<String, f64>,
    /// game factor while someone talks (1 — off)
    level: f32,
    /// duck "Media" too
    pub media: bool,
    /// the current factor
    pub cur: f32,
    /// ChatMix for the game
    game_factor: f32,
    /// measurements in a row with voice in chat
    above: i32,
    last_voice: Option<Instant>,
    speaking: bool,
    /// the next step (None when there is nothing to watch)
    pub next: Option<Instant>,
    logged: String,
    echo_log: String,
    echo_msg: String,
    echo_at: Option<Instant>,
    loud_db: f64,
    loud_name: String,
    // duck_step working tables (40 times a second) — the same every time, no new allocations
    post: HashMap<String, f64>,
    ep_now: HashMap<String, f64>,
    sum_lin: HashMap<String, f64>,
    by_pid: HashMap<u32, f64>,
    // A chat session that repeats everything playing on the headset (Discord has one during a call):
    // compared with the device meter and the sound of other apps.
    eps: HashMap<String, IAudioMeterInformation>,
    ep_hist: HashMap<String, LvlHist>,
    post_hist: HashMap<String, LvlHist>,
    pub mirror: HashSet<String>,
    mirror_score: HashMap<String, i32>,
}

impl Default for Ducker {
    fn default() -> Self {
        Ducker {
            meters: Vec::new(),
            others: Vec::new(),
            hist: HashMap::new(),
            games: Vec::new(),
            floor: HashMap::new(),
            level: 1.0,
            media: false,
            cur: 1.0,
            game_factor: 1.0,
            above: 0,
            last_voice: None,
            speaking: false,
            next: None,
            logged: String::new(),
            echo_log: String::new(),
            echo_msg: String::new(),
            echo_at: None,
            loud_db: -120.0,
            loud_name: String::new(),
            post: HashMap::new(),
            ep_now: HashMap::new(),
            sum_lin: HashMap::new(),
            by_pid: HashMap::new(),
            eps: HashMap::new(),
            ep_hist: HashMap::new(),
            post_hist: HashMap::new(),
            mirror: HashSet::new(),
            mirror_score: HashMap::new(),
        }
    }
}

// ---------- for the window ----------

static CHAT_DB: AtomicI32 = AtomicI32::new(-120);
static THR_DB: AtomicI32 = AtomicI32::new(-42);
static VOICE: AtomicBool = AtomicBool::new(false);
/// "Discord.exe ← chrome.exe" when the chat sound is a copy of another app
static ECHO: Mutex<String> = Mutex::new(String::new());

/// Ducking meter for the settings: chat level, threshold, voice, echo source.
pub fn duck_meter() -> serde_json::Value {
    serde_json::json!({
        "db": CHAT_DB.load(Ordering::Relaxed),
        "thr": THR_DB.load(Ordering::Relaxed),
        "voice": VOICE.load(Ordering::Relaxed),
        "echo": ECHO.lock().unwrap().clone(),
        "active": DUCK_ACTIVE.load(Ordering::Relaxed),
        "watching": DUCK_WATCHING.load(Ordering::Relaxed),
        "win": super::win_ducking_pref(),
    })
}

fn duck_level(c: &Config) -> f32 {
    if !c.on("duck") {
        return 1.0;
    }
    let l = c.duck_level;
    if l <= 0 || l >= 100 { 1.0 } else { l.max(5) as f32 / 100.0 }
}

/// Pushes into the history of key (allocates only for a new key).
fn hist_push(map: &mut HashMap<String, LvlHist>, key: &str, v: f64) {
    match map.get_mut(key) {
        Some(h) => h.push(v),
        None => {
            let mut h = LvlHist::default();
            h.push(v);
            map.insert(key.to_string(), h);
        }
    }
}

/// Sets the value of key (allocates only for a new key).
fn set_val(map: &mut HashMap<String, f64>, key: &str, v: f64) {
    match map.get_mut(key) {
        Some(x) => *x = v,
        None => {
            map.insert(key.to_string(), v);
        }
    }
}

fn peak_db(m: &IAudioMeterInformation) -> Option<f64> {
    let v = unsafe { m.GetPeakValue() }.ok()?;
    Some(if v > 1e-6 { 20.0 * (v as f64).log10() } else { -120.0 })
}

impl Ducker {
    fn release(&mut self) {
        self.meters.clear();
        self.others.clear();
        self.eps.clear();
        self.games.clear();
    }

    fn set_speaking(&mut self, on: bool, exe: &str, db: f64, floor: f64) {
        if on == self.speaking {
            return;
        }
        self.speaking = on;
        DUCK_ACTIVE.store(on, Ordering::Relaxed);
        if on {
            let other = if !self.loud_name.is_empty() && self.loud_db > -60.0 { format!("{} {} dB", self.loud_name, self.loud_db.round() as i32) } else { String::new() };
            logf!("duck: voice in {exe} ({db:.0} dB, background {floor:.0} dB; loudest other app: {other})");
        } else {
            logf!("duck: chat silent");
        }
        server::notify_subs();
    }

    /// Whether the chat session sound repeats another app's (then it is not a conversation).
    /// While there is too little data about an app that just started sounding — wait too (≈ ¼ s).
    fn echo_source(&self, h: &LvlHist) -> (String, f64) {
        for o in &self.others {
            let oh = self.hist.get(&o.key).copied().unwrap_or_default();
            if oh.n < ECHO_MIN_N && oh.n > 0 && oh.at(0) > DUCK_MIN_DB {
                return (o.exe.clone(), 0.0);
            }
            let r = echo_of(h, &oh, 4);
            if r >= ECHO_CORR {
                return (o.exe.clone(), r);
            }
        }
        (String::new(), 0.0)
    }

    /// The chat session repeats the whole device sound, and that is other apps' sound. While it
    /// matches, the measurement is not talk; after ≈ 0.3 s of matches the session is remembered as
    /// a "mirror": voice in it is then looked for only as sound above other apps.
    fn is_mirror(&mut self, m_key: &str, m_exe: &str, m_dev: &str, db: f64) -> bool {
        if self.mirror.contains(m_key) {
            return true;
        }
        if !mirror_evidence(self.hist.get(m_key), self.ep_hist.get(m_dev), self.post_hist.get(m_dev), db) {
            return false;
        }
        let s = self.mirror_score.entry(m_key.to_string()).or_insert(0);
        *s += 1;
        if *s >= 12 {
            self.mirror.insert(m_key.to_string());
            logf!("duck: {m_exe} has a session that repeats everything playing on the headset — counting only sound above other apps there");
        }
        true
    }
}

impl Engine {
    /// (start of apply) Whether to collect sessions for ducking.
    pub(super) fn duck_begin(&mut self, cfg: &Config, restore: bool) -> bool {
        let d = &mut self.dk;
        d.release();
        d.level = duck_level(cfg);
        let on = !restore && d.level < 1.0;
        d.media = on && cfg.duck_media;
        if !on {
            d.cur = 1.0; // switched off — apply returns the normal volume right away
        }
        if d.floor.len() > 64 {
            d.floor.clear(); // sessions of closed apps
        }
        if d.hist.len() > 64 {
            d.hist.clear();
        }
        if d.mirror.len() > 64 || d.ep_hist.len() > 64 {
            d.mirror.clear();
            d.mirror_score.clear();
            d.ep_hist.clear();
            d.post_hist.clear();
        }
        on
    }

    pub(super) fn duck_collect(&mut self, cat: &str, key: &str, exe: &str, pid: u32, s: &com::Session, meter: bool) {
        let d = &mut self.dk;
        // the level meter comes from IAudioSessionControl (as mixers do, e.g. EarTrumpet): for chat —
        // to hear the conversation, for the rest — to recognise a copy of their sound in chat
        if meter {
            if let Ok(m) = s.ctl.cast::<IAudioMeterInformation>() {
                let dm = DuckMeter { exe: exe.into(), key: key.into(), pid, meter: m, dev: s.dev_id.clone(), sav: None };
                if cat == "chat" {
                    d.meters.push(dm);
                } else {
                    d.others.push(DuckMeter { sav: Some(s.sav.clone()), ..dm });
                }
            }
        }
        if cat == "game" || (cat == "media" && d.media) {
            d.games.push(DuckGame { key: key.into(), sav: s.sav.clone(), game: cat == "game", exe: exe.into() });
        }
    }

    /// (end of apply) Start or stop watching.
    pub(super) fn duck_end(&mut self, game: i32) {
        let d = &mut self.dk;
        d.game_factor = game as f32 / 100.0;
        let active = d.level < 1.0 && !d.meters.is_empty() && !d.games.is_empty();
        DUCK_WATCHING.store(active, Ordering::Relaxed);
        if active && d.next.is_none() {
            d.next = Some(Instant::now() + DUCK_TICK);
        }
        // what is listened to and what gets quieter — into the log (only when it changed)
        let names: Vec<&str> = d.meters.iter().map(|m| m.exe.as_str()).collect();
        let mut ducked: Vec<&str> = d.games.iter().map(|g| g.exe.as_str()).collect();
        ducked.sort_unstable();
        ducked.dedup();
        let sig = format!("{}|{}", names.join(","), ducked.join(","));
        if active && sig != d.logged {
            d.logged = sig;
            logf!("duck: listening to {}; quieter: {}; level {:.0}%", names.join(", "), ducked.join(", "), d.level * 100.0);
        }
        if !active {
            d.next = None;
            d.above = 0;
            d.last_voice = None;
            d.logged.clear();
            d.set_speaking(false, "", 0.0, 0.0);
            VOICE.store(false, Ordering::Relaxed);
            CHAT_DB.store(-120, Ordering::Relaxed);
            if d.cur != 1.0 {
                // chat closed mid-conversation — the game volume comes back
                d.cur = 1.0;
                self.duck_set_volumes();
            }
        }
    }

    pub(super) fn duck_step(&mut self) {
        // the level of every device and the loudest other app on it (with its volume: the session
        // meter shows sound before the volume, and the copy in chat — after it); the tables keep
        // their keys between steps, so a step allocates nothing in the usual case
        let d = &mut self.dk;
        let eps = &d.eps;
        d.post.retain(|k, _| eps.contains_key(k));
        for v in d.ep_now.values_mut() {
            *v = f64::NAN; // NaN — no level from the device meter this step
        }
        for v in d.sum_lin.values_mut() {
            *v = 0.0;
        }
        for (dev, em) in &d.eps {
            if let Some(db) = peak_db(em) {
                hist_push(&mut d.ep_hist, dev, db);
                set_val(&mut d.ep_now, dev, db);
            }
            set_val(&mut d.post, dev, -120.0);
        }
        let (mut od, mut oname) = (-120.0, None::<usize>);
        for (i, o) in d.others.iter().enumerate() {
            let Some(db) = peak_db(&o.meter) else { continue };
            let sav = o.sav.as_ref();
            let mute = sav.is_some_and(|s| unsafe { s.GetMute() }.map(|b| b.as_bool()).unwrap_or(false));
            let vol = sav.map(com::get_vol).unwrap_or(1.0);
            let pdb = if mute { -120.0 } else { db + 20.0 * (vol as f64).max(1e-6).log10() };
            hist_push(&mut d.hist, &o.key, pdb);
            if let Some(v) = d.post.get_mut(&o.dev) {
                if pdb > *v {
                    *v = pdb;
                }
            }
            match d.sum_lin.get_mut(&o.dev) {
                Some(v) => *v += 10f64.powf(pdb / 20.0),
                None => {
                    d.sum_lin.insert(o.dev.clone(), 10f64.powf(pdb / 20.0));
                }
            }
            if pdb > od && o.exe != SELF_EXE {
                (od, oname) = (pdb, Some(i));
            }
        }
        for (dev, v) in &d.post {
            hist_push(&mut d.post_hist, dev, *v);
        }
        d.loud_db = od;
        if let Some(i) = oname {
            if d.loud_name != d.others[i].exe {
                d.loud_name = d.others[i].exe.clone();
            }
        }
        let (mut voice, mut best, mut who, mut best_floor, mut echo) = (false, -120.0f64, None::<usize>, DUCK_FLOOR_MIN, None::<(usize, String)>);
        d.by_pid.clear();
        let meters = std::mem::take(&mut self.dk.meters);
        for (i, m) in meters.iter().enumerate() {
            let pm = self.pm.get(m.pid);
            let d = &mut self.dk;
            let db = if let Some(pm) = pm {
                // the app's own sound — no copies of foreign sound in it
                *d.by_pid.entry(m.pid).or_insert_with(|| pm.take(0))
            } else if let Some(v) = peak_db(&m.meter) {
                v
            } else {
                continue;
            };
            hist_push(&mut d.hist, &m.key, db);
            let mut db = db;
            if pm.is_none() && d.is_mirror(&m.key, &m.exe, &m.dev, db) {
                // the session meter shows all headset sound; the chat voice is only what is above
                // other apps' sound (works when the voice is louder than music or the game)
                db = residual_db(&d.ep_now, &d.sum_lin, &m.dev);
            }
            // background: drops fast to quieter values, rises slowly (≈4 dB/s)
            let f = match d.floor.get(&m.key) {
                None => db,
                Some(&f) if db < f => f + (db - f) * 0.3,
                Some(&f) => f + DUCK_FLOOR_RISE,
            }
            .max(DUCK_FLOOR_MIN);
            set_val(&mut d.floor, &m.key, f);
            if db > best {
                (best, who, best_floor) = (db, Some(i), f);
            }
            if db > DUCK_MIN_DB && db > f + DUCK_OVER_FLOOR {
                let h = d.hist.get(&m.key).copied().unwrap_or_default();
                let (src, r) = d.echo_source(&h);
                if pm.is_none() && !d.mirror.contains(&m.key) && !src.is_empty() {
                    // the chat level moves together with another app's sound — not a conversation
                    let tag = format!("{}{src}", m.exe);
                    if d.echo_log != tag && r > 0.0 {
                        logf!("duck: ignoring {} — its level follows {src} (r={r:.2}), not a conversation", m.exe);
                        d.echo_log = tag;
                    }
                    echo = Some((i, src));
                    continue;
                }
                voice = true;
            }
        }
        let who = who.map(|i| meters[i].exe.clone()).unwrap_or_default();
        let echo = echo.map(|(i, src)| format!("{} ← {}", crate::appinfo::display_name(&meters[i].exe), crate::appinfo::display_name(&src))); // for the window
        self.dk.meters = meters;
        let echo = echo.unwrap_or_default();
        let d = &mut self.dk;
        if !echo.is_empty() {
            d.echo_msg = echo;
            d.echo_at = Some(Instant::now());
        } else if d.echo_at.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
            d.echo_msg.clear(); // the explanation stays in the window a while, so it does not blink
        }
        *ECHO.lock().unwrap() = d.echo_msg.clone();
        CHAT_DB.store(best.round() as i32, Ordering::Relaxed);
        THR_DB.store(DUCK_MIN_DB.max(best_floor + DUCK_OVER_FLOOR).round() as i32, Ordering::Relaxed);
        VOICE.store(voice, Ordering::Relaxed);
        let now = Instant::now();
        d.above = if voice { d.above + 1 } else { 0 };
        if d.above >= 2 {
            // at least 50 ms of sound — single clicks do not count
            d.last_voice = Some(now);
        }
        let talking = d.last_voice.is_some_and(|t| now.duration_since(t) < DUCK_HOLD);
        d.set_speaking(talking, &who, best, best_floor);
        let target = if talking { d.level } else { 1.0 };
        let prev = d.cur;
        if target < d.cur {
            d.cur += (target - d.cur) * attack();
        } else if target > d.cur {
            d.cur += (target - d.cur) * release();
        }
        if (d.cur - target).abs() < 0.003 {
            d.cur = target;
        }
        if d.cur != prev {
            self.duck_set_volumes();
        }
    }

    /// Game app volume = the user's level × ChatMix × ducking.
    fn duck_set_volumes(&mut self) {
        let (cur, gf) = (self.dk.cur, self.dk.game_factor);
        for g in &self.dk.games {
            let f = if g.game { cur * gf } else { cur }; // ChatMix applies only to the game
            let v = com::get_vol(&g.sav);
            let sv = match self.vols.get_mut(&g.key) {
                None => {
                    if f >= 0.999 {
                        continue;
                    }
                    self.vols.entry(g.key.clone()).or_insert(super::SessVol { base: v, last: v, fac: 1.0, seen: true })
                }
                Some(sv) => {
                    if (v - sv.last).abs() > 0.01 && sv.fac > 0.05 {
                        sv.base = (v / sv.fac).min(1.0); // the user changed the volume in the mixer
                    }
                    sv
                }
            };
            let target = sv.base * f;
            if target != v {
                com::set_vol(&g.sav, target);
            }
            sv.last = target;
            sv.fac = f;
            if f >= 0.999 {
                self.vols.remove(&g.key); // the user's level is back — no more tracking
            }
        }
    }

    /// Devices whose sound ducking listens to: the default output (whatever it is: headset,
    /// headphones on an interface, speakers), the headset (Game and Chat) even when it is not the
    /// default, the devices picked for single apps, and routing cables when routing is on. Sessions
    /// on other devices (other virtual ones, a monitor) do not count as talk.
    pub(super) fn listen_endpoints(&mut self, cfg: &Config) -> HashSet<String> {
        let mut set = HashSet::new();
        let mut names = Vec::new();
        let Some(en) = self.en.get().cloned() else { return set };
        for d in com::list_flow(&en, eRender) {
            let ln = d.name.to_lowercase();
            if is_virtual_name(&d.name) || ln.contains("sonar") || !(ln.contains("arctis") || ln.contains("steelseries")) {
                continue;
            }
            set.insert(d.id.clone());
            names.push(d.name.clone());
        }
        let id = com::default_id(&en, eRender);
        if !id.is_empty() && set.insert(id) {
            names.push("default output".into());
        }
        for id in cfg.app_outs.values().filter(|id| !id.is_empty()) {
            if set.insert(id.clone()) {
                names.push("an app's own output".into());
            }
        }
        if cfg.route_live() {
            for id in ["game", "chat", "media"].map(|k| cfg.route_cable(k)) {
                if !id.is_empty() {
                    set.insert(id.to_string());
                }
            }
        }
        let sig = names.join("; ");
        if sig != self.endpoints_log {
            self.endpoints_log = sig.clone();
            logf!("duck: listening on {sig}");
        }
        set
    }

    /// (in apply) Level meters of the devices listened to.
    pub(super) fn duck_endpoints(&mut self, listen: &HashSet<String>) {
        let Some(en) = self.en.get().cloned() else { return };
        for id in listen {
            if let Some(dev) = com::get_device(&en, id) {
                if let Ok(m) = unsafe { dev.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None) } {
                    self.dk.eps.insert(id.clone(), m);
                }
            }
        }
    }
}
