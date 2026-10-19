//! config.json (the same file and fields as the earlier Go builds, so their settings carry over).
//! Unknown fields are kept as they are (`extra`); nulls written by the Go builds read as empty values.

use std::{
    collections::BTreeMap,
    fs,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::{logf, util};

fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub theme: String,
    pub lang: String,
    #[serde(deserialize_with = "null_default")]
    pub chat: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub media: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub game: Vec<String>,
    #[serde(rename = "lowBattery")]
    pub low_battery: bool,
    #[serde(rename = "batteryHoldSec")]
    pub hold_sec: i32,
    pub mode: String,
    pub sidetone: i32,
    #[serde(rename = "autoOff")]
    pub auto_off: i32,
    #[serde(rename = "customA")]
    pub custom_a: String,
    #[serde(rename = "customB")]
    pub custom_b: String,
    /// the custom theme in dark mode on a pure black background (OLED)
    #[serde(rename = "customOled")]
    pub custom_oled: bool,
    #[serde(deserialize_with = "null_default")]
    pub pinned: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub eq: Vec<f64>,
    #[serde(rename = "eqPreset")]
    pub eq_preset: String,
    #[serde(rename = "eqCustom", deserialize_with = "null_default")]
    pub eq_custom: Vec<f64>,
    #[serde(rename = "micGain")]
    pub mic_gain: i32,
    #[serde(rename = "muteLed")]
    pub mute_led: i32,
    #[serde(rename = "micDev")]
    pub mic_dev: String,
    #[serde(rename = "wheelMode")]
    pub wheel_mode: String,
    #[serde(rename = "devSettings", deserialize_with = "null_default", skip_serializing_if = "BTreeMap::is_empty")]
    pub dev_settings: BTreeMap<String, BTreeMap<String, f64>>,

    pub route: RouteCfg,
    #[serde(rename = "micFx")]
    pub mic_fx: MicFxCfg,
    #[serde(rename = "softEq", deserialize_with = "null_default", skip_serializing_if = "BTreeMap::is_empty")]
    pub soft_eq: BTreeMap<String, SoftEq>,
    #[serde(rename = "routedApps", deserialize_with = "null_default", skip_serializing_if = "Vec::is_empty")]
    pub routed_apps: Vec<String>,
    /// An app's own output device (exe → device id), over the cable of its category.
    #[serde(rename = "appOuts", deserialize_with = "null_default", skip_serializing_if = "BTreeMap::is_empty")]
    pub app_outs: BTreeMap<String, String>,

    #[serde(rename = "autoSwitch")]
    pub auto_switch: bool,
    #[serde(rename = "outDevs", deserialize_with = "null_default", skip_serializing_if = "Vec::is_empty")]
    pub out_devs: Vec<OutDev>,
    #[serde(rename = "switchMic")]
    pub switch_mic: bool,
    #[serde(rename = "prevMic", skip_serializing_if = "String::is_empty")]
    pub prev_mic: String,
    #[serde(rename = "prevOut", skip_serializing_if = "String::is_empty")]
    pub prev_out: String,
    #[serde(rename = "lowLevel")]
    pub low_level: i32,
    #[serde(rename = "fullNotify")]
    pub full_notify: bool,
    pub osd: bool,
    #[serde(rename = "micHotkey")]
    pub mic_hotkey: bool,
    pub ptt: bool,
    #[serde(rename = "pttKey")]
    pub ptt_key: i32,
    pub sounds: bool,
    #[serde(rename = "duckLevel")]
    pub duck_level: i32,
    #[serde(rename = "duckMedia")]
    pub duck_media: bool,

    #[serde(deserialize_with = "null_default", skip_serializing_if = "BTreeMap::is_empty")]
    pub hotkeys: BTreeMap<String, Hotkey>,
    #[serde(rename = "eqUser", deserialize_with = "null_default", skip_serializing_if = "Vec::is_empty")]
    pub eq_user: Vec<EqUserPreset>,
    pub onboarded: bool,
    #[serde(rename = "keepOpen")]
    pub keep_open: bool,
    pub taskbar: bool,
    #[serde(rename = "devNames", deserialize_with = "null_default", skip_serializing_if = "BTreeMap::is_empty")]
    pub dev_names: BTreeMap<String, String>,
    #[serde(deserialize_with = "null_default", skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<Profile>,

    /// Volume memory: Windows or other apps sometimes reset volumes — the saved level comes back.
    #[serde(rename = "appVols", deserialize_with = "null_default", skip_serializing_if = "BTreeMap::is_empty")]
    pub app_vols: BTreeMap<String, f64>,
    #[serde(rename = "micLevels", deserialize_with = "null_default", skip_serializing_if = "BTreeMap::is_empty")]
    pub mic_levels: BTreeMap<String, i32>,

    /// "Features": false — the feature is off (neither shown nor working).
    #[serde(deserialize_with = "null_default", skip_serializing_if = "BTreeMap::is_empty")]
    pub features: BTreeMap<String, bool>,

    /// Fields this version does not know (written by another version) — kept untouched.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Hotkey {
    /// MOD_ALT 1, MOD_CONTROL 2, MOD_SHIFT 4, MOD_WIN 8
    #[serde(rename = "mod")]
    pub modifiers: u32,
    pub vk: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EqUserPreset {
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub bands: Vec<f64>,
    #[serde(deserialize_with = "null_default")]
    pub freqs: Vec<f64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icon: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub eq: Vec<f64>,
    #[serde(rename = "eqPreset")]
    pub eq_preset: String,
    #[serde(rename = "softEq", deserialize_with = "null_default", skip_serializing_if = "BTreeMap::is_empty")]
    pub soft_eq: BTreeMap<String, SoftEq>,
    pub sidetone: i32,
    pub gate: f64,
    pub ceil: f64,
    pub denoise: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RouteCfg {
    pub on: bool,
    /// virtual cable for "Game" ("" — not routed)
    pub game: String,
    pub chat: String,
    pub media: String,
    /// headphones; "" — Windows' default output
    pub out: String,
    /// route apps by category automatically
    pub auto: bool,
}

impl Default for RouteCfg {
    fn default() -> Self {
        RouteCfg { on: false, game: String::new(), chat: String::new(), media: String::new(), out: String::new(), auto: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MicFxCfg {
    pub on: bool,
    /// obsolete (before 1.8.1): the microphone is MicDev now
    #[serde(rename = "in")]
    pub input: String,
    /// virtual cable the processed sound goes to
    pub out: String,
    /// noise gate threshold, dB; <= -100 — off
    pub gate: f64,
    /// limiter ceiling, dB; >= 0 — off
    pub ceil: f64,
    pub denoise: bool,
    #[serde(deserialize_with = "null_default", skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<VstSlot>,
}

impl Default for MicFxCfg {
    fn default() -> Self {
        MicFxCfg { on: false, input: String::new(), out: String::new(), gate: -50.0, ceil: -6.0, denoise: false, plugins: Vec::new() }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SoftEq {
    #[serde(deserialize_with = "null_default")]
    pub bands: Vec<f64>,
    pub preset: String,
    #[serde(deserialize_with = "null_default")]
    pub custom: Vec<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OutDev {
    pub id: String,
    /// last known name — to show a disconnected device too
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VstSlot {
    pub id: String,
    pub path: String,
    pub on: bool,
    /// saved plugin state (base64)
    #[serde(skip_serializing_if = "String::is_empty")]
    pub state: String,
}

/// Features that can be switched off in the settings.
pub const FEATURE_KEYS: [&str; 8] = ["eq", "profiles", "volmem", "vst", "denoise", "duck", "monitor", "route"];

impl Default for Config {
    fn default() -> Self {
        let s = |l: &[&str]| l.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        Config {
            theme: "win".into(),
            lang: String::new(),
            chat: s(&["discord.exe", "telegram.exe", "teams.exe", "zoom.exe", "skype.exe", "viber.exe", "steam.exe"]),
            media: s(&["spotify.exe", "vlc.exe", "chrome.exe", "msedge.exe", "firefox.exe"]),
            game: Vec::new(),
            low_battery: true,
            hold_sec: 3,
            mode: "dark".into(),
            sidetone: -1,
            auto_off: -1,
            custom_a: "#FF6A3D".into(),
            custom_b: "#3DC8FF".into(),
            custom_oled: false,
            pinned: Vec::new(),
            eq: Vec::new(),
            eq_preset: String::new(),
            eq_custom: Vec::new(),
            mic_gain: -1,
            mute_led: -1,
            mic_dev: String::new(),
            wheel_mode: "battery".into(),
            dev_settings: BTreeMap::new(),
            route: RouteCfg::default(),
            mic_fx: MicFxCfg::default(),
            soft_eq: BTreeMap::new(),
            routed_apps: Vec::new(),
            app_outs: BTreeMap::new(),
            auto_switch: false,
            out_devs: Vec::new(),
            switch_mic: true,
            prev_mic: String::new(),
            prev_out: String::new(),
            low_level: 20,
            full_notify: true,
            osd: true,
            mic_hotkey: true,
            ptt: false,
            ptt_key: 0,
            sounds: true,
            duck_level: 100,
            duck_media: false,
            hotkeys: BTreeMap::new(),
            eq_user: Vec::new(),
            onboarded: false,
            keep_open: false,
            taskbar: false,
            dev_names: BTreeMap::new(),
            profiles: Vec::new(),
            app_vols: BTreeMap::new(),
            mic_levels: BTreeMap::new(),
            features: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Whether a feature is on (not listed — on).
    pub fn on(&self, f: &str) -> bool {
        self.features.get(f).copied().unwrap_or(true)
    }

    /// The cable of a routing category: "" when none is chosen or it carries the processed
    /// microphone — the microphone wins, so game sound never reaches the people in a call.
    pub fn route_cable(&self, cat: &str) -> &str {
        let id = match cat {
            "game" => &self.route.game,
            "chat" => &self.route.chat,
            "media" => &self.route.media,
            _ => return "",
        };
        if self.mic_fx.on && !self.mic_fx.out.is_empty() && *id == self.mic_fx.out { "" } else { id }
    }

    /// "game" | "chat" | "media"
    pub fn category(&self, exe: &str) -> &'static str {
        let e = exe.to_lowercase();
        if self.media.iter().any(|x| *x == e) {
            "media"
        } else if self.chat.iter().any(|x| *x == e) {
            "chat"
        } else {
            "game"
        }
    }
}

// ---------- load and save ----------

static CFG: Mutex<Option<Config>> = Mutex::new(None);

fn lock() -> MutexGuard<'static, Option<Config>> {
    CFG.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn get() -> Config {
    lock().clone().unwrap_or_default()
}

/// Reads the settings without copying them (for frequent checks).
pub fn with<R>(f: impl FnOnce(&Config) -> R) -> R {
    let g = lock();
    match g.as_ref() {
        Some(c) => f(c),
        None => f(&Config::default()),
    }
}

pub fn replace(c: Config) {
    *lock() = Some(c);
}

/// Changes the settings and writes them right away.
pub fn set(f: impl FnOnce(&mut Config)) {
    let c = {
        let mut g = lock();
        let c = g.get_or_insert_with(Config::default);
        f(c);
        c.clone()
    };
    save(&c);
}

static SAVE_LATER: Mutex<bool> = Mutex::new(false);

/// Changes the settings now, writes them a bit later: a dragged slider gives one write.
pub fn set_later(f: impl FnOnce(&mut Config)) {
    {
        let mut g = lock();
        f(g.get_or_insert_with(Config::default));
    }
    let mut pending = SAVE_LATER.lock().unwrap();
    if !*pending {
        *pending = true;
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(1500));
            flush();
        });
    }
}

/// Writes postponed changes (also before exit).
pub fn flush() {
    let pending = std::mem::replace(&mut *SAVE_LATER.lock().unwrap(), false);
    if pending {
        save(&get());
    }
}

pub fn load() -> Config {
    let path = util::config_path();
    let Ok(b) = fs::read(&path) else {
        let c = Config::default();
        save(&c);
        return c;
    };
    let parsed: Result<Value, _> = serde_json::from_slice(&b);
    let c = parsed.and_then(|v| {
        // settings exist — no onboarding needed (the field appeared in 1.8)
        let had_onboarded = v.get("onboarded").is_some();
        let mut c: Config = serde_json::from_value(v)?;
        if !had_onboarded {
            c.onboarded = true;
        }
        Ok(c)
    });
    let mut c = match c {
        Ok(c) => c,
        Err(e) => {
            // keep the damaged file next to it so the settings can be restored by hand
            let _ = fs::write(path.with_extension("json.bad"), &b);
            logf!("config.json is corrupted ({e}), using defaults; copy saved as config.json.bad");
            return Config::default();
        }
    };
    migrate(&mut c);
    for l in [&mut c.chat, &mut c.media, &mut c.game] {
        for x in l.iter_mut() {
            *x = x.trim().to_lowercase();
        }
    }
    c
}

/// Settings from older versions.
pub fn migrate(c: &mut Config) {
    // early builds offered a 25 % low-battery alert; the headset LED turns red below 20 %
    if c.low_level == 25 {
        c.low_level = 20;
    }
    // before 1.8.1 microphone processing had its own microphone — now one for the whole app
    if !c.mic_fx.input.is_empty() {
        if c.mic_dev.is_empty() {
            c.mic_dev = c.mic_fx.input.clone();
        }
        c.mic_fx.input.clear();
    }
    // 1.8.9: the violet theme is gone — pink took its place
    if c.theme == "violet" {
        c.theme = "pink".into();
    }
    // early builds: the tray icon had its own colour and three battery morphs — both are gone
    c.extra.remove("iconTheme");
    c.extra.remove("morph");
    // early builds: a helper is handled as its app (Steam Client WebHelper as Steam); the app's own category
    // wins, the helper's pin and saved volume pass to the app
    let is = |x: &String, n: &str| x.eq_ignore_ascii_case(n);
    for (helper, app) in crate::appinfo::ALIASES {
        let app_cat = [&c.chat, &c.media, &c.game].iter().any(|l| l.iter().any(|x| is(x, app)));
        for l in [&mut c.chat, &mut c.media, &mut c.game] {
            if app_cat {
                l.retain(|x| !is(x, helper));
            } else {
                l.iter_mut().filter(|x| x.eq_ignore_ascii_case(helper)).for_each(|x| *x = app.to_string());
            }
        }
        if c.pinned.iter().any(|x| is(x, helper)) {
            c.pinned.retain(|x| !is(x, helper));
            if !c.pinned.iter().any(|x| is(x, app)) {
                c.pinned.push(app.to_string());
            }
        }
        if let Some(v) = c.app_vols.remove(helper) {
            c.app_vols.entry(app.to_string()).or_insert(v);
        }
    }
}

/// Early builds: there is no separate "Custom" equalizer slot any more — the curve kept in it becomes one of
/// the user's presets (named in the app language), once; a curve that is a preset already is dropped.
pub fn move_custom_eq(name: &str) {
    let c = get();
    if c.eq_custom.is_empty() {
        return;
    }
    let keep = c.eq_custom.iter().any(|v| *v != 0.0) && c.eq_custom.len() == 10 && !c.eq_user.iter().any(|p| p.bands == c.eq_custom);
    set(|c| {
        let bands = std::mem::take(&mut c.eq_custom);
        if !keep {
            return;
        }
        let mut n = name.to_string();
        let mut i = 2;
        while c.eq_user.iter().any(|p| p.name.to_lowercase() == n.to_lowercase()) {
            n = format!("{name} {i}");
            i += 1;
        }
        if c.eq_preset == "custom" && c.eq == bands {
            c.eq_preset = format!("u:{n}");
        }
        c.eq_user.push(EqUserPreset { name: n, bands, freqs: crate::devices::STD10.to_vec(), icon: String::new() });
    });
}

static SAVE_MU: Mutex<()> = Mutex::new(());

/// Atomic write: a temporary file first, then the swap — a crash mid-write does not spoil config.json.
pub fn save(c: &Config) {
    let b = match serde_json::to_vec_pretty(c) {
        Ok(b) => b,
        Err(e) => {
            logf!("failed to encode config: {e}");
            return;
        }
    };
    let _g = SAVE_MU.lock().unwrap();
    let path = util::config_path();
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = fs::write(&tmp, &b) {
        logf!("failed to save config.json: {e}");
        return;
    }
    if let Err(e) = fs::rename(&tmp, &path) {
        logf!("failed to save config.json: {e}");
    }
}

// ---------- themes ----------

/// Game and chat colours (RGB). Must match the THEMES table in web/index.html.
/// "win" (as in Windows) and "black" (OLED) take their colours from Windows (wintheme::colors).
pub const PALETTES: [(&str, [u8; 3], [u8; 3]); 10] = [
    ("pink", [0xFF, 0x8D, 0xB4], [0x8D, 0xB6, 0xFF]),
    ("blue", [0x6C, 0xB8, 0xFF], [0x4F, 0xE3, 0xD9]),
    ("gray", [0xD3, 0xD6, 0xDD], [0x8F, 0xB2, 0xE3]),
    ("black", [0x26, 0x8C, 0xE6], [0xB3, 0x9B, 0xFF]),
    ("spotify", [0x1E, 0xD7, 0x60], [0xB3, 0xB3, 0xB3]),
    ("discord", [0x58, 0x65, 0xF2], [0x45, 0xA3, 0x66]),
    ("steam", [0x66, 0xC0, 0xF4], [0xA4, 0xD0, 0x07]),
    ("twitch", [0xBF, 0x94, 0xFF], [0xFF, 0x75, 0xE6]),
    ("youtube", [0xFF, 0x00, 0x33], [0x3E, 0xA6, 0xFF]),
    ("telegram", [0x2A, 0xAB, 0xEE], [0x7B, 0xC8, 0x62]),
];


pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 3];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

pub fn valid_theme(k: &str) -> bool {
    k == "custom" || k == "win" || PALETTES.iter().any(|p| p.0 == k)
}

/// Game and chat colours of a theme (custom_a / custom_b of c for the custom one). The OLED theme takes
/// the Windows accent, as "win" does, on its black background.
pub fn colors_of(theme: &str, c: &Config) -> ([u8; 3], [u8; 3]) {
    if theme == "win" || theme == "black" {
        return crate::wintheme::colors();
    }
    if theme == "custom" {
        if let (Some(a), Some(b)) = (parse_hex(&c.custom_a), parse_hex(&c.custom_b)) {
            return (a, b);
        }
    }
    let p = PALETTES.iter().find(|p| p.0 == theme).unwrap_or(&PALETTES[0]);
    (p.1, p.2)
}

#[allow(dead_code)]
pub fn skip_false(b: &bool) -> bool {
    is_false(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_go_config_with_nulls_and_unknown_fields() {
        let src = r#"{"theme":"pink","chat":null,"eq":null,"game":null,"route":{"on":true},
            "micFx":{"on":false,"in":"","out":"x","gate":-40,"ceil":-6,"denoise":true},
            "futureField":{"a":1},"micLevels":{"dev":80}}"#;
        let v: Value = serde_json::from_str(src).unwrap();
        let c: Config = serde_json::from_value(v).unwrap();
        assert_eq!(c.theme, "pink");
        assert!(c.chat.is_empty() && c.eq.is_empty());
        assert!(c.route.on && c.route.auto, "missing nested fields keep defaults");
        assert_eq!(c.mic_fx.gate, -40.0);
        assert_eq!(c.mic_levels.get("dev"), Some(&80));
        assert!(c.extra.contains_key("futureField"));
        let out = serde_json::to_string(&c).unwrap();
        assert!(out.contains("\"futureField\":{\"a\":1}"), "unknown fields survive a save");
        assert!(!out.contains("devSettings"), "empty omitempty maps are not written");
    }

    #[test]
    fn helper_settings_pass_to_its_app() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // the user's case: both in chat, the helper pinned with a saved volume
        let mut c = Config { chat: s(&["steam.exe", "discord.exe", "steamwebhelper.exe"]), pinned: s(&["discord.exe", "steamwebhelper.exe"]), ..Default::default() };
        c.app_vols.insert("steamwebhelper.exe".into(), 0.66);
        migrate(&mut c);
        assert_eq!(c.chat, s(&["steam.exe", "discord.exe"]));
        assert_eq!(c.pinned, s(&["discord.exe", "steam.exe"]));
        assert_eq!(c.app_vols.get("steam.exe"), Some(&0.66));
        assert!(!c.app_vols.contains_key("steamwebhelper.exe"));
        // the app has no category of its own: the helper's one passes to it
        let mut c = Config { chat: s(&[]), media: s(&["steamwebhelper.exe"]), ..Default::default() };
        c.app_vols.insert("steam.exe".into(), 0.5);
        c.app_vols.insert("steamwebhelper.exe".into(), 0.66);
        migrate(&mut c);
        assert_eq!(c.category("steam.exe"), "media");
        assert_eq!(c.app_vols.get("steam.exe"), Some(&0.5), "the app's own volume wins");
    }

    #[test]
    fn dropped_fields_leave_the_file() {
        let v: Value = serde_json::from_str(r#"{"theme":"steam","iconTheme":"twitch","morph":"drop","appOuts":{"spotify.exe":"r2"}}"#).unwrap();
        let mut c: Config = serde_json::from_value(v).unwrap();
        migrate(&mut c);
        assert!(!c.extra.contains_key("iconTheme") && !c.extra.contains_key("morph"));
        assert_eq!(c.app_outs.get("spotify.exe").map(String::as_str), Some("r2"));
    }

    #[test]
    fn the_microphone_cable_is_not_routed() {
        let mut c = Config::default();
        (c.route.game, c.route.chat) = ("cable".into(), "other".into());
        (c.mic_fx.on, c.mic_fx.out) = (true, "cable".into());
        assert_eq!((c.route_cable("game"), c.route_cable("chat"), c.route_cable("media")), ("", "other", ""));
        c.mic_fx.on = false;
        assert_eq!(c.route_cable("game"), "cable");
    }

    #[test]
    fn hex() {
        assert_eq!(parse_hex("#FF8DB4"), Some([0xFF, 0x8D, 0xB4]));
        assert_eq!(parse_hex("zz0000"), None);
    }
}
