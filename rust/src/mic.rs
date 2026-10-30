//! Microphone: the Ctrl+Alt+M hotkey (mute / unmute), push-to-talk (a key or a mouse button),
//! sounds and the badge on the tray icon. The microphone acted on is the one the app controls
//! (Windows' default or the chosen one).

use std::{
    cell::Cell,
    sync::{
        LazyLock,
        atomic::{AtomicBool, AtomicI32, Ordering},
        mpsc,
    },
    time::Duration,
};

use windows::{
    Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT},
        UI::WindowsAndMessaging::{CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, KillTimer, PostMessageW, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL},
    },
    core::PCWSTR,
};

use crate::{TIMER_PTT, WM_PTT, audio, config, hotkeys, i18n::t, logf, osd, tray, util};

static PTT_ON: AtomicBool = AtomicBool::new(false);
static PTT_KEY: AtomicI32 = AtomicI32::new(0);
static PTT_HELD: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// (UI thread) talking with push-to-talk right now; push-to-talk was on last time
    static TALKING: Cell<bool> = const { Cell::new(false) };
    static WAS_ON: Cell<bool> = const { Cell::new(false) };
    static KB_HOOK: Cell<isize> = const { Cell::new(0) };
}

/// Two soft chime notes: the microphone on — up a fifth (E5 → B5), off — down (B5 → E5).
static SND_UNMUTE: LazyLock<Vec<u8>> = LazyLock::new(|| make_chime(&[(659.26, 0.0), (987.77, 0.085)], 0.085));
static SND_MUTE: LazyLock<Vec<u8>> = LazyLock::new(|| make_chime(&[(987.77, 0.0), (659.26, 0.085)], 0.08));

/// Kalimba-like notes (frequency, start in seconds): a rounded attack, a sine with soft overtones
/// that fade faster than the fundamental, a long exponential tail; no clicks at the edges.
fn make_chime(notes: &[(f64, f64)], amp: f64) -> Vec<u8> {
    const RATE: usize = 44100;
    // 40 ms of silence first: a wireless headset has time to "wake up", and the sound does not crackle
    let lead = RATE * 40 / 1000;
    let dur = 0.42;
    let n = (dur * RATE as f64) as usize;
    let mut out = vec![0f64; n];
    let tau = std::f64::consts::TAU;
    for &(f, start) in notes {
        let s0 = (start * RATE as f64) as usize;
        for k in s0..n {
            let t = (k - s0) as f64 / RATE as f64;
            let att = 0.5 - 0.5 * (std::f64::consts::PI * (t / 0.006).min(1.0)).cos(); // a 6 ms rounded attack
            let body = (tau * f * t).sin() * (-t / 0.11).exp();
            let shine = 0.22 * (tau * 2.0 * f * t).sin() * (-t / 0.04).exp() + 0.06 * (tau * 3.01 * f * t).sin() * (-t / 0.02).exp();
            out[k] += att * (body + shine);
        }
    }
    let mut pcm = vec![0i16; lead + n];
    for (k, v) in out.iter().enumerate() {
        let x = k as f64 / n as f64;
        let fade = if x > 0.8 { 0.5 + 0.5 * (std::f64::consts::PI * (x - 0.8) / 0.2).cos() } else { 1.0 }; // to zero at the end
        pcm[lead + k] = (amp * fade * v * 32767.0).clamp(-32767.0, 32767.0) as i16;
    }
    wav_bytes(&pcm, RATE as u32)
}

