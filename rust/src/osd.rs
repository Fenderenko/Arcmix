//! On-screen hint (OSD): briefly shows the game / chat balance when ChatMix turns (headset dial,
//! hotkeys, mouse wheel over the icon), the microphone state on the hotkey, the output device and the
//! headset battery (low, charged). A layered window
//! above everything, without focus and transparent for the mouse. Exclusive full-screen games do
//! not show such windows — only borderless windowed ones.

use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::c_void,
    sync::atomic::{AtomicBool, AtomicIsize, Ordering},
    time::Instant,
};

use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection, CreateFontW, DIB_RGB_COLORS, DT_END_ELLIPSIS,
            DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, GetDC, GetMonitorInfoW, HDC, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow, ReleaseDC, SelectObject,
            SetBkMode, SetTextColor, TRANSPARENT, ANTIALIASED_QUALITY, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, DEFAULT_PITCH, DRAW_TEXT_FORMAT,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, GetForegroundWindow, HWND_TOPMOST, PostMessageW, RegisterClassExW, SW_HIDE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
            SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WINDOW_EX_STYLE, WM_APP, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
            WS_EX_TRANSPARENT, WS_POPUP,
        },
    },
    core::{HSTRING, w},
};

use crate::{
    config,
    icon::{clampf, rgbf, sd_box},
};

/// A frame of the animation, posted by the pacer in step with the display.
const WM_OSD_FRAME: u32 = WM_APP + 41;
const OSD_W: f64 = 232.0; // DIP
const OSD_W_TEXT: f64 = 300.0;
const OSD_H: f64 = 58.0;
const HOLD_MS: u128 = 1100;
/// the battery stays longer: it is not an answer to a key just pressed and is easy to miss in a game
const HOLD_BATT_MS: u128 = 4000;
const FADE_MS: f64 = 220.0;

// ---------- frames in step with the monitor ----------
// While the hint is on screen a thread waits for each composition of the desktop (DwmFlush — the
// monitor's own rate: 60, 144, 240 Hz…) and asks the UI thread for the next frame. The rest of the
// time nothing runs.

static PACING: AtomicBool = AtomicBool::new(false);
/// a frame is asked for and not drawn yet: no queue of frames when the UI thread is busy
static PENDING: AtomicBool = AtomicBool::new(false);
static OSD_HWND: AtomicIsize = AtomicIsize::new(0);

fn pace_start(hwnd: HWND) {
    OSD_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
    if PACING.swap(true, Ordering::AcqRel) {
        return;
    }
    let started = std::thread::Builder::new().name("osd frames".into()).spawn(|| {
        while PACING.load(Ordering::Acquire) {
            // the next composition; without the compositor, about 60 Hz
            if unsafe { windows::Win32::Graphics::Dwm::DwmFlush() }.is_err() {
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
            if !PENDING.swap(true, Ordering::AcqRel) {
                let h = HWND(OSD_HWND.load(Ordering::Relaxed) as *mut c_void);
                if unsafe { PostMessageW(Some(h), WM_OSD_FRAME, WPARAM(0), LPARAM(0)) }.is_err() {
                    PENDING.store(false, Ordering::Release);
                }
            }
        }
    });
    if started.is_err() {
        PACING.store(false, Ordering::Release);
    }
}

fn pace_stop() {
    PACING.store(false, Ordering::Release);
}

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Balance,
    Text,
    Battery,
}

/// Icons of the Segoe MDL2 Assets font (in Windows 10 and 11).
pub const ICON_GAME: char = '\u{E7FC}';
pub const ICON_CHAT: char = '\u{E95B}';
pub const ICON_MIC: char = '\u{E720}';
pub const ICON_MIC_OFF: char = '\u{EC54}';
pub const ICON_SPEAKER: char = '\u{E7F5}';

/// Battery0..Battery9 (E850..E859) and Battery10 (E83F) of Segoe MDL2 Assets.
fn battery_glyph(level: i32) -> char {
    if level >= 100 { '\u{E83F}' } else { char::from_u32(0xE850 + (level.max(0) / 10) as u32).unwrap_or('\u{E850}') }
}

