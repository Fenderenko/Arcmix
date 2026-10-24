//! Profiles bound to an app: the profile applies while that app is in the foreground, and the
//! settings from before come back once another app takes the focus. Windows reports the change
//! (SetWinEventHook, EVENT_SYSTEM_FOREGROUND) — no polling; the hook is set only while some profile
//! is bound to an app. Everything here runs on the UI thread.

use std::cell::{Cell, RefCell};

use windows::Win32::{
    Foundation::HWND,
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        WindowsAndMessaging::{EVENT_SYSTEM_FOREGROUND, GetWindowThreadProcessId, WINEVENT_OUTOFCONTEXT},
    },
};

use crate::{
    config::{self, Profile},
    logf, settings, util,
};

thread_local! {
    static HOOK: Cell<HWINEVENTHOOK> = Cell::new(HWINEVENTHOOK::default());
    /// the bound profile applied now and the settings it replaced
    static ACTIVE: RefCell<Option<(String, Profile)>> = const { RefCell::new(None) };
}

/// (UI thread, at start and after every settings change) The hook is there exactly while some
/// profile is bound to an app.
pub fn apply() {
    let c = config::get();
    let want = c.on("profiles") && c.profiles.iter().any(|p| !p.app.is_empty());
    let have = HOOK.with(|h| !h.get().is_invalid());
    if want && !have {
        let h = unsafe { SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT) };
        HOOK.with(|x| x.set(h));
    } else if !want && have {
        HOOK.with(|x| {
            let _ = unsafe { UnhookWinEvent(x.replace(HWINEVENTHOOK::default())) };
        });
        // nothing is bound any more: what the bound profile replaced comes back
        restore();
    }
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _event: u32, hwnd: HWND, _obj: i32, _child: i32, _thread: u32, _time: u32) {
    util::safely("focus", || foreground(hwnd));
}

fn foreground(hwnd: HWND) {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    // the app's own window (opened to look at the profile, say) changes nothing
    if pid == 0 || pid == std::process::id() {
        return;
    }
    let path = util::exe_path_of(pid);
    let exe = path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
    let exe = crate::appinfo::canonical(&exe).to_string();
    let c = config::get();
    let bound = c.profiles.iter().find(|p| !p.app.is_empty() && p.app == exe).cloned();
    let active = ACTIVE.with(|a| a.borrow().as_ref().map(|x| x.0.clone()));
    match (active, bound) {
        (None, Some(p)) => {
            let before = settings::snapshot_profile("");
            logf!("profile for {exe}: {}", p.name);
            settings::apply_profile_data(&p);
            ACTIVE.with(|a| *a.borrow_mut() = Some((p.name.clone(), before)));
        }
        (Some(cur), Some(p)) if cur != p.name => {
            logf!("profile for {exe}: {}", p.name);
            settings::apply_profile_data(&p);
            ACTIVE.with(|a| a.borrow_mut().as_mut().map(|x| x.0 = p.name.clone()));
        }
        (Some(_), None) => restore(),
        _ => {}
    }
}

fn restore() {
    if let Some((name, before)) = ACTIVE.with(|a| a.borrow_mut().take()) {
        logf!("profile {name}: the app lost the focus, the settings from before are back");
        settings::apply_profile_data(&before);
    }
}
