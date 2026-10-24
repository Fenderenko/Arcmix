//! Settings export and import, the diagnostics report, and profiles (saved sound setups).

use std::{fs, path::PathBuf};

use serde_json::Value;
use windows::{
    Win32::UI::{
        Controls::Dialogs::{GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW},
        Shell::{FOLDERID_Desktop, FOLDERID_Documents, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath},
    },
    core::{HSTRING, PCWSTR, PWSTR},
};

use crate::{
    APP_NAME, APP_VER,
    config::{self, Config, Profile},
    i18n::{self, t},
    logf, tray, util,
};

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

/// (UI thread) The standard open or save file dialog.
fn file_dialog(save: bool, title: &str, filter: &str, def_name: &str) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    for (i, c) in def_name.encode_utf16().enumerate().take(1000) {
        buf[i] = c;
    }
    let flt: Vec<u16> = filter.encode_utf16().collect();
    let title = HSTRING::from(title);
    let ext = HSTRING::from("json");
    let dir = known_folder(&FOLDERID_Documents).map(|p| HSTRING::from(p.as_os_str()));
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(flt.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrDefExt: PCWSTR(ext.as_ptr()),
        lpstrInitialDir: dir.as_ref().map(|d| PCWSTR(d.as_ptr())).unwrap_or(PCWSTR::null()),
        hwndOwner: util::main_hwnd(),
        ..Default::default()
    };
    let ok = unsafe {
        if save {
            ofn.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_EXPLORER;
            GetSaveFileNameW(&mut ofn).as_bool()
        } else {
            ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_EXPLORER;
            GetOpenFileNameW(&mut ofn).as_bool()
        }
    };
    ok.then(|| util::from_wide(&buf))
}

const SETTINGS_FILTER: &str = "Arcmix (*.json)\0*.json\0\0";

/// (UI thread)
pub fn export() {
    let Some(p) = file_dialog(true, &t("Зберегти налаштування"), SETTINGS_FILTER, "Arcmix-settings.json") else { return };
    let r = serde_json::to_vec_pretty(&config::get()).map_err(|e| e.to_string()).and_then(|b| fs::write(&p, b).map_err(|e| e.to_string()));
    if let Err(e) = r {
        logf!("export settings: {e}");
        tray::balloon(&t("Не вдалося зберегти налаштування"), &e, true);
        return;
    }
    logf!("settings exported: {p}");
    show_in_explorer(&p);
}

/// (UI thread) The current settings stay next to them as config.json.bak.
pub fn import() {
    let Some(p) = file_dialog(false, &t("Завантажити налаштування"), SETTINGS_FILTER, "") else { return };
    let parsed = fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let c = parsed.as_ref().filter(|v| v.get("theme").is_some()).and_then(|v| serde_json::from_value::<Config>(v.clone()).ok());
    let Some(mut c) = c.filter(|c| config::valid_theme(&c.theme)) else {
        logf!("import settings {}: not a settings file", util::base_name(&p));
        tray::balloon(&t("Не вдалося завантажити налаштування"), &t("Це не файл налаштувань Arcmix."), true);
        return;
    };
    c.onboarded = true;
    config::migrate(&mut c);
    let path = util::config_path();
    let _ = fs::rename(&path, path.with_extension("json.bak"));
    config::replace(c.clone());
    config::save(&c);
    crate::apply_lang();
    logf!("settings imported from {p} (previous saved as config.json.bak)");
    util::post_ui(crate::WM_CFG_CHG, 0);
    crate::server::notify_subs();
}

/// Settings back to defaults; the current file stays next to them as config.json.bak.
pub fn reset() {
    let path = util::config_path();
    let _ = fs::rename(&path, path.with_extension("json.bak"));
    let c = Config { onboarded: true, ..Default::default() };
    config::replace(c.clone());
    config::save(&c);
    crate::apply_lang();
    logf!("settings reset to defaults (previous saved as config.json.bak)");
    util::post_ui(crate::WM_CFG_CHG, 0);
}

/// Opens the folder with the file selected.
pub fn show_in_explorer(path: &str) {
    let args = HSTRING::from(format!("/select,\"{path}\""));
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(None, windows::core::w!("open"), windows::core::w!("explorer.exe"), &args, None, windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
    }
}

/// (UI thread) The Windows Sound dialog on a tab (0 — playback, 3 — communications).
pub fn open_win_sound(tab: i32) {
    crate::fly::hide();
    let args = HSTRING::from(format!("shell32.dll,Control_RunDLL mmsys.cpl,,{tab}"));
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(None, windows::core::w!("open"), windows::core::w!("rundll32.exe"), &args, None, windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
    }
}