struct Line {
    s: String,
    col: [f64; 3],
    /// DT_LEFT 0, DT_CENTER 1, DT_RIGHT 2
    align: u32,
    /// drawn with the icon font
    icon: bool,
    /// extra distance from the edge it is aligned to, DIP
    inset: f64,
    /// room kept free at the other edge (for a line on the other side), DIP
    clear: f64,
}

impl Line {
    fn text(s: String, col: [f64; 3], align: u32, inset: f64) -> Line {
        Line { s, col, align, icon: false, inset, clear: 0.0 }
    }
    fn icon(c: char, col: [f64; 3], align: u32) -> Line {
        Line { s: c.to_string(), col, align, icon: true, inset: 0.0, clear: 0.0 }
    }
}

/// Room for an icon before the text, DIP.
const ICON_W: f64 = 26.0;

struct Osd {
    hwnd: HWND,
    shown_at: Instant,
    last: Option<Instant>,
    visible: bool,
    mode: Mode,
    text: String,
    icon: Option<char>,
    /// balance position -100..100: shown and target (shown catches up smoothly)
    cur: f64,
    tgt: f64,
    /// battery level, % (Mode::Battery; the text is the headset name)
    level: i32,
    /// the Nova colour steps of the battery (icon::battery_color)
    nova: bool,
    bg: Vec<f64>,
    pix: Vec<f64>,
    bg_key: String,
    txt: HashMap<String, Vec<u8>>,
    class_ok: bool,
}

thread_local! {
    static OSD: RefCell<Osd> = RefCell::new(Osd {
        hwnd: HWND::default(), shown_at: Instant::now(), last: None, visible: false, mode: Mode::Balance,
        text: String::new(), icon: None, cur: 0.0, tgt: 0.0, level: 0, nova: false, bg: Vec::new(), pix: Vec::new(), bg_key: String::new(),
        txt: HashMap::new(), class_ok: false,
    });
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if m == WM_OSD_FRAME {
        PENDING.store(false, Ordering::Release);
        crate::util::safely("osd", tick);
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(h, m, wp, lp) }
}

fn ensure() -> bool {
    OSD.with(|o| {
        let mut o = o.borrow_mut();
        if !o.hwnd.is_invalid() {
            return true;
        }
        unsafe {
            let inst = GetModuleHandleW(None).unwrap_or_default();
            if !o.class_ok {
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(wndproc),
                    hInstance: inst.into(),
                    lpszClassName: w!("ArcmixOSD"),
                    ..Default::default()
                };
                RegisterClassExW(&wc);
                o.class_ok = true;
            }
            let ex = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
            o.hwnd = CreateWindowExW(WINDOW_EX_STYLE(ex.0), w!("ArcmixOSD"), None, WS_POPUP, 0, 0, 1, 1, None, None, Some(inst.into()), None).unwrap_or_default();
            !o.hwnd.is_invalid()
        }
    })
}

/// Balance; the bar slides to the new value (like the slider in the window).
pub fn show_balance(pos: f64) {
    if !config::get().osd || crate::fly::visible() {
        return;
    }
    OSD.with(|o| {
        let mut o = o.borrow_mut();
        let prev = o.tgt;
        o.tgt = pos;
        if !o.visible || o.mode != Mode::Balance {
            o.cur = prev; // start from what was shown last
            if !o.visible && o.last.is_none_or(|l| l.elapsed().as_millis() > 3000) {
                o.cur = pos; // shown long ago — no animation from the past
            }
        }
        o.mode = Mode::Balance;
    });
    frame();
}

/// A short message after an icon that says what changed (the microphone, the output device).
pub fn show(icon: Option<char>, text: &str) {
    if !config::get().osd {
        return;
    }
    OSD.with(|o| {
        let mut o = o.borrow_mut();
        o.mode = Mode::Text;
        o.text = text.to_string();
        o.icon = icon;
    });
    frame();
}

