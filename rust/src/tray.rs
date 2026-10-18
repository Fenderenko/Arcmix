//! The tray icon: adding and updating it, the balance / battery animation, the menu, the mouse
//! wheel over the icon and balloon notifications. Everything here runs on the UI thread.

use std::{
    cell::RefCell,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{LPARAM, POINT, RECT, WPARAM},
        UI::{
            Shell::{NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFY_ICON_DATA_FLAGS, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW},
            WindowsAndMessaging::{
                AppendMenuW, CallNextHookEx, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, GetSystemMetrics, HHOOK, HICON, HMENU, KillTimer, MENU_ITEM_FLAGS, MF_DISABLED, MF_GRAYED, MF_POPUP,
                MF_SEPARATOR, MF_STRING, MSLLHOOKSTRUCT, PostMessageW, SM_CXSMICON, SetForegroundWindow, SetTimer, SetWindowsHookExW, TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
                TrackPopupMenu, UnhookWindowsHookEx, WH_MOUSE_LL, WM_MOUSEWHEEL,
            },
        },
    },
    core::HSTRING,
};

use crate::{
    APP_NAME, APP_VER, POS, audio, config, devices, headset,
    hotkeys, i18n::t,
    icon::{self, IconParams},
    logf, util,
};

pub const TIMER_ANIM: usize = 1;
pub const TIMER_BATT: usize = 2;
pub const TIMER_TRAY: usize = 3;
pub const TIMER_HOVER: usize = 11;

const ID_OPEN: u32 = 1000;
const ID_RESET: u32 = 1001;
const ID_QUIT: u32 = 1199;
const ID_PROF: u32 = 6000;

/// UI thread state of the tray icon.
struct Ui {
    /// target balance -100..100
    pos: f64,
    cur_pos: f64,
    cur_b: f64,
    cur_bat: f64,
    batt_show: bool,
    animating: bool,
    last_tick: Instant,
    light_task: bool,
    icon: HICON,
    icon_size: usize,
    retries: u32,
    mouse_hook: HHOOK,
    hover: bool,
    wheel_accum: i32,
    wheel_last: Instant,
    wheel_toggle: Instant,
}

thread_local! {
    static UI: RefCell<Ui> = RefCell::new(Ui {
        pos: 0.0, cur_pos: 0.0, cur_b: 1.0, cur_bat: 0.0, batt_show: false, animating: false,
        last_tick: Instant::now(), light_task: false, icon: HICON::default(), icon_size: 16, retries: 0,
        mouse_hook: HHOOK::default(), hover: false, wheel_accum: 0,
        wheel_last: Instant::now(), wheel_toggle: Instant::now() - Duration::from_secs(1),
    });
}

/// Short borrow of the UI state (never call anything that may dispatch messages inside).
fn ui<R>(f: impl FnOnce(&mut Ui) -> R) -> R {
    UI.with(|u| f(&mut u.borrow_mut()))
}

pub fn pos() -> f64 {
    ui(|u| u.pos)
}

pub fn init() {
    let m = unsafe { GetSystemMetrics(SM_CXSMICON) };
    let light = crate::wintheme::system_light();
    ui(|u| {
        if m >= 16 {
            u.icon_size = m as usize;
        }
        u.light_task = light;
    });
}

fn nid(flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: util::main_hwnd(),
        uID: 1,
        uFlags: flags,
        ..Default::default()
    }
}

fn params() -> IconParams {
    // called on every animation frame: the settings are read in place, not copied
    let (cg, cc) = (icon::GAME, icon::CHAT);
    let hs = headset::snapshot();
    let mic = crate::mic::badge();
    ui(|u| IconParams {
        size: u.icon_size,
        pos: u.cur_pos,
        b: u.cur_b,
        bat: u.cur_bat,
        level: hs.batt_pct,
        light: u.light_task,
        mic,
        cg,
        cc,
    })
}

fn build_icon() -> HICON {
    let p = params();
    icon::make_hicon(p.size, &icon::render(&p))
}

