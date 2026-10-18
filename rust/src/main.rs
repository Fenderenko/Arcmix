//! Arcmix: a Windows tray app — game / chat balance for any apps, equalizer, microphone and headset
//! settings in one window next to the clock: a WebView2 page (web/index.html) over a local HTTP API.

#![windows_subsystem = "windows"]

mod appinfo;
mod audio;
mod autostart;
mod cfgpatch;
mod config;
mod devices;
mod devs;
mod dsp;
mod fly;
mod headset;
mod hotkeys;
mod http;
mod i18n;
mod icon;
mod mic;
mod osd;
mod png;
mod rnnoise;
mod router;
mod server;
mod settings;
mod state;
mod tray;
mod update;
mod util;
mod version;
mod vst;
mod wintheme;

use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
            LibraryLoader::GetModuleHandleW,
            Threading::CreateMutexW,
        },
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_SYSTEM_AWARE, SetProcessDpiAwarenessContext},
            WindowsAndMessaging::{
                AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GetMessageW, GetWindowThreadProcessId, KillTimer, MSG, PostMessageW,
                PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY, WM_ENDSESSION, WM_HOTKEY, WM_LBUTTONUP, WM_MOUSEMOVE,
                WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW,
            },
        },
    },
    core::{HSTRING, w},
};

pub const APP_NAME: &str = "Arcmix";
/// The settings folder in %APPDATA%.
pub const APP_DIR_NAME: &str = "Arcmix";
pub const APP_VER: &str = env!("CARGO_PKG_VERSION");

const WM_APP: u32 = 0x8000;
pub const WM_TRAY: u32 = WM_APP + 1;
pub const WM_HID: u32 = WM_APP + 2;
pub const WM_WHEEL_TRY: u32 = WM_APP + 3;
pub const WM_SET_POS: u32 = WM_APP + 4;
pub const WM_CFG_CHG: u32 = WM_APP + 5;
pub const WM_HIDE_FLY: u32 = WM_APP + 6;
pub const WM_QUIT_REQ: u32 = WM_APP + 7;
pub const WM_OPEN_URL: u32 = WM_APP + 8;
/// wheel over the icon changes the balance (wParam: 1 — towards chat, 0 — towards game)
pub const WM_WHEEL_BAL: u32 = WM_APP + 9;
pub const WM_RUN_UI: u32 = WM_APP + 10;
pub const WM_PTT: u32 = WM_APP + 11;
pub const WM_MIC_CHG: u32 = WM_APP + 12;
/// displays changed: wParam 1 — the window is hidden (close it), 0 — shown
pub const WM_FLY_RESET: u32 = WM_APP + 13;
/// the app was started again (e.g. the pinned taskbar button)
pub const WM_SHOW_REQ: u32 = WM_APP + 14;

const TIMER_VST: usize = 6;
pub const TIMER_PTT: usize = 8;
/// picking a hotkey ends by itself (the window may have closed meanwhile)
pub const TIMER_HKCAP: usize = 13;
/// (wparam 1 / 0) picking a hotkey in the window starts / ends
pub const WM_HK_CAPTURE: u32 = WM_APP + 15;

/// Balance position for other threads (-100..100).
pub static POS: AtomicI32 = AtomicI32::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub fn dpi_scale() -> f64 {
    fly::dpi_scale()
}

