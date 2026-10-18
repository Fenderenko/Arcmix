//! On-screen hint (OSD): briefly shows the game / chat balance when ChatMix turns (headset dial,
//! hotkeys, mouse wheel over the icon) and the microphone state on the hotkey. A layered window
//! above everything, without focus and transparent for the mouse. Exclusive full-screen games do
//! not show such windows — only borderless windowed ones.

use std::{cell::RefCell, collections::HashMap, ffi::c_void, time::Instant};

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
            CreateWindowExW, DefWindowProcW, GetForegroundWindow, HWND_TOPMOST, KillTimer, RegisterClassExW, SW_HIDE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetTimer,
            SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WINDOW_EX_STYLE, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
            WS_EX_TRANSPARENT, WS_POPUP,
        },
    },
    core::{HSTRING, w},
};

use crate::{
    config,
    i18n::t,
    icon::{clampf, rgbf, sd_box},
};

const TIMER_OSD: usize = 7;
const OSD_W: f64 = 232.0; // DIP
const OSD_H: f64 = 58.0;
const HOLD_MS: u128 = 1100;
const FADE_MS: f64 = 220.0;
const TICK_MS: u32 = 15;

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Balance,
    Text,
}

struct Line {
    s: String,
    col: [f64; 3],
    /// DT_LEFT 0, DT_CENTER 1, DT_RIGHT 2
    align: u32,
}

struct Osd {
    hwnd: HWND,
    shown_at: Instant,
    last: Option<Instant>,
    visible: bool,
    mode: Mode,
    text: String,
    /// balance position -100..100: shown and target (shown catches up smoothly)
    cur: f64,
    tgt: f64,
    bg: Vec<f64>,
    pix: Vec<f64>,
    bg_key: String,
    txt: HashMap<String, Vec<u8>>,
    class_ok: bool,
}

thread_local! {
    static OSD: RefCell<Osd> = RefCell::new(Osd {
        hwnd: HWND::default(), shown_at: Instant::now(), last: None, visible: false, mode: Mode::Balance,
        text: String::new(), cur: 0.0, tgt: 0.0, bg: Vec::new(), pix: Vec::new(), bg_key: String::new(),
        txt: HashMap::new(), class_ok: false,
    });
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if m == WM_TIMER && wp.0 == TIMER_OSD {
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

/// A short message (e.g. about the microphone).
pub fn show_text(text: &str) {
    if !config::get().osd {
        return;
    }
    OSD.with(|o| {
        let mut o = o.borrow_mut();
        o.mode = Mode::Text;
        o.text = text.to_string();
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
    let w = (OSD_W * sc) as usize;
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
            vec![Line { s: format!("{}  {game}%", t("Гра")), col: col_g, align: 0 }, Line { s: format!("{}  {chat}%", t("Чат")), col: col_c, align: 2 }]
        } else {
            vec![Line { s: o.text.clone(), col: fg, align: 1 }]
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

/// Text coverage mask of a line (cached: during the animation the lines are mostly the same).
fn text_mask(w: usize, h: usize, sc: f64, mode: Mode, ln: &Line) -> Vec<u8> {
    let key = format!("{} {} {}", mode as u8, ln.align, ln.s);
    if let Some(m) = OSD.with(|o| o.borrow().txt.get(&key).cloned()) {
        return m;
    }
    let mut out = vec![0u8; w * h * 4];
    unsafe {
        let sdc = GetDC(None);
        let dc = CreateCompatibleDC(Some(sdc));
        let font = CreateFontW(
            (-14.0 * sc) as i32,
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Segoe UI"),
        );
        if let Some((bmp, bits)) = dib(dc, w, h) {
            let old = SelectObject(dc, bmp.into());
            let old_f = SelectObject(dc, font.into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00FFFFFF));
            let mut r = if mode == Mode::Text {
                RECT { left: (16.0 * sc) as i32, top: 0, right: w as i32 - (16.0 * sc) as i32, bottom: h as i32 }
            } else {
                RECT { left: (16.0 * sc) as i32, top: (7.0 * sc) as i32, right: w as i32 - (16.0 * sc) as i32, bottom: (29.0 * sc) as i32 }
            };
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
            // lower right corner of the monitor with the active window (the game), above the taskbar
            let mon = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let wk = mi.rcWork;
            let m = (16.0 * crate::dpi_scale()) as i32;
            let pt = POINT { x: wk.right - w as i32 - m, y: wk.bottom - h as i32 - m };
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
            SetTimer(Some(hwnd), TIMER_OSD, TICK_MS, None);
        }
    }
    OSD.with(|o| {
        let mut o = o.borrow_mut();
        o.visible = true;
        o.shown_at = Instant::now();
        o.last = Some(Instant::now());
    });
}

/// The bar catches up with the target (exponent ~70 ms, as in the window), then a pause and a fade.
fn tick() {
    let animate = OSD.with(|o| {
        let mut o = o.borrow_mut();
        if o.mode == Mode::Balance && (o.tgt - o.cur).abs() > 0.05 {
            o.cur += (o.tgt - o.cur) * (1.0 - (-(TICK_MS as f64) / 70.0).exp());
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
    let (hwnd, el) = OSD.with(|o| {
        let o = o.borrow();
        (o.hwnd, o.shown_at.elapsed().as_millis())
    });
    if el < HOLD_MS {
        return;
    }
    let a = 255 - ((el - HOLD_MS) as f64 / FADE_MS * 255.0) as i64;
    unsafe {
        if a <= 0 {
            let _ = KillTimer(Some(hwnd), TIMER_OSD);
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