fn wav_bytes(pcm: &[i16], rate: u32) -> Vec<u8> {
    let data = (pcm.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend(b"RIFF");
    b.extend((36 + data).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes()); // PCM
    b.extend(1u16.to_le_bytes()); // mono
    b.extend(rate.to_le_bytes());
    b.extend((rate * 2).to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(data.to_le_bytes());
    for v in pcm {
        b.extend(v.to_le_bytes());
    }
    b
}

fn play(snd: &'static [u8]) {
    if !config::get().sounds || snd.is_empty() {
        return;
    }
    unsafe {
        let _ = PlaySoundW(PCWSTR(snd.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

/// (UI thread) The mute hotkey and push-to-talk mode from the settings.
pub fn apply_hotkeys() {
    let c = config::get();
    if c.mic_hotkey {
        hotkeys::register(hotkeys::HK_MIC, "mic", hotkeys::hotkey_of(&c, "mic"), 0x4000); // no autorepeat
    } else {
        hotkeys::unregister(hotkeys::HK_MIC, "mic");
    }
    PTT_KEY.store(c.ptt_key, Ordering::Relaxed);
    let on = c.ptt && c.ptt_key > 0;
    PTT_ON.store(on, Ordering::Relaxed);
    let want_kb = on && c.ptt_key > 6; // 1–6 — mouse buttons (the mouse hook catches them)
    let hook = KB_HOOK.with(|h| h.get());
    if want_kb && hook == 0 {
        match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), None, 0) } {
            Ok(h) => KB_HOOK.with(|x| x.set(h.0 as isize)),
            Err(e) => logf!("failed to install keyboard hook: {e}"),
        }
    } else if !want_kb && hook != 0 {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(hook as *mut _));
        }
        KB_HOOK.with(|x| x.set(0));
    }
    tray::update_mouse_hook();
    if on != WAS_ON.with(|w| w.get()) {
        WAS_ON.with(|w| w.set(on));
        TALKING.with(|x| x.set(false));
        audio::set_mic_mute(on); // push-to-talk switched on — the mic is silent until the key is pressed
        tray::update(false);
    }
}

fn vk_matches(vk: u32, want: i32) -> bool {
    match want {
        0x10 => matches!(vk, 0x10 | 0xA0 | 0xA1), // Shift
        0x11 => matches!(vk, 0x11 | 0xA2 | 0xA3), // Ctrl
        0x12 => matches!(vk, 0x12 | 0xA4 | 0xA5), // Alt
        _ => vk as i32 == want,
    }
}

unsafe extern "system" fn kb_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && PTT_ON.load(Ordering::Relaxed) && !hotkeys::CAPTURING.load(Ordering::Relaxed) {
        let vk = unsafe { (*(lp.0 as *const KBDLLHOOKSTRUCT)).vkCode };
        if vk_matches(vk, PTT_KEY.load(Ordering::Relaxed)) {
            let hwnd = util::main_hwnd();
            match wp.0 as u32 {
                // WM_KEYDOWN, WM_SYSKEYDOWN (repeats ignored)
                0x100 | 0x104 => {
                    if !PTT_HELD.swap(true, Ordering::AcqRel) {
                        unsafe {
                            let _ = PostMessageW(Some(hwnd), WM_PTT, WPARAM(1), LPARAM(0));
                        }
                    }
                }
                0x101 | 0x105 => {
                    if PTT_HELD.swap(false, Ordering::AcqRel) {
                        unsafe {
                            let _ = PostMessageW(Some(hwnd), WM_PTT, WPARAM(0), LPARAM(0));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

/// Push-to-talk works on a mouse button (then the mouse hook is needed).
pub fn ptt_mouse_wanted() -> bool {
    let k = PTT_KEY.load(Ordering::Relaxed);
    PTT_ON.load(Ordering::Relaxed) && k > 0 && k <= 6
}

/// The middle (4) and side (5, 6) mouse buttons for push-to-talk — from the mouse hook.
pub fn ptt_mouse(msg: u32, mouse_data: u32) {
    if !PTT_ON.load(Ordering::Relaxed) || hotkeys::CAPTURING.load(Ordering::Relaxed) {
        return;
    }
    let (btn, down) = match msg {
        0x207 | 0x208 => (4, msg == 0x207),                         // WM_MBUTTONDOWN / UP
        0x20B | 0x20C => (4 + (mouse_data >> 16) as i32, msg == 0x20B), // WM_XBUTTONDOWN / UP
        _ => return,
    };
    if btn != PTT_KEY.load(Ordering::Relaxed) {
        return;
    }
    let hwnd = util::main_hwnd();
    if down && !PTT_HELD.swap(true, Ordering::AcqRel) {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_PTT, WPARAM(1), LPARAM(0));
        }
    } else if !down && PTT_HELD.swap(false, Ordering::AcqRel) {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_PTT, WPARAM(0), LPARAM(0));
        }
    }
}

/// (UI thread) Pressed — talking, released — silence again after 180 ms (so the end of a word is
/// not cut off).
pub fn on_ptt(down: bool) {
    if !PTT_ON.load(Ordering::Relaxed) {
        return;
    }
    let hwnd = util::main_hwnd();
    if down {
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_PTT);
        }
        if !TALKING.with(|x| x.get()) {
            TALKING.with(|x| x.set(true));
            audio::set_mic_mute(false);
            play(&SND_UNMUTE);
            tray::update(false);
        }
        return;
    }
    unsafe {
        SetTimer(Some(hwnd), TIMER_PTT, 180, None);
    }
}

/// (UI thread) The release delay passed — the microphone goes silent.
pub fn ptt_release() {
    unsafe {
        let _ = KillTimer(Some(util::main_hwnd()), TIMER_PTT);
    }
    if TALKING.with(|x| x.get()) && PTT_ON.load(Ordering::Relaxed) {
        TALKING.with(|x| x.set(false));
        audio::set_mic_mute(true);
        play(&SND_MUTE);
        tray::update(false);
    }
}

/// On exit: a microphone held muted by push-to-talk is unmuted (waits up to 1 s).
pub fn shutdown() {
    let hook = KB_HOOK.with(|h| h.replace(0));
    if hook != 0 {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(hook as *mut _));
        }
    }
    if PTT_ON.load(Ordering::Relaxed) {
        let (tx, rx) = mpsc::channel();
        audio::run(move |e| {
            e.unmute_mic();
            let _ = tx.send(());
        });
        let _ = rx.recv_timeout(Duration::from_secs(1));
    }
}