fn tip_text() -> String {
    let hs = headset::snapshot();
    let p = pos();
    let g = 100 - p.max(0.0) as i32;
    let c = 100 - (-p).max(0.0) as i32;
    let mut s = format!("{APP_NAME}\n{} {g}% · {} {c}%", t("Гра"), t("Чат"));
    if hs.connected && hs.have_batt {
        s += &format!("\n{} {}%", t("Заряд"), hs.batt_pct);
        if hs.charging {
            s += &format!(" ({})", t("заряджається"));
        } else {
            let h = devices::batt_hours();
            if h > 0.0 {
                let hs = format!("{h:.1}");
                s += &format!(" · ≈ {} {}", hs.trim_end_matches(".0"), t("год"));
            }
        }
    }
    s
}

fn swap_icon(ic: HICON) {
    let old = ui(|u| std::mem::replace(&mut u.icon, ic));
    if !old.is_invalid() {
        unsafe {
            let _ = DestroyIcon(old);
        }
    }
}

/// Adds the icon. While Windows signs in (autostart) the taskbar may not be ready or busy — then
/// the call fails and is retried on a timer.
pub fn add() -> bool {
    let ic = build_icon();
    let mut n = nid(NIF_MESSAGE | NIF_ICON | NIF_TIP);
    n.uCallbackMessage = crate::WM_TRAY;
    n.hIcon = ic;
    util::copy_wide(&mut n.szTip, &tip_text());
    let mut ok = unsafe { Shell_NotifyIconW(NIM_ADD, &n) }.as_bool();
    if !ok {
        // maybe the icon is there already (adding "failed" by timeout but worked)
        ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &n) }.as_bool();
    }
    swap_icon(ic);
    if !ok {
        logf!("tray icon not added yet, will retry");
    }
    ok
}

pub fn add_or_retry() {
    if !add() {
        ui(|u| u.retries = 0);
        unsafe {
            SetTimer(Some(util::main_hwnd()), TIMER_TRAY, 2000, None);
        }
    }
}

pub fn retry() {
    let n = ui(|u| {
        u.retries += 1;
        u.retries
    });
    if add() {
        unsafe {
            let _ = KillTimer(Some(util::main_hwnd()), TIMER_TRAY);
        }
        logf!("tray icon added after {n} retries");
    } else if n >= 90 {
        // 3 minutes
        unsafe {
            let _ = KillTimer(Some(util::main_hwnd()), TIMER_TRAY);
        }
    }
}

pub fn update(with_tip: bool) {
    let ic = build_icon();
    let mut fl = NIF_ICON;
    if with_tip {
        fl |= NIF_TIP;
    }
    let mut n = nid(fl);
    n.hIcon = ic;
    if with_tip {
        util::copy_wide(&mut n.szTip, &tip_text());
    }
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
    }
    swap_icon(ic);
}

pub fn remove() {
    let n = nid(NOTIFY_ICON_DATA_FLAGS(0));
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &n);
    }
    swap_icon(HICON::default());
}

pub fn balloon(title: &str, text: &str, warn: bool) {
    let mut n = nid(NIF_INFO);
    util::copy_wide(&mut n.szInfoTitle, title);
    util::copy_wide(&mut n.szInfo, text);
    n.dwInfoFlags = if warn { NIIF_WARNING } else { NIIF_INFO };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
    }
}

// ---------- animation ----------

pub fn start_anim() {
    let start = ui(|u| {
        if u.animating {
            return false;
        }
        u.animating = true;
        u.last_tick = Instant::now();
        true
    });
    if start {
        unsafe {
            SetTimer(Some(util::main_hwnd()), TIMER_ANIM, 16, None);
        }
    }
}

fn approach(cur: f64, target: f64, dt: f64, tau: f64, eps: f64) -> (f64, bool) {
    let d = target - cur;
    if d.abs() <= eps {
        return (target, true);
    }
    (cur + d * (1.0 - (-dt / tau).exp()), false)
}

pub fn anim_tick() {
    let done = ui(|u| {
        let now = Instant::now();
        let dt = now.duration_since(u.last_tick).as_secs_f64().min(0.1);
        u.last_tick = now;
        let (p, s1) = approach(u.cur_pos, u.pos, dt, 0.07, 0.15);
        u.cur_pos = p;
        let tb = if u.pos.abs() < 4.0 { 1.0 } else { 0.0 };
        let (b, s2) = approach(u.cur_b, tb, dt, 0.16, 0.01); // logo ↔ capsules: slow enough to be seen
        u.cur_b = b;
        // the morph runs on time (0.55 s); easing is in the renderer
        let step = dt / 0.55;
        if u.batt_show && u.cur_bat < 1.0 {
            u.cur_bat = (u.cur_bat + step).min(1.0);
        } else if !u.batt_show && u.cur_bat > 0.0 {
            u.cur_bat = (u.cur_bat - step).max(0.0);
        }
        let s3 = (u.batt_show && u.cur_bat >= 1.0) || (!u.batt_show && u.cur_bat <= 0.0);
        let done = s1 && s2 && s3;
        if done {
            u.animating = false;
        }
        done
    });
    update(false);
    if done {
        unsafe {
            let _ = KillTimer(Some(util::main_hwnd()), TIMER_ANIM);
        }
        update(true);
    }
}

