//! The state the window page renders (/api/state and /api/events).

use std::collections::BTreeMap;

use serde::Serialize;

use crate::{
    APP_VER, audio, config,
    config::{EqUserPreset, Hotkey, MicFxCfg, RouteCfg, SoftEq},
    devices, headset, hotkeys, router, vst, wintheme,
};

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct AppJson {
    pub exe: String,
    pub name: String,
    pub cat: String,
    pub live: bool,
    /// process runs, but has no sound right now
    pub running: bool,
    pub playing: bool,
    pub removable: bool,
    /// audible right now (not just an open stream)
    pub sound: bool,
    /// app volume 0..100 (-1 — unknown: an app without sound)
    pub vol: i32,
    /// its own output device ("" — as in Windows)
    #[serde(skip_serializing_if = "String::is_empty")]
    pub out: String,
}

#[derive(Serialize)]
pub struct CfgJson {
    pub theme: String,
    pub lang: String,
    pub mode: String,
    #[serde(rename = "lowBattery")]
    pub low_battery: bool,
    #[serde(rename = "wheelMode")]
    pub wheel_mode: String,
    pub sidetone: i32,
    #[serde(rename = "autoOff")]
    pub auto_off: i32,
    pub autostart: bool,
    #[serde(rename = "micGain")]
    pub mic_gain: i32,
    #[serde(rename = "muteLed")]
    pub mute_led: i32,
    #[serde(rename = "customA")]
    pub custom_a: String,
    #[serde(rename = "customB")]
    pub custom_b: String,
    #[serde(rename = "customOled")]
    pub custom_oled: bool,
    pub route: RouteCfg,
    #[serde(rename = "micFx")]
    pub mic_fx: MicFxCfg,
    #[serde(rename = "softEq")]
    pub soft_eq: BTreeMap<String, SoftEq>,
    #[serde(rename = "micDev")]
    pub mic_dev: String,
    #[serde(rename = "autoSwitch")]
    pub auto_switch: bool,
    #[serde(rename = "switchMic")]
    pub switch_mic: bool,
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
    pub profiles: Vec<String>,
    pub hotkeys: BTreeMap<String, Hotkey>,
    #[serde(rename = "eqUser")]
    pub eq_user: Vec<EqUserPreset>,
    pub onboarded: bool,
    #[serde(rename = "keepOpen")]
    pub keep_open: bool,
    pub taskbar: bool,
    pub features: BTreeMap<String, bool>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct DuckJson {
    /// there is a chat app and a game app
    pub watching: bool,
    /// someone talks in chat — the game is ducked
    pub active: bool,
}

#[derive(Serialize)]
pub struct StateJson {
    pub connected: bool,
    pub powered: bool,
    #[serde(rename = "haveBattery")]
    pub have_batt: bool,
    pub battery: i32,
    pub charging: bool,
    pub game: i32,
    pub chat: i32,
    pub pos: i32,
    pub cfg: CfgJson,
    pub apps: Vec<AppJson>,
    pub mic: audio::MicInfo,
    pub eq: Vec<f64>,
    #[serde(rename = "eqPreset")]
    pub eq_preset: String,
    pub device: devices::DevStateJson,
    #[serde(rename = "micMuted")]
    pub mic_muted: Option<bool>,
    pub batteries: Option<BTreeMap<String, i32>>,
    pub version: &'static str,
    pub route: router::PipeStatus,
    #[serde(rename = "micFx")]
    pub mic_fx: router::PipeStatus,
    /// Windows lets apps be routed automatically
    #[serde(rename = "appRoute")]
    pub app_route: bool,
    pub vst: Vec<vst::VstInfo>,
    #[serde(rename = "battHours")]
    pub batt_hours: f64,
    pub duck: DuckJson,
    pub monitor: router::MonState,
    #[serde(rename = "hotkeyErr")]
    pub hotkey_err: Vec<String>,
    #[serde(rename = "winCall")]
    pub win_call: audio::WinCallJson,
    pub out: audio::OutInfo,
    /// output devices by priority
    pub outs: Vec<audio::OutDevJson>,
    /// Windows look (for the "As in Windows" theme)
    pub win: wintheme::WinThemeInfo,
    pub cable: audio::CableJson,
}

pub fn build_apps() -> Vec<AppJson> {
    let c = config::get();
    let mut out: Vec<AppJson> = Vec::new();
    for s in audio::sessions() {
        let k = s.exe.to_lowercase();
        out.push(AppJson {
            exe: s.exe.clone(),
            name: crate::appinfo::display_name(&s.exe),
            cat: c.category(&k).into(),
            live: true,
            playing: s.playing,
            sound: s.playing && audio::audible(&k),
            vol: (s.vol * 100.0 + 0.5) as i32,
            out: c.app_outs.get(&k).cloned().unwrap_or_default(),
            ..Default::default()
        });
    }
    for k in &c.pinned {
        if k.is_empty() || out.iter().any(|a| a.exe.to_lowercase() == *k) {
            continue;
        }
        out.push(AppJson {
            exe: k.clone(),
            name: crate::appinfo::display_name(k),
            cat: c.category(k).into(),
            removable: true,
            running: audio::is_running(k),
            // no sound yet: the level is remembered and set once it plays (volume memory)
            vol: if c.on("volmem") { c.app_vols.get(k).map_or(100, |v| (v * 100.0).round() as i32) } else { -1 },
            out: c.app_outs.get(k).cloned().unwrap_or_default(),
            ..Default::default()
        });
    }
    out.sort_by(|a, b| {
        b.live
            .cmp(&a.live)
            .then(b.playing.cmp(&a.playing))
            .then(b.running.cmp(&a.running))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

pub fn build() -> StateJson {
    let c = config::get();
    let hs = headset::snapshot();
    let p = crate::POS.load(std::sync::atomic::Ordering::Relaxed);
    let mut hk = BTreeMap::new();
    for k in hotkeys::DEFAULTS.iter().map(|d| d.0) {
        hk.insert(k.to_string(), hotkeys::hotkey_of(&c, k));
    }
    let features = config::FEATURE_KEYS.iter().map(|k| (k.to_string(), c.on(k))).collect();
    let (route, mic_fx) = router::status(&c);
    let device = devices::state();
    let nb = device.eq.as_ref().map(|e| e.bands).unwrap_or(10);
    let (mut eq, mut eq_preset) = (c.eq.clone(), c.eq_preset.clone());
    if eq.len() != nb {
        (eq, eq_preset) = (vec![0.0; nb], "flat".into());
    }
    StateJson {
        connected: hs.connected,
        powered: hs.powered,
        have_batt: hs.have_batt,
        battery: hs.batt_pct,
        charging: hs.charging,
        game: 100 - p.max(0),
        chat: 100 - (-p).max(0),
        pos: p,
        cfg: CfgJson {
            theme: c.theme.clone(),
            lang: c.lang.clone(),
            mode: c.mode.clone(),
            low_battery: c.low_battery,
            wheel_mode: c.wheel_mode.clone(),
            sidetone: c.sidetone,
            auto_off: c.auto_off,
            autostart: crate::autostart::cached(),
            mic_gain: c.mic_gain,
            mute_led: c.mute_led,
            custom_a: c.custom_a.clone(),
            custom_oled: c.custom_oled,
            custom_b: c.custom_b.clone(),
            route: c.route.clone(),
            mic_fx: c.mic_fx.clone(),
            soft_eq: c.soft_eq.clone(),
            mic_dev: c.mic_dev.clone(),
            auto_switch: c.auto_switch,
            switch_mic: c.switch_mic,
            low_level: c.low_level,
            full_notify: c.full_notify,
            osd: c.osd,
            mic_hotkey: c.mic_hotkey,
            ptt: c.ptt,
            ptt_key: c.ptt_key,
            sounds: c.sounds,
            duck_level: c.duck_level,
            duck_media: c.duck_media,
            profiles: c.profiles.iter().map(|p| p.name.clone()).collect(),
            hotkeys: hk,
            eq_user: c.eq_user.clone(),
            onboarded: c.onboarded,
            keep_open: c.keep_open,
            taskbar: c.taskbar,
            features,
        },
        apps: build_apps(),
        mic: audio::mic_state(),
        eq,
        eq_preset,
        mic_muted: (hs.mic_muted >= 0 && hs.generic).then_some(hs.mic_muted == 1),
        batteries: (!hs.extra_batt.is_empty()).then(|| hs.extra_batt.clone()),
        device,
        version: APP_VER,
        route,
        mic_fx,
        app_route: audio::app_route_ok(),
        vst: vst::status(),
        batt_hours: devices::batt_hours(),
        duck: audio::duck_state(),
        monitor: router::monitor_status(),
        hotkey_err: hotkeys::fails(),
        win_call: audio::win_call_state(),
        out: audio::out_state(),
        outs: audio::outs_state(),
        win: wintheme::get(),
        cable: audio::cable_state(),
    }
}