// ---------- diagnostics report ----------

fn system_info() -> String {
    let mut b = format!("{APP_NAME} {APP_VER} (Rust)\n");
    let v = crate::windows_version();
    b += &format!("Windows {}.{} build {}\n", v.0, v.1, v.2);
    b += &format!("UI language: {} (Windows: {})\n", i18n::lang(), i18n::system_lang());
    let hs = crate::headset::snapshot();
    b += &format!("Headset: {}, connected {}, powered {}, battery {}% (known {})\n", hs.dev_name, hs.connected, hs.powered, hs.batt_pct, hs.have_batt);
    let (r, c) = crate::audio::list_devices();
    for x in &r {
        b += &format!("output: {}{}{}\n", x.name, if x.default { " [default]" } else { "" }, if x.is_virtual { " [virtual]" } else { "" });
    }
    for x in &c {
        b += &format!("capture: {}{}{}\n", x.name, if x.default { " [default]" } else { "" }, if x.is_virtual { " [virtual]" } else { "" });
    }
    let (rt, mic) = crate::router::status(&config::get());
    b += &format!("routing: {rt:?}\nmic processing: {mic:?}\n");
    for p in crate::vst::status() {
        b += &format!("vst: {} {}\n", p.name, p.err);
    }
    b
}

/// A zip on the desktop with the log, crash report, settings and system info; returns its path.
pub fn make_report() -> Result<String, String> {
    let desk = known_folder(&FOLDERID_Desktop).unwrap_or_else(|| util::data_dir().to_path_buf());
    let path = desk.join(format!("Arcmix-report-{}.zip", util::file_stamp()));
    let mut files = Vec::new();
    for n in ["log.txt", "crash.txt"] {
        if let Ok(b) = fs::read(util::data_dir().join(n)) {
            files.push((n.to_string(), b));
        }
    }
    let mut c = config::get();
    for p in c.mic_fx.plugins.iter_mut() {
        p.state.clear(); // plugin state is large and tells nothing for diagnostics
    }
    if let Ok(b) = serde_json::to_vec_pretty(&c) {
        files.push(("config.json".into(), b));
    }
    files.push(("system.txt".into(), system_info().into_bytes()));
    fs::write(&path, crate::png::zip(&files)).map_err(|e| e.to_string())?;
    let p = path.display().to_string();
    logf!("report saved: {p}");
    Ok(p)
}

// ---------- profiles ----------

pub fn snapshot_profile(name: &str) -> Profile {
    let c = config::get();
    Profile {
        name: name.into(),
        eq: c.eq.clone(),
        eq_preset: c.eq_preset.clone(),
        soft_eq: c.soft_eq.clone(),
        sidetone: c.sidetone,
        gate: c.mic_fx.gate,
        ceil: c.mic_fx.ceil,
        denoise: c.mic_fx.denoise,
        app: String::new(),
    }
}

pub fn save_profile(name: &str, app: &str) {
    let name: String = name.trim().chars().take(40).collect();
    if name.is_empty() {
        return;
    }
    let mut p = snapshot_profile(&name);
    p.app = crate::appinfo::canonical(&app.trim().to_lowercase()).to_string();
    config::set(|c| match c.profiles.iter().position(|x| x.name.to_lowercase() == name.to_lowercase()) {
        Some(i) => c.profiles[i] = p,
        None => c.profiles.push(p),
    });
    logf!("profile saved: {name}");
}

pub fn delete_profile(name: &str) {
    config::set(|c| c.profiles.retain(|x| x.name != name));
}

/// Applies a profile and sends the changes to the headset.
pub fn apply_profile(name: &str) {
    let c = config::get();
    let Some(p) = c.profiles.iter().find(|x| x.name == name).cloned() else { return };
    apply_profile_data(&p);
}

/// Applies the settings of a profile (also one made on the fly: what a bound profile replaced).
pub fn apply_profile_data(p: &Profile) {
    let name = p.name.clone();
    config::set(|c| {
        if !p.eq.is_empty() {
            c.eq = p.eq.clone();
            c.eq_preset = p.eq_preset.clone();
        }
        c.soft_eq = p.soft_eq.clone();
        if p.sidetone >= 0 {
            c.sidetone = p.sidetone;
        }
        c.mic_fx.gate = p.gate;
        c.mic_fx.ceil = p.ceil;
        c.mic_fx.denoise = p.denoise;
    });
    crate::devices::apply_profile(p);
    logf!("profile applied: {name}");
    util::post_ui(crate::WM_CFG_CHG, 0);
}