/// Shows (number instead of ChatMix) or hides the battery level.
pub fn set_battery(on: bool) {
    if on && !headset::lock().connected {
        return;
    }
    ui(|u| u.batt_show = on);
    let hwnd = util::main_hwnd();
    unsafe {
        let _ = KillTimer(Some(hwnd), TIMER_BATT);
    }
    if on {
        let sec = config::get().hold_sec;
        if sec > 0 {
            unsafe {
                SetTimer(Some(hwnd), TIMER_BATT, sec as u32 * 1000, None);
            }
        }
    }
    start_anim();
}

pub fn toggle_battery() {
    let on = ui(|u| u.batt_show);
    set_battery(!on);
}

pub fn set_pos(p: f64) {
    let p = p.round().clamp(-100.0, 100.0);
    ui(|u| u.pos = p);
    POS.store(p as i32, Ordering::Relaxed);
    crate::server::notify_subs();
    apply_audio();
    start_anim();
}

/// Sets the target position without the animation of the audio (headset dial): the balance came
/// from the headset itself.
pub fn set_pos_quiet(p: f64) {
    ui(|u| u.pos = p);
    POS.store(p as i32, Ordering::Relaxed);
}

pub fn apply_audio() {
    let p = pos();
    let (mut g, mut c) = (100 - p.max(0.0) as i32, 100 - (-p).max(0.0) as i32);
    if devices::hardware_chatmix() {
        // hardware ChatMix (Arctis 7/9/Pro): the headset mixes itself
        (g, c) = (100, 100);
    }
    audio::set(g, c);
}

pub fn set_light_task(l: bool) -> bool {
    ui(|u| {
        let changed = u.light_task != l;
        u.light_task = l;
        changed
    })
}

// ---------- menu ----------

fn append(m: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str) {
    let s = HSTRING::from(text);
    unsafe {
        let _ = AppendMenuW(m, flags, id, &s);
    }
}

pub fn show_menu() {
    let c = config::get();
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else { return };
    let sub = |title: &str, fill: &dyn Fn(HMENU)| {
        if let Ok(m) = unsafe { CreatePopupMenu() } {
            fill(m);
            append(menu, MF_POPUP, m.0 as usize, title);
        }
    };
    append(menu, MF_DISABLED | MF_GRAYED, 0, &format!("{APP_NAME} {APP_VER}"));
    append(menu, MF_SEPARATOR, 0, "");
    append(menu, MF_STRING, ID_OPEN as usize, &t("Відкрити вікно"));
    let mut reset = t("Баланс: порівну");
    let k = hotkeys::label(hotkeys::hotkey_of(&c, "balance"));
    if !k.is_empty() {
        reset += &format!(" ({k})");
    }
    append(menu, MF_STRING, ID_RESET as usize, &reset);
    // only what is needed every day: the rest is in the window
    if !c.profiles.is_empty() && c.on("profiles") {
        sub(&t("Профіль"), &|m| {
            for (i, p) in c.profiles.iter().enumerate() {
                append(m, MF_STRING, ID_PROF as usize + i, &p.name);
            }
        });
    }
    append(menu, MF_SEPARATOR, 0, "");
    append(menu, MF_STRING, ID_QUIT as usize, &t("Вийти"));

    let mut pt = POINT::default();
    let hwnd = util::main_hwnd();
    let cmd = unsafe {
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let r = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_NONOTIFY | TPM_RETURNCMD, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), 0, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        r.0 as u32
    };
    on_menu(cmd);
}