/// Ctrl+Alt+M.
pub fn toggle_mute() {
    let m = audio::mic_state();
    if !m.ok {
        osd::show(Some(osd::ICON_MIC_OFF), &t("Мікрофон не знайдено"));
        return;
    }
    let mute = !m.mute;
    audio::set_mic_mute(mute);
    if mute {
        play(&SND_MUTE);
        osd::show(Some(osd::ICON_MIC_OFF), &t("Вимк."));
    } else {
        play(&SND_UNMUTE);
        osd::show(Some(osd::ICON_MIC), &t("Увімк."));
    }
}

/// Tray icon badge: 0 none, 1 muted (red), 2 talking with push-to-talk (green).
pub fn badge() -> i32 {
    if PTT_ON.load(Ordering::Relaxed) {
        return if TALKING.with(|x| x.get()) { 2 } else { 0 };
    }
    let m = audio::mic_state();
    if m.ok && m.mute {
        return 1;
    }
    // muted with the headset button: only when that headset's microphone is the one in use
    if m.cont.and_then(|c| crate::headset::mic_muted_at(&c)) == Some(true) {
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chimes_stay_quiet_and_end_in_silence() {
        for snd in [&*SND_UNMUTE, &*SND_MUTE] {
            let pcm: Vec<i16> = snd[44..].chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
            let peak = pcm.iter().map(|v| v.unsigned_abs()).max().unwrap();
            assert!(peak > 2000 && peak < 12000, "peak {peak}");
            assert!(pcm.last().unwrap().unsigned_abs() < 50, "no click at the end");
        }
    }

    /// The sounds as files to listen to: ARCMIX_SOUNDS_DIR=<dir> cargo test -- --ignored sound_files
    #[test]
    #[ignore]
    fn sound_files() {
        let Ok(dir) = std::env::var("ARCMIX_SOUNDS_DIR") else { return };
        std::fs::write(format!("{dir}/mic_on.wav"), &*SND_UNMUTE).unwrap();
        std::fs::write(format!("{dir}/mic_off.wav"), &*SND_MUTE).unwrap();
    }
}