/// Windows version (major, minor, build) from RtlGetVersion — GetVersionEx lies without a manifest.
pub fn windows_version() -> (u32, u32, u32) {
    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        csd: [u16; 128],
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
    }
    let mut v = OsVersionInfo { size: std::mem::size_of::<OsVersionInfo>() as u32, major: 0, minor: 0, build: 0, platform: 0, csd: [0; 128] };
    unsafe {
        RtlGetVersion(&mut v);
    }
    (v.major, v.minor, v.build)
}

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
    }
    let mutex = unsafe { CreateMutexW(None, false, w!(r"Local\ArcmixSingleton")) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        show_running(); // a copy runs already — its window is shown
        return;
    }
    // held while the app runs: a second copy does not start
    let _mutex: HANDLE = mutex.unwrap_or_default();
    let legacy_closed = autostart::close_legacy_app(); // the previous version (old name) — close it, two must not run
    let legacy_note = autostart::migrate_legacy_data();
    util::init_paths(APP_DIR_NAME);
    util::install_crash_output();
    logf!("=== start {APP_NAME} {APP_VER} ===");
    if legacy_closed || !legacy_note.is_empty() {
        logf!("rename:{}{}", if legacy_closed { " previous version closed" } else { "" }, legacy_note);
    }
    let v = windows_version();
    logf!("Windows {}.{} build {}", v.0, v.1, v.2);

    config::replace(config::load());
    i18n::set_lang(&i18n::resolve(&config::get().lang, i18n::system_lang()));
    logf!("language: {} (Windows: {})", i18n::lang(), i18n::system_lang());
    config::move_custom_eq(&i18n::t("Власний"));
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); // STA — needed for WebView2
    }
    audio::start();
    tray::init();
    TASKBAR_CREATED.store(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }, Ordering::Relaxed);

    let hwnd = unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: w!("ArcmixWnd"), ..Default::default() };
        if RegisterClassExW(&wc) == 0 {
            logf!("RegisterClassEx failed");
            return;
        }
        CreateWindowExW(WINDOW_EX_STYLE(0), w!("ArcmixWnd"), &HSTRING::from(APP_NAME), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, Some(inst.into()), None)
    };
    let Ok(hwnd) = hwnd else {
        logf!("CreateWindowEx failed");
        return;
    };
    util::set_main_hwnd(hwnd);

    autostart::migrate_legacy_autostart();
    autostart::sync_path();
    tray::add_or_retry();
    hotkeys::apply();
    mic::apply_hotkeys(); // and the mouse hook if push-to-talk is on a mouse button

    server::start();
    router::start();
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SetTimer(Some(hwnd), TIMER_VST, 10000, None);
    }
    vst::sync();
    std::thread::spawn(fly::cleanup_old);
    devices::start();

    let mut m = MSG::default();
    while unsafe { GetMessageW(&mut m, None, 0, 0) }.0 > 0 {
        unsafe {
            let _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
    shutdown();
    unsafe {
        let _ = CloseHandle(_mutex);
    }
}

/// The app runs already — ask it to show its window (this is how a button pinned to the taskbar
/// works too: Windows starts the exe again).
fn show_running() {
    unsafe {
        let Ok(w) = FindWindowW(w!("ArcmixWnd"), None) else { return };
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, Some(&mut pid));
        let _ = AllowSetForegroundWindow(pid); // the running copy may bring its window to the front
        let _ = PostMessageW(Some(w), WM_SHOW_REQ, WPARAM(0), LPARAM(0));
    }
}

fn shutdown() {
    config::flush();
    hotkeys::unregister_all();
    tray::unhook_mouse();
    mic::shutdown();
    tray::remove();
    fly::close();
    devices::shutdown();
    audio::restore_and_stop(); // puts back apps sent into cables too
    router::shutdown();
    vst::shutdown();
    config::flush();
    logf!("exit");
}

/// Updates the language from the settings; true when it changed.
pub fn apply_lang() -> bool {
    let l = i18n::resolve(&config::get().lang, i18n::system_lang());
    if l == i18n::lang() {
        return false;
    }
    i18n::set_lang(&l);
    logf!("language: {l}");
    true
}

/// (UI thread) The settings changed from the window (or the menu).
pub fn on_cfg_changed() {
    std::thread::spawn(|| {
        router::sync(&config::get());
        audio::reapply();
    });
    vst::sync();
    tray::update(true);
    fly::background();
    fly::apply_style();
    tray::apply_audio();
    hotkeys::apply();
    mic::apply_hotkeys();
    server::notify_subs();
}

/// Headset state changed (HID threads post WM_HID).
fn on_hid() {
    let (conn, powered, fresh, game, chat, have, lvl, chg) = {
        let mut h = headset::lock();
        let fresh = std::mem::take(&mut h.mix_new);
        (h.connected, h.powered, fresh, h.game, h.chat, h.have_batt, h.batt_pct, h.charging)
    };
    devices::on_state(conn, powered);
    if !conn {
        tray::set_pos_quiet(0.0);
        audio::set(100, 100);
        tray::start_anim();
        tray::update(true);
        return;
    }
    if fresh {
        let p = if game < 100 {
            (100 - game) as f64
        } else if chat < 100 {
            -((100 - chat) as f64)
        } else {
            0.0
        };
        tray::set_pos_quiet(p);
        tray::apply_audio();
        tray::start_anim();
        osd::show_balance(p);
    }
    server::notify_subs();
    if have && powered {
        devices::on_battery(lvl, chg);
    }
    tray::update(true);
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let r = util::safely_value("main window", || handle(h, m, wp, lp)).flatten();
    match r {
        Some(v) => v,
        None => unsafe { DefWindowProcW(h, m, wp, lp) },
    }
}