fn on_menu(id: u32) {
    match id {
        ID_OPEN => crate::fly::open(),
        ID_RESET => set_pos(0.0),
        _ if id >= ID_PROF && ((id - ID_PROF) as usize) < config::get().profiles.len() => {
            let name = config::get().profiles[(id - ID_PROF) as usize].name.clone();
            crate::settings::apply_profile(&name);
        }
        ID_QUIT => unsafe { windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) },
        _ => {}
    }
    crate::server::notify_subs();
}

// ---------- mouse wheel over the icon ----------

/// The low-level mouse hook sees every mouse move in the system, so it is installed only when
/// needed: while the cursor is over the tray icon (wheel — battery or balance) or when
/// push-to-talk is on a mouse button. The rest of the time the app knows nothing of the mouse.
pub fn update_mouse_hook() {
    let want = ui(|u| u.hover) || crate::mic::ptt_mouse_wanted();
    let hook = ui(|u| u.mouse_hook);
    if want && hook.is_invalid() {
        match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) } {
            Ok(h) => ui(|u| u.mouse_hook = h),
            Err(e) => logf!("failed to install mouse hook: {e}"),
        }
    } else if !want && !hook.is_invalid() {
        unsafe {
            let _ = UnhookWindowsHookEx(hook);
        }
        ui(|u| u.mouse_hook = HHOOK::default());
    }
}

pub fn unhook_mouse() {
    let hook = ui(|u| std::mem::take(&mut u.mouse_hook));
    if !hook.is_invalid() {
        unsafe {
            let _ = UnhookWindowsHookEx(hook);
        }
    }
}

/// The cursor is over the icon (Windows reports moves over it) — listen to the wheel.
pub fn on_hover() {
    if ui(|u| u.hover) || config::get().wheel_mode == "off" {
        return;
    }
    ui(|u| u.hover = true);
    update_mouse_hook();
    unsafe {
        SetTimer(Some(util::main_hwnd()), TIMER_HOVER, 300, None);
    }
}

/// The cursor left the icon — stop listening to the wheel.
pub fn hover_tick() {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    if over_tray(pt) {
        return;
    }
    unsafe {
        let _ = KillTimer(Some(util::main_hwnd()), TIMER_HOVER);
    }
    ui(|u| u.hover = false);
    update_mouse_hook();
}

/// One wheel "click" is 120 units; precise wheels send smaller steps — they add up.
unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> windows::Win32::Foundation::LRESULT {
    if code >= 0 {
        let info = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
        crate::mic::ptt_mouse(wp.0 as u32, info.mouseData);
        if wp.0 as u32 == WM_MOUSEWHEEL && ui(|u| u.hover) {
            let mode = config::get().wheel_mode;
            if (mode == "battery" || mode == "balance") && over_tray(info.pt) {
                let delta = (info.mouseData >> 16) as i16 as i32;
                let hwnd = util::main_hwnd();
                ui(|u| {
                    let now = Instant::now();
                    if now.duration_since(u.wheel_last) > Duration::from_millis(600) {
                        u.wheel_accum = 0;
                    }
                    u.wheel_last = now;
                    u.wheel_accum += delta;
                    if mode == "balance" {
                        // every wheel click is a 5 % step, like the slider in the window
                        while u.wheel_accum >= 120 {
                            u.wheel_accum -= 120;
                            unsafe {
                                let _ = PostMessageW(Some(hwnd), crate::WM_WHEEL_BAL, WPARAM(1), LPARAM(0));
                            }
                        }
                        while u.wheel_accum <= -120 {
                            u.wheel_accum += 120;
                            unsafe {
                                let _ = PostMessageW(Some(hwnd), crate::WM_WHEEL_BAL, WPARAM(0), LPARAM(0));
                            }
                        }
                    } else if u.wheel_accum.abs() >= 120 && now.duration_since(u.wheel_toggle) > Duration::from_millis(220) {
                        u.wheel_accum = 0;
                        u.wheel_toggle = now;
                        unsafe {
                            let _ = PostMessageW(Some(hwnd), crate::WM_WHEEL_TRY, WPARAM(0), LPARAM(0));
                        }
                    }
                });
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

/// The tray icon rectangle (screen coordinates).
pub fn icon_rect() -> Option<RECT> {
    let id = NOTIFYICONIDENTIFIER { cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: util::main_hwnd(), uID: 1, ..Default::default() };
    unsafe { Shell_NotifyIconGetRect(&id) }.ok()
}

pub fn over_tray(pt: POINT) -> bool {
    icon_rect().is_some_and(|r| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom)
}
