//! Headsets: the native Arctis 7+ driver and headsets described in web/devices.json (devs/), what
//! the window shows about the headset, commands from the window, and battery notifications.

use std::{cell::RefCell, time::Instant};

use serde::Serialize;

use crate::{
    config::{self, Profile},
    devs::hid,
    headset, logf,
};

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct DevSettingOpt {
    pub v: f64,
    pub label: String,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct DevSetting {
    pub api: String,
    pub field: String,
    pub label: String,
    /// seg | toggle | slider
    pub kind: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub opts: Vec<DevSettingOpt>,
    pub min: f64,
    pub max: f64,
    pub value: Option<f64>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct DevEq {
    pub api: String,
    /// graphic | parametric
    pub kind: String,
    #[serde(skip)]
    pub fields: Vec<String>,
    pub freqs: Vec<f64>,
    pub min: f64,
    pub max: f64,
    pub bands: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// built-in headset presets (Nova Pro, GameDAC: "Flat", game presets and so on)
    #[serde(rename = "presetApi", skip_serializing_if = "String::is_empty")]
    pub preset_api: String,
    #[serde(skip)]
    pub preset_field: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<DevSettingOpt>,
    #[serde(rename = "customSlot")]
    pub custom_slot: f64,
    #[serde(skip)]
    pub has_custom: bool,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct DevStateJson {
    #[serde(rename = "hwPreset")]
    pub hw_preset: Option<f64>,
    pub name: String,
    pub native: bool,
    pub settings: Vec<DevSetting>,
    pub eq: Option<DevEq>,
}

/// The ten bands of the headset equalizer (Hz).
pub const STD10: [f64; 10] = [32.0, 64.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

/// Starts looking for the headset (a HID thread of its own).
pub fn start() {
    std::thread::Builder::new().name("hid".into()).spawn(hid::hid_loop).ok();
}

pub fn shutdown() {
    hid::shutdown();
}

pub fn state() -> DevStateJson {
    let hs = headset::snapshot();
    let Some(g) = hid::generic() else {
        // no supported headset: nothing of a headset is shown (users of other headsets do not see it)
        if !hs.connected {
            return DevStateJson::default();
        }
        let eq = DevEq { api: "eq".into(), kind: "graphic".into(), bands: 10, freqs: STD10.to_vec(), min: -12.0, max: 12.0, ..Default::default() };
        return DevStateJson { name: hs.dev_name.clone(), native: true, eq: Some(eq), ..Default::default() };
    };
    let stored = config::get().dev_settings;
    let stored0 = |api: &str, field: &str| stored.get(&g.key(api)).and_then(|m| m.get(field)).copied();
    let mut ds = DevStateJson { name: g.name.clone(), eq: g.eq.clone(), ..Default::default() };
    if let Some(eq) = g.eq.as_ref().filter(|e| !e.preset_field.is_empty()) {
        ds.hw_preset = g.status_value(&eq.preset_field).or_else(|| stored0(&eq.preset_api, &eq.preset_field));
    }
    for s in &g.schema {
        let mut s = s.clone();
        s.value = g.status_value(&s.field).or_else(|| stored0(&s.api, &s.field));
        ds.settings.push(s);
    }
    ds
}

/// ChatMix is mixed in the headset itself (a separate "Chat" device in Windows and no software
/// ChatMix, as with Nova Pro).
pub fn hardware_chatmix() -> bool {
    if !crate::audio::HW_CHATMIX.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    let h = headset::lock();
    !h.generic || !h.generic_soft_chatmix
}

thread_local! {
    /// (UI thread) the last state reacted to: receiver found, headset on, both (-1 — not yet)
    static LAST: std::cell::Cell<(i8, i8, i8)> = const { std::cell::Cell::new((-1, -1, -1)) };
}

/// (UI thread) Receiver found / headset powered changed: output order and the headset mic follow,
/// the header shows "2.4 GHz" / "off" next to the output device.
pub fn on_state(connected: bool, powered: bool) {
    let (c, p, on) = (connected as i8, powered as i8, (connected && powered) as i8);
    let (lc, lp, lon) = LAST.with(|l| l.get());
    LAST.with(|l| l.set((c, p, on)));
    if on != lon {
        crate::audio::headset_power(on == 1);
    } else if (c, p) != (lc, lp) {
        crate::audio::refresh_out();
    }
}

// ---------- commands from the window ----------

/// The headset equalizer curve (dB per band).
pub fn set_eq(bands: &[f64]) {
    match hid::generic() {
        Some(g) => g.set_eq(bands),
        None if bands.len() == 10 => hid::queue_eq(bands),
        None => {}
    }
}

/// A setting of a headset described in devices.json.
pub fn set_setting(api: &str, field: &str, value: f64) {
    let Some(g) = hid::generic() else { return };
    let (api, field) = (api.to_string(), field.to_string());
    g.post(move |s| {
        s.set_setting(&api, &field, value, true);
        crate::server::notify_subs();
    });
}

/// A built-in equalizer preset of the headset (Nova Pro, GameDAC).
pub fn set_hw_preset(v: f64) {
    let Some(g) = hid::generic() else { return };
    g.post(move |s| {
        s.set_hw_preset(v);
        s.g.schedule_save();
        crate::server::notify_subs();
    });
}

/// A one-argument command to the Arctis 7+ (sidetone, auto-off, mic gain, mute light).
pub fn send_cmd(cmd: u8, arg: u8) -> bool {
    hid::send_device_cmd(cmd, arg)
}

/// Brightness of the muted-mic light (0 off, 1 low, 2 medium, 3 high): Arctis 7+ takes 0..3 as is.
pub fn mute_led_value(level: i32) -> u8 {
    level.clamp(0, 3) as u8
}

/// Sends a profile's equalizer and sidetone to the headset.
pub fn apply_profile(p: &Profile) {
    let g = hid::generic();
    if !p.eq.is_empty() {
        match &g {
            Some(g) => {
                if g.eq.as_ref().is_some_and(|e| e.bands == p.eq.len()) {
                    g.set_eq(&p.eq);
                }
            }
            None if p.eq.len() == 10 => hid::queue_eq(&p.eq),
            None => {}
        }
    }
    if (0..=3).contains(&p.sidetone) && g.is_none() {
        let v = p.sidetone as u8;
        std::thread::spawn(move || send_cmd(0x39, v));
    }
}

// ---------- battery ----------

#[derive(Default)]
struct Batt {
    last: i32,
    /// the first level drop of this discharge — the reference point
    drop_at: Option<Instant>,
    drop_pct: i32,
    /// runtime estimate, hours (0 — unknown)
    hours: f64,
    low_sent: bool,
    full_sent: bool,
    was_charge: bool,
}

thread_local! {
    static BATT: RefCell<Batt> = RefCell::new(Batt::default());
}

static HOURS: std::sync::Mutex<f64> = std::sync::Mutex::new(0.0);

pub fn batt_hours() -> f64 {
    *HOURS.lock().unwrap()
}

/// (UI thread) Notifications and the runtime estimate.
pub fn on_battery(lvl: i32, charging: bool) {
    let c = config::get();
    let now = Instant::now();
    // what to tell: "low" or "charged"
    let mut note: Option<&str> = None;
    BATT.with(|b| {
        let b = &mut *b.borrow_mut();
        if charging {
            (b.drop_at, b.hours, b.low_sent) = (None, 0.0, false);
            if lvl >= 100 && !b.full_sent && c.full_notify && b.was_charge {
                b.full_sent = true;
                note = Some("charged");
            }
            b.was_charge = true;
        } else {
            (b.full_sent, b.was_charge) = (false, false);
            if b.last > 0 && lvl < b.last && b.drop_at.is_none() {
                (b.drop_at, b.drop_pct) = (Some(now), lvl);
            } else if let Some(at) = b.drop_at.filter(|_| lvl < b.drop_pct) {
                let h = now.duration_since(at).as_secs_f64() / 3600.0;
                if h > 0.05 {
                    let rate = (b.drop_pct - lvl) as f64 / h; // % per hour
                    b.hours = (lvl as f64 / rate * 2.0).round() / 2.0;
                }
            }
            let thr = if c.low_level <= 0 { 20 } else { c.low_level };
            if lvl <= thr && !b.low_sent && c.low_battery {
                b.low_sent = true;
                note = Some("low");
            } else if lvl > thr {
                b.low_sent = false;
            }
        }
        b.last = lvl;
        *HOURS.lock().unwrap() = b.hours;
    });
    if let Some(n) = note {
        logf!("battery notification: {n}, {lvl}%");
        // the on-screen hint, like the balance and the output device (no system notification: easy to miss
        // in a game, and it stays in the action center)
        let name = headset::lock().dev_name.clone();
        let nova = name.to_lowercase().contains("nova");
        crate::osd::show_battery(&name, lvl, nova);
    }
}
