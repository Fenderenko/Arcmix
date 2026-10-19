//! Reassignable hotkeys: more game, more chat, even balance, microphone, next / previous output and
//! input device.

use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use windows::Win32::UI::{
    Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey},
    WindowsAndMessaging::{KillTimer, SetTimer},
};

use crate::{
    config::{self, Config, Hotkey},
    logf, util,
};

/// The arrow points where the line in the window moves: more game — the chat line steps right.
/// Devices: Ctrl+Alt with ←/→ for the output, ↑/↓ for the input.
pub const DEFAULTS: [(&str, Hotkey); 8] = [
    ("game", Hotkey { modifiers: 6, vk: 0x27 }),       // Ctrl+Shift+→
    ("chat", Hotkey { modifiers: 6, vk: 0x25 }),       // Ctrl+Shift+←
    ("balance", Hotkey { modifiers: 6, vk: 0x28 }),    // Ctrl+Shift+↓
    ("mic", Hotkey { modifiers: 3, vk: b'M' as u32 }), // Ctrl+Alt+M
    ("out", Hotkey { modifiers: 3, vk: 0x27 }),        // Ctrl+Alt+→ next output
    ("outPrev", Hotkey { modifiers: 3, vk: 0x25 }),    // Ctrl+Alt+← previous output
    ("in", Hotkey { modifiers: 3, vk: 0x28 }),         // Ctrl+Alt+↓ next input
    ("inPrev", Hotkey { modifiers: 3, vk: 0x26 }),     // Ctrl+Alt+↑ previous input
];

pub const HK_GAME: i32 = 1;
pub const HK_CHAT: i32 = 2;
pub const HK_BALANCE: i32 = 3;
pub const HK_MIC: i32 = 4;
pub const HK_OUT: i32 = 5;
pub const HK_IN: i32 = 6;
pub const HK_OUT_PREV: i32 = 7;
pub const HK_IN_PREV: i32 = 8;

const IDS: [(i32, &str); 7] =
    [(HK_GAME, "game"), (HK_CHAT, "chat"), (HK_BALANCE, "balance"), (HK_OUT, "out"), (HK_OUT_PREV, "outPrev"), (HK_IN, "in"), (HK_IN_PREV, "inPrev")];

thread_local! {
    /// What is registered now (UI thread).
    static REG: RefCell<HashMap<i32, Hotkey>> = RefCell::new(HashMap::new());
}

/// Taken by another app.
static FAILING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A new combination is being picked in the window: the hotkeys (and push-to-talk) are off, so the
/// keys reach the window instead of doing their job.
pub static CAPTURING: AtomicBool = AtomicBool::new(false);

/// (UI thread) Picking starts or ends; it also ends by itself after a while (the window may be gone).
pub fn capture(on: bool) {
    CAPTURING.store(on, Ordering::Relaxed);
    let hwnd = util::main_hwnd();
    if on {
        unregister_all();
        unsafe {
            let _ = SetTimer(Some(hwnd), crate::TIMER_HKCAP, 20_000, None);
        }
    } else {
        unsafe {
            let _ = KillTimer(Some(hwnd), crate::TIMER_HKCAP);
        }
        apply();
        crate::mic::apply_hotkeys();
    }
}

pub fn hotkey_of(c: &Config, name: &str) -> Hotkey {
    match c.hotkeys.get(name) {
        Some(h) if h.vk != 0 => *h,
        _ => DEFAULTS.iter().find(|d| d.0 == name).map(|d| d.1).unwrap_or_default(),
    }
}

/// "Ctrl+Alt+↓" for the tray menu ("" — a key without a short name).
pub fn label(h: Hotkey) -> String {
    let mut b = String::new();
    for (bit, s) in [(2, "Ctrl"), (1, "Alt"), (4, "Shift"), (8, "Win")] {
        if h.modifiers & bit != 0 {
            b += s;
            b += "+";
        }
    }
    let v = h.vk;
    match v {
        0x25..=0x28 => b += ["←", "↑", "→", "↓"][(v - 0x25) as usize],
        0x30..=0x39 | 0x41..=0x5A => b.push(v as u8 as char),
        0x70..=0x87 => b += &format!("F{}", v - 0x6F),
        _ => return String::new(),
    }
    b
}

fn set_fail(name: &str, fail: bool) {
    let mut f = FAILING.lock().unwrap();
    f.retain(|x| x != name);
    if fail {
        f.push(name.to_string());
    }
}

pub fn fails() -> Vec<String> {
    FAILING.lock().unwrap().clone()
}

/// (UI thread) Registers again only if the combination changed.
pub fn register(id: i32, name: &str, h: Hotkey, flags: u32) -> bool {
    if CAPTURING.load(Ordering::Relaxed) {
        return true; // registered again when picking ends
    }
    if h.vk == 0 {
        unregister(id, name); // no combination picked
        return true;
    }
    let hwnd = util::main_hwnd();
    let old = REG.with(|r| r.borrow().get(&id).copied());
    if let Some(old) = old {
        if old == h {
            return true;
        }
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), id);
        }
        REG.with(|r| r.borrow_mut().remove(&id));
    }
    let ok = unsafe { RegisterHotKey(Some(hwnd), id, HOT_KEY_MODIFIERS((h.modifiers & 0xF) | flags), h.vk) }.is_ok();
    if !ok {
        logf!("failed to register hotkey {name} (mod {}, vk {:#x}) — already in use?", h.modifiers, h.vk);
        set_fail(name, true);
        return false;
    }
    REG.with(|r| r.borrow_mut().insert(id, h));
    set_fail(name, false);
    true
}

pub fn unregister(id: i32, name: &str) {
    let had = REG.with(|r| r.borrow_mut().remove(&id).is_some());
    if had {
        unsafe {
            let _ = UnregisterHotKey(Some(util::main_hwnd()), id);
        }
    }
    set_fail(name, false);
}

/// (UI thread) Balance hotkeys; the microphone one is in mic::apply_hotkeys.
pub fn apply() {
    let c = config::get();
    for (id, name) in IDS {
        register(id, name, hotkey_of(&c, name), 0);
    }
}

pub fn unregister_all() {
    let ids: Vec<i32> = REG.with(|r| r.borrow().keys().copied().collect());
    for id in ids {
        unsafe {
            let _ = UnregisterHotKey(Some(util::main_hwnd()), id);
        }
    }
    REG.with(|r| r.borrow_mut().clear());
}