fn handle(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    match m {
        WM_TRAY => {
            match (lp.0 & 0xffff) as u32 {
                WM_LBUTTONUP => fly::toggle(),
                WM_RBUTTONUP => tray::show_menu(),
                WM_MOUSEMOVE => tray::on_hover(), // the cursor is over the icon
                _ => {}
            }
        }
        WM_HID => on_hid(),
        WM_WHEEL_TRY => tray::toggle_battery(),
        WM_WHEEL_BAL => {
            let p = tray::pos() + if wp.0 == 1 { 5.0 } else { -5.0 };
            tray::set_pos(p);
            tray::update(true); // the tip with percentages right away
            osd::show_balance(tray::pos());
        }
        WM_SET_POS => tray::set_pos(wp.0 as f64 - 200.0),
        WM_CFG_CHG => on_cfg_changed(),
        WM_HIDE_FLY => fly::hide(),
        WM_QUIT_REQ => unsafe { PostQuitMessage(0) },
        WM_RUN_UI => util::drain_ui(),
        WM_FLY_RESET => fly::reset(wp.0 == 1),
        WM_SHOW_REQ => fly::on_show_req(),
        WM_OPEN_URL => {
            if let Some(u) = server::OPEN_URLS.get(wp.0) {
                fly::hide();
                util::shell_open(u);
            }
        }
        WM_TIMER => match wp.0 {
            tray::TIMER_ANIM => tray::anim_tick(),
            tray::TIMER_BATT => tray::set_battery(false),
            tray::TIMER_TRAY => tray::retry(),
            TIMER_PTT => mic::ptt_release(),
            TIMER_HKCAP => hotkeys::capture(false),
            TIMER_VST => vst::timer(),
            fly::TIMER_IDLE => fly::idle_tick(),
            fly::TIMER_WATCH => fly::watch(),
            fly::TIMER_HIDE => fly::hide_tick(),
            tray::TIMER_HOVER => tray::hover_tick(),
            id => unsafe {
                let _ = KillTimer(Some(h), id);
            },
        },
        WM_HOTKEY => match wp.0 as i32 {
            hotkeys::HK_GAME => {
                tray::set_pos(tray::pos() - 5.0);
                osd::show_balance(tray::pos());
            }
            hotkeys::HK_CHAT => {
                tray::set_pos(tray::pos() + 5.0);
                osd::show_balance(tray::pos());
            }
            hotkeys::HK_BALANCE => {
                tray::set_pos(0.0);
                osd::show_balance(0.0);
            }
            hotkeys::HK_MIC => mic::toggle_mute(),
            _ => {}
        },
        WM_PTT => mic::on_ptt(wp.0 == 1),
        WM_MIC_CHG => tray::update(true),
        WM_HK_CAPTURE => hotkeys::capture(wp.0 != 0),
        WM_SETTINGCHANGE | 0x320 => {
            // WM_DWMCOLORIZATIONCOLORCHANGED too
            if tray::set_light_task(wintheme::refresh_system_light()) {
                tray::update(false);
                if config::get().mode == "auto" {
                    // "as in Windows" mode
                    fly::background();
                    server::notify_subs();
                }
            }
            if wintheme::refresh() {
                // the "as in Windows" theme: accent and mode
                let c = config::get();
                if c.theme == "win" || c.theme == "black" {
                    tray::update(true);
                    fly::background();
                }
                server::notify_subs();
            }
            return Some(LRESULT(0));
        }
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        // Windows is shutting down or signing out: the usual exit (app volumes back, the equalizer
        // into the headset flash) — the process may be ended right after this message
        WM_ENDSESSION if wp.0 != 0 => {
            shutdown();
            std::process::exit(0);
        }
        _ => {
            let tc = TASKBAR_CREATED.load(Ordering::Relaxed);
            if tc != 0 && m == tc {
                tray::add_or_retry(); // Explorer restarted
                return Some(LRESULT(0));
            }
            return None;
        }
    }
    Some(LRESULT(0))
}