/// The headset battery (low, charged), laid out like the balance: the name, the level and a bar of it
/// in the colours of the headset LED. Shown whatever the hint setting: its own switches ask for it.
pub fn show_battery(name: &str, level: i32, nova: bool) {
    OSD.with(|o| {
        let mut o = o.borrow_mut();
        o.mode = Mode::Battery;
        o.text = name.to_string();
        o.level = level.clamp(0, 100);
        o.nova = nova;
    });
    frame();
}

fn frame() {
    if !ensure() {
        return;
    }
    let (w, h) = render();
    present(w, h);
}

fn render() -> (usize, usize) {
    let sc = crate::dpi_scale();
    let mode = OSD.with(|o| o.borrow().mode);
    // messages (a device name) get more room than the balance; the battery is laid out like the balance
    let w = ((if mode == Mode::Text { OSD_W_TEXT } else { OSD_W }) * sc) as usize;
    let h = if mode == Mode::Text { (40.0 * sc) as usize } else { (OSD_H * sc) as usize };
    // every animation frame: the window theme (not the icon's own), read in place
    let ((ga, gb), light) = config::with(|c| (config::colors_of(&c.theme, c), crate::fly::is_light(c)));
    let (mut bg, mut fg, mut track) = ([0.09, 0.085, 0.10], [0.96, 0.95, 0.96], [0.25, 0.24, 0.27]);
    if light {
        (bg, fg, track) = ([0.98, 0.98, 0.99], [0.08, 0.08, 0.1], [0.84, 0.84, 0.87]);
    }
    let (mut col_g, mut col_c) = (rgbf(ga), rgbf(gb));
    if light {
        // darker colours on a light background, to be readable
        for i in 0..3 {
            col_g[i] *= 0.7;
            col_c[i] *= 0.7;
        }
    }
    let lines = OSD.with(|o| {
        let mut o = o.borrow_mut();
        // background with a frame — cached (drawn every frame of the animation)
        let key = format!("{w} {h} {light}");
        if o.bg_key != key {
            o.bg_key = key;
            o.txt.clear();
            let mut b = vec![0.0; w * h * 4];
            let rad = 12.0 * sc;
            for y in 0..h {
                for x in 0..w {
                    let d = sd_box(x as f64 + 0.5, y as f64 + 0.5, w as f64 / 2.0, h as f64 / 2.0, w as f64 / 2.0 - 1.0, h as f64 / 2.0 - 1.0, rad);
                    let a = clampf(0.5 - d, 0.0, 1.0);
                    if a <= 0.0 {
                        continue;
                    }
                    let al = a * 0.94;
                    let col = if d > -1.2 * sc { [bg[0] * 0.4 + track[0] * 0.6, bg[1] * 0.4 + track[1] * 0.6, bg[2] * 0.4 + track[2] * 0.6] } else { bg };
                    let i = (y * w + x) * 4;
                    b[i] = col[2] * al;
                    b[i + 1] = col[1] * al;
                    b[i + 2] = col[0] * al;
                    b[i + 3] = al;
                }
            }
            o.bg = b;
        }
        let bgc = o.bg.clone();
        o.pix.clear();
        o.pix.extend_from_slice(&bgc);
        if o.mode == Mode::Balance {
            let p = clampf(o.cur, -100.0, 100.0);
            let game = (100.0 - p.max(0.0)).round() as i32;
            let chat = (100.0 - (-p).max(0.0)).round() as i32;
            // the balance bar, as in the window: the game line runs from the left edge, chat from the
            // right; the quieter side steps back to its edge (even — the lines meet in the middle)
            let (bx0, bx1) = (16.0 * sc, w as f64 - 16.0 * sc);
            let by = h as f64 - 15.0 * sc;
            let bh = 2.5 * sc;
            let half = (bx1 - bx0) / 2.0;
            let gx = bx0 + half * (100.0 - p.max(0.0)) / 100.0;
            let cx = bx1 - half * (100.0 - (-p).max(0.0)) / 100.0;
            let pix = &mut o.pix;
            for y in (by - 8.0 * sc) as i64..(by + 8.0 * sc) as i64 {
                for x in (bx0 - 4.0) as i64..(bx1 + 4.0) as i64 {
                    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                    put(pix, w, h, x, y, track, clampf(0.5 - sd_box(px, py, (bx0 + bx1) / 2.0, by, half, bh, bh), 0.0, 1.0));
                    if gx - bx0 > 0.5 {
                        put(pix, w, h, x, y, col_g, clampf(0.5 - sd_box(px, py, (bx0 + gx) / 2.0, by, (gx - bx0) / 2.0, bh, bh), 0.0, 1.0));
                    }
                    if bx1 - cx > 0.5 {
                        put(pix, w, h, x, y, col_c, clampf(0.5 - sd_box(px, py, (cx + bx1) / 2.0, by, (bx1 - cx) / 2.0, bh, bh), 0.0, 1.0));
                    }
                }
            }
            // icons instead of the words: game on the left, chat on the right
            vec![
                Line::icon(ICON_GAME, col_g, 0),
                Line::text(format!("{game}%"), col_g, 0, ICON_W),
                Line::icon(ICON_CHAT, col_c, 2),
                Line::text(format!("{chat}%"), col_c, 2, ICON_W),
            ]
        } else if o.mode == Mode::Battery {
            let lvl = o.level;
            let col = crate::icon::battery_color(lvl, light, o.nova);
            // the level as a bar where the balance bar is, from the left edge
            let (bx0, bx1) = (16.0 * sc, w as f64 - 16.0 * sc);
            let by = h as f64 - 15.0 * sc;
            let bh = 2.5 * sc;
            let fx = bx0 + (bx1 - bx0) * lvl as f64 / 100.0;
            let pix = &mut o.pix;
            for y in (by - 8.0 * sc) as i64..(by + 8.0 * sc) as i64 {
                for x in (bx0 - 4.0) as i64..(bx1 + 4.0) as i64 {
                    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                    put(pix, w, h, x, y, track, clampf(0.5 - sd_box(px, py, (bx0 + bx1) / 2.0, by, (bx1 - bx0) / 2.0, bh, bh), 0.0, 1.0));
                    if fx - bx0 > 0.5 {
                        put(pix, w, h, x, y, col, clampf(0.5 - sd_box(px, py, (bx0 + fx) / 2.0, by, (fx - bx0) / 2.0, bh, bh), 0.0, 1.0));
                    }
                }
            }
            let mut name = Line::text(o.text.clone(), fg, 0, ICON_W);
            name.clear = 48.0; // the level on the right
            vec![Line::icon(battery_glyph(lvl), col, 0), name, Line::text(format!("{lvl}%"), col, 2, 0.0)]
        } else if let Some(ic) = o.icon {
            vec![Line::icon(ic, fg, 0), Line::text(o.text.clone(), fg, 0, ICON_W)]
        } else {
            vec![Line::text(o.text.clone(), fg, 1, 0.0)]
        }
    });
    for ln in lines {
        let m = text_mask(w, h, sc, mode, &ln);
        OSD.with(|o| {
            let pix = &mut o.borrow_mut().pix;
            for i in 0..w * h {
                let a = m[i * 4 + 1] as f64 / 255.0;
                if a > 0.0 {
                    put(pix, w, h, (i % w) as i64, (i / w) as i64, ln.col, a);
                }
            }
        });
    }
    (w, h)
}

