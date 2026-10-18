//! Start with Windows (HKCU Run) and the move from the old app name (ArctisChatMix).

use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, LPARAM, WPARAM},
        System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE},
        UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_CLOSE},
    },
    core::HSTRING,
};

use crate::{APP_DIR_NAME, APP_NAME, APP_VER, appinfo, logf, util};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Arcmix";
const LEGACY_NAME: &str = "ArctisChatMix";

static CACHE: AtomicBool = AtomicBool::new(false);

pub fn enabled() -> bool {
    util::reg_open(RUN_KEY).map(|k| k.has(RUN_VALUE)).unwrap_or(false)
}

/// For the window state (the registry is not read on every update).
pub fn cached() -> bool {
    CACHE.load(Ordering::Relaxed)
}

pub fn refresh_cache() {
    CACHE.store(enabled(), Ordering::Relaxed);
}

/// The start command written in the registry ("" — autostart is off).
fn value() -> String {
    util::reg_open(RUN_KEY).and_then(|k| k.string(RUN_VALUE)).unwrap_or_default()
}

pub fn set(on: bool) {
    let Some(k) = util::reg_create(RUN_KEY) else {
        logf!("autostart: RegCreateKeyEx failed");
        return;
    };
    if !on {
        k.delete(RUN_VALUE);
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        k.set_string(RUN_VALUE, &format!("\"{}\"", exe.display()));
    }
}

/// When autostart points to another copy of the app (the exe was moved or a new version was
/// downloaded elsewhere), the path is moved to this exe.
pub fn sync_path() {
    let v = value();
    if v.is_empty() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.display().to_string();
    let reg = v.trim_matches(|c| c == '"' || c == ' ').to_string();
    if reg.eq_ignore_ascii_case(&exe) {
        return;
    }
    if !Path::new(&reg).exists() {
        logf!("autostart pointed to missing {v}, updating to {exe}");
        set(true);
        return;
    }
    // A pre-release build (version with "-") never takes autostart over from an existing copy:
    // trying an alpha must not make it start with Windows.
    if APP_VER.contains('-') {
        return;
    }
    // the version is in the file name, so a new version is another file: autostart moves to the
    // newer copy of the same app (an older one started for a while does not take it)
    if appinfo::file_string_info(&reg, "ProductName") == APP_NAME && crate::version::newer(APP_VER, &appinfo::file_string_info(&reg, "ProductVersion")) {
        logf!("autostart: {reg} is an older version, switching to {exe}");
        set(true);
    }
}

// ---------- the old name ----------

/// If the previous version runs, ask it to close (it puts app volumes back and removes its icon)
/// and wait up to 5 s.
pub fn close_legacy_app() -> bool {
    let name = HSTRING::from(format!(r"Local\{LEGACY_NAME}Singleton"));
    let running = || unsafe {
        match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &name) {
            Ok(h) => {
                let _ = CloseHandle(h);
                true
            }
            Err(_) => false,
        }
    };
    if !running() {
        return false;
    }
    unsafe {
        if let Ok(w) = FindWindowW(&HSTRING::from(format!("{LEGACY_NAME}Wnd")), None) {
            let _ = PostMessageW(Some(w), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    for _ in 0..50 {
        if !running() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

/// The settings folder under the old name → under the new one (if there is none yet).
pub fn migrate_legacy_data() -> String {
    let base = util::appdata_base();
    let (old, new) = (base.join(LEGACY_NAME), base.join(APP_DIR_NAME));
    if new.exists() || !old.exists() {
        return String::new();
    }
    if fs::rename(&old, &new).is_ok() {
        return format!("; settings moved from {}", old.display());
    }
    // the folder could not be moved (files busy) — copy the settings
    if let Ok(b) = fs::read(old.join("config.json")) {
        if fs::create_dir_all(&new).is_ok() && fs::write(new.join("config.json"), b).is_ok() {
            return format!("; settings copied from {}", old.display());
        }
    }
    String::new()
}

/// Autostart under the old name → under the new one, to this exe.
pub fn migrate_legacy_autostart() {
    let had = util::reg_open(RUN_KEY).map(|k| k.has(LEGACY_NAME)).unwrap_or(false);
    if !had {
        return;
    }
    if let Some(k) = util::reg_create(RUN_KEY) {
        k.delete(LEGACY_NAME);
    }
    set(true);
    logf!("autostart moved to the new name");
}
