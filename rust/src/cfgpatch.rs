//! Settings changed from the window (/api/cfg): every value is checked before it is stored.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{
    autostart,
    config::{self, FEATURE_KEYS, Hotkey},
    hotkeys, i18n, util,
};

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CfgPatch {
    theme: Option<String>,
    lang: Option<String>,
    mode: Option<String>,
    #[serde(rename = "lowBattery")]
    low_battery: Option<bool>,
    #[serde(rename = "wheelMode")]
    wheel_mode: Option<String>,
    sidetone: Option<i32>,
    #[serde(rename = "autoOff")]
    auto_off: Option<i32>,
    autostart: Option<bool>,
    #[serde(rename = "micGain")]
    mic_gain: Option<i32>,
    #[serde(rename = "muteLed")]
    mute_led: Option<i32>,
    #[serde(rename = "customA")]
    custom_a: Option<String>,
    #[serde(rename = "customB")]
    custom_b: Option<String>,
    #[serde(rename = "customOled")]
    custom_oled: Option<bool>,

    #[serde(rename = "autoSwitch")]
    auto_switch: Option<bool>,
    #[serde(rename = "switchMic")]
    switch_mic: Option<bool>,
    #[serde(rename = "lowLevel")]
    low_level: Option<i32>,
    #[serde(rename = "fullNotify")]
    full_notify: Option<bool>,
    osd: Option<bool>,
    #[serde(rename = "micHotkey")]
    mic_hotkey: Option<bool>,
    ptt: Option<bool>,
    #[serde(rename = "pttKey")]
    ptt_key: Option<i32>,
    sounds: Option<bool>,
    #[serde(rename = "duckLevel")]
    duck_level: Option<i32>,
    #[serde(rename = "duckMedia")]
    duck_media: Option<bool>,
    onboarded: Option<bool>,
    #[serde(rename = "keepOpen")]
    keep_open: Option<bool>,
    taskbar: Option<bool>,

    hotkeys: Option<BTreeMap<String, Hotkey>>,
    features: Option<BTreeMap<String, bool>>,
}

fn set_b(dst: &mut bool, v: Option<bool>) {
    if let Some(v) = v {
        *dst = v;
    }
}

pub fn apply(p: CfgPatch) {
    config::set(|c| {
        if let Some(t) = &p.theme {
            if config::valid_theme(t) {
                c.theme = t.clone();
            }
        }
        if let Some(l) = &p.lang {
            if l.is_empty() || i18n::supported(l) {
                c.lang = l.clone();
            }
        }
        if let Some(m) = &p.mode {
            if ["dark", "light", "auto"].contains(&m.as_str()) {
                c.mode = m.clone();
            }
        }
        set_b(&mut c.low_battery, p.low_battery);
        if let Some(m) = &p.wheel_mode {
            if ["off", "battery", "balance"].contains(&m.as_str()) {
                c.wheel_mode = m.clone();
            }
        }
        if let Some(v) = p.sidetone.filter(|v| (0..=3).contains(v)) {
            c.sidetone = v;
        }
        if let Some(v) = p.mic_gain.filter(|v| (1..=7).contains(v)) {
            c.mic_gain = v;
        }
        if let Some(v) = p.mute_led.filter(|v| (0..=3).contains(v)) {
            c.mute_led = v;
        }
        if let Some(v) = p.custom_a.as_ref().filter(|v| config::parse_hex(v).is_some()) {
            c.custom_a = v.clone();
        }
        if let Some(v) = p.custom_b.as_ref().filter(|v| config::parse_hex(v).is_some()) {
            c.custom_b = v.clone();
        }
        set_b(&mut c.custom_oled, p.custom_oled);
        if let Some(v) = p.auto_off.filter(|v| (0..=90).contains(v)) {
            c.auto_off = v;
        }
        set_b(&mut c.auto_switch, p.auto_switch);
        set_b(&mut c.switch_mic, p.switch_mic);
        set_b(&mut c.full_notify, p.full_notify);
        set_b(&mut c.osd, p.osd);
        set_b(&mut c.mic_hotkey, p.mic_hotkey);
        set_b(&mut c.ptt, p.ptt);
        set_b(&mut c.sounds, p.sounds);
        if let Some(v) = p.low_level.filter(|v| (5..=75).contains(v)) {
            c.low_level = v;
        }
        if let Some(v) = p.ptt_key.filter(|v| (0..256).contains(v)) {
            c.ptt_key = v;
        }
        set_b(&mut c.duck_media, p.duck_media);
        set_b(&mut c.onboarded, p.onboarded);
        set_b(&mut c.keep_open, p.keep_open);
        set_b(&mut c.taskbar, p.taskbar);
        if let Some(hk) = &p.hotkeys {
            for (name, h) in hk {
                if hotkeys::DEFAULTS.iter().any(|d| d.0 == name) && h.vk > 0 && h.vk < 255 && h.modifiers <= 0xF {
                    c.hotkeys.insert(name.clone(), *h);
                }
            }
        }
        if let Some(v) = p.duck_level.filter(|v| (10..=100).contains(v)) {
            c.duck_level = (v + 5) / 10 * 10;
        }
        if let Some(f) = &p.features {
            let m = FEATURE_KEYS.iter().map(|k| (k.to_string(), f.get(*k).copied().unwrap_or_else(|| c.on(k)))).collect();
            c.features = m;
        }
    });
    if p.features.is_some() && !config::get().on("monitor") && crate::router::monitor_wanted() {
        crate::router::monitor_set("stop"); // "listen to yourself" was switched off while running
    }
    if p.auto_switch == Some(true) {
        crate::audio::auto_switch_now();
    }
    if p.lang.is_some() {
        crate::apply_lang(); // before the answer: the page reloads in the new language right away
    }
    if let Some(a) = p.autostart {
        autostart::set(a);
        autostart::refresh_cache();
    }
    let dev = |cmd: u8, arg: u8| std::thread::spawn(move || crate::devices::send_cmd(cmd, arg));
    if let Some(v) = p.sidetone.filter(|v| (0..=3).contains(v)) {
        dev(0x39, v as u8);
    }
    if let Some(v) = p.auto_off.filter(|v| (0..=90).contains(v)) {
        dev(0xa3, v as u8);
    }
    if let Some(v) = p.mic_gain.filter(|v| (1..=7).contains(v)) {
        dev(0x37, v as u8);
    }
    if let Some(v) = p.mute_led.filter(|v| (0..=3).contains(v)) {
        dev(0xae, crate::devices::mute_led_value(v));
    }
    util::post_ui(crate::WM_CFG_CHG, 0);
}