fn put(pix: &mut [f64], w: usize, h: usize, x: i64, y: i64, col: [f64; 3], a: f64) {
    if a <= 0.0 || x < 0 || y < 0 || x as usize >= w || y as usize >= h {
        return;
    }
    let i = (y as usize * w + x as usize) * 4;
    let inv = 1.0 - a;
    pix[i] = col[2] * a + pix[i] * inv;
    pix[i + 1] = col[1] * a + pix[i + 1] * inv;
    pix[i + 2] = col[0] * a + pix[i + 2] * inv;
    pix[i + 3] = a + pix[i + 3] * inv;
}

fn dib(dc: HDC, w: usize, h: usize) -> Option<(windows::Win32::Graphics::Gdi::HBITMAP, *mut u8)> {
    let bih = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bmp = unsafe { CreateDIBSection(Some(dc), &bih, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    (!bits.is_null()).then_some((bmp, bits as *mut u8))
}

/// The window's own font (Onest, embedded) for the text of the indicator; Segoe UI if it cannot be added.
fn text_face() -> windows::core::PCWSTR {
    static ADDED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ok = *ADDED.get_or_init(|| {
        let f = crate::server::FONT_TTF;
        let mut n = 0u32;
        let h = unsafe { windows::Win32::Graphics::Gdi::AddFontMemResourceEx(f.as_ptr() as *const c_void, f.len() as u32, None, &mut n) };
        !h.is_invalid() && n > 0
    });
    if ok { w!("Onest") } else { w!("Segoe UI") }
}

/// Text coverage mask of a line (cached: during the animation the lines are mostly the same).
fn text_mask(w: usize, h: usize, sc: f64, mode: Mode, ln: &Line) -> Vec<u8> {
    let key = format!("{} {} {} {} {} {}", mode as u8, ln.align, ln.icon, ln.inset, ln.clear, ln.s);
    if let Some(m) = OSD.with(|o| o.borrow().txt.get(&key).cloned()) {
        return m;
    }
    let mut out = vec![0u8; w * h * 4];
    unsafe {
        let sdc = GetDC(None);
        let dc = CreateCompatibleDC(Some(sdc));
        let font = CreateFontW(
            ((if ln.icon { -16.0 } else { -14.0 }) * sc) as i32,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            DEFAULT_PITCH.0 as u32,
            if ln.icon { w!("Segoe MDL2 Assets") } else { text_face() },
        );
        if let Some((bmp, bits)) = dib(dc, w, h) {
            let old = SelectObject(dc, bmp.into());
            let old_f = SelectObject(dc, font.into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00FFFFFF));
            let edge = ((16.0 + ln.inset) * sc) as i32;
            let mut r = if mode == Mode::Text {
                RECT { left: edge, top: 0, right: w as i32 - edge, bottom: h as i32 }
            } else {
                RECT { left: edge, top: (7.0 * sc) as i32, right: w as i32 - edge, bottom: (29.0 * sc) as i32 }
            };
            let far = (ln.clear * sc) as i32;
            if ln.align == 2 {
                r.left += far;
            } else {
                r.right -= far;
            }
            let mut s: Vec<u16> = ln.s.encode_utf16().collect();
            DrawTextW(dc, &mut s, &mut r, DRAW_TEXT_FORMAT(ln.align) | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            std::ptr::copy_nonoverlapping(bits, out.as_mut_ptr(), w * h * 4);
            SelectObject(dc, old_f);
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
        }
        let _ = DeleteObject(font.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, sdc);
    }
    OSD.with(|o| {
        let mut o = o.borrow_mut();
        if o.txt.len() > 64 {
            o.txt.clear();
        }
        o.txt.insert(key, out.clone());
    });
    out
}

fn present(w: usize, h: usize) {
    let (hwnd, pix, was_visible) = OSD.with(|o| {
        let o = o.borrow();
        (o.hwnd, o.pix.clone(), o.visible)
    });
    unsafe {
        let sdc = GetDC(None);
        let dc = CreateCompatibleDC(Some(sdc));
        if let Some((bmp, bits)) = dib(dc, w, h) {
            let dst = std::slice::from_raw_parts_mut(bits, w * h * 4);
            for (d, s) in dst.iter_mut().zip(pix.iter()) {
                *d = (clampf(*s, 0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            let old = SelectObject(dc, bmp.into());
            // the chosen corner of the monitor with the active window (the game), inside the work area
            let mon = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let wk = mi.rcWork;
            let m = (16.0 * crate::dpi_scale()) as i32;
            let corner = config::with(|c| c.osd_corner.clone());
            let x = if corner.ends_with('l') { wk.left + m } else { wk.right - w as i32 - m };
            let y = if corner.starts_with('t') { wk.top + m } else { wk.bottom - h as i32 - m };
            let pt = POINT { x, y };
            let sz = SIZE { cx: w as i32, cy: h as i32 };
            let src = POINT::default();
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(hwnd, Some(sdc), Some(&pt), Some(&sz), Some(dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, sdc);
        if !was_visible {
            pace_start(hwnd);
        }
    }
    OSD.with(|o| {
        let mut o = o.borrow_mut();
        o.visible = true;
        o.shown_at = Instant::now();
        o.last = Some(Instant::now());
    });
}

/// The bar catches up with the target (exponent ~70 ms, as in the window), then a pause and a fade;
/// by the time passed since the previous frame, whatever the monitor's rate.
fn tick() {
    let animate = OSD.with(|o| {
        let mut o = o.borrow_mut();
        let now = Instant::now();
        let dt = o.last.map_or(16.0, |t| now.duration_since(t).as_secs_f64() * 1000.0).min(100.0);
        o.last = Some(now);
        if o.mode == Mode::Balance && (o.tgt - o.cur).abs() > 0.05 {
            o.cur += (o.tgt - o.cur) * (1.0 - (-dt / 70.0).exp());
            if (o.tgt - o.cur).abs() <= 0.05 {
                o.cur = o.tgt;
            }
            true
        } else {
            false
        }
    });
    if animate {
        frame();
        return;
    }
    let (hwnd, el, hold) = OSD.with(|o| {
        let o = o.borrow();
        (o.hwnd, o.shown_at.elapsed().as_millis(), if o.mode == Mode::Battery { HOLD_BATT_MS } else { HOLD_MS })
    });
    if el < hold {
        return;
    }
    let a = 255 - ((el - hold) as f64 / FADE_MS * 255.0) as i64;
    unsafe {
        if a <= 0 {
            pace_stop();
            let _ = ShowWindow(hwnd, SW_HIDE);
            OSD.with(|o| o.borrow_mut().visible = false);
            return;
        }
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: a as u8, AlphaFormat: AC_SRC_ALPHA as u8 };
        let _ = UpdateLayeredWindow(hwnd, None, None, None, None, None, COLORREF(0), Some(&blend), ULW_ALPHA);
    }
}

#[allow(dead_code)]
fn unused(_: HSTRING) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hint as drawn, for a look: ARCMIX_OSD_SHEET=<file.png> cargo test -- --ignored osd_sheet
    #[test]
    #[ignore]
    fn osd_sheet() {
        let Ok(path) = std::env::var("ARCMIX_OSD_SHEET") else { return };
        let mut frames = Vec::new();
        for (mode, icon, text, pos, level) in [
            (Mode::Balance, None, "", 30.0, 0),
            (Mode::Text, Some(ICON_MIC_OFF), "Off", 0.0, 0),
            (Mode::Text, Some(ICON_SPEAKER), "Headphones (LinkBuds Fit)", 0.0, 0),
            (Mode::Battery, None, "Arctis 7+", 0.0, 0),
            (Mode::Battery, None, "Arctis 7+", 0.0, 25),
            (Mode::Battery, None, "Arctis Nova Pro Wireless", 0.0, 100),
        ] {
            OSD.with(|o| {
                let mut o = o.borrow_mut();
                (o.mode, o.icon, o.text, o.cur, o.level) = (mode, icon, text.to_string(), pos, level);
            });
            let (w, h) = render();
            frames.push((w, h, OSD.with(|o| o.borrow().pix.clone())));
        }
        let w = frames.iter().map(|f| f.0).max().unwrap() + 16;
        let h: usize = frames.iter().map(|f| f.1 + 8).sum::<usize>() + 8;
        let mut img = vec![0u8; w * h * 4];
        for p in img.chunks_mut(4) {
            p.copy_from_slice(&[0x30, 0x30, 0x34, 255]);
        }
        let mut y0 = 8;
        for (fw, fh, pix) in frames {
            for y in 0..fh {
                for x in 0..fw {
                    let i = (y * fw + x) * 4;
                    let o = ((y0 + y) * w + 8 + x) * 4;
                    let a = pix[i + 3];
                    for c in 0..3 {
                        // premultiplied BGR over the backdrop
                        img[o + c] = ((pix[i + 2 - c] + img[o + c] as f64 / 255.0 * (1.0 - a)) * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
            y0 += fh + 8;
        }
        std::fs::write(path, crate::png::encode(w as u32, h as u32, &img)).unwrap();
    }
}
