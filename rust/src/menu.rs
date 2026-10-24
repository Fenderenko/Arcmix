//! The tray menu drawn in the app's own style (rounded, the window theme, icons) instead of the
//! system menu: a layered popup window at the cursor, with hover, the keyboard (↑ ↓ Enter Esc) and
//! closing when it loses focus.

use std::{cell::RefCell, ffi::c_void};

use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection, CreateFontW,
            DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DRAW_TEXT_FORMAT, DT_END_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, GetDC, GetMonitorInfoW, HBITMAP,
            HDC, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, OUT_DEFAULT_PRECIS, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SPACE, VK_UP},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, HWND_TOPMOST, IDC_ARROW, LoadCursorW, RegisterClassExW, SW_SHOW, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow,
                SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WA_INACTIVE, WINDOW_EX_STYLE, WM_ACTIVATE, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WNDCLASSEXW, WS_EX_LAYERED,
                WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
    core::w,
};

use crate::{
    config,
    icon::{clampf, rgbf, sd_box},
};

const WM_MOUSELEAVE: u32 = 0x02A3;

/// One line of the menu.
pub enum Entry {
    /// the title line: the app logo, its name and (muted, on the right) the version
    Title(String, String),
    /// a muted caption over a group of items
    Caption(String),
    Separator,
    /// a choice: id handed to the callback, icon (Segoe MDL2 Assets), text, a muted hint on the right
    /// (a hotkey), red on hover (quit)
    Item { id: u32, icon: char, text: String, hint: String, danger: bool },
}

// layout, DIP
const W: f64 = 248.0;
/// room for the shadow around the card
const SHADOW: f64 = 10.0;
const PAD: f64 = 6.0;
const RADIUS: f64 = 10.0;

fn height(e: &Entry) -> f64 {
    match e {
        Entry::Title(..) => 34.0,
        Entry::Caption(_) => 24.0,
        Entry::Separator => 9.0,
        Entry::Item { .. } => 34.0,
    }
}

struct Menu {
    hwnd: HWND,
    entries: Vec<Entry>,
    /// top of every entry in pixels, inside the card
    tops: Vec<f64>,
    hover: Option<usize>,
    on_pick: fn(u32),
    sc: f64,
    w: usize,
    h: usize,
    /// text coverage: normal and muted
    main: Vec<u8>,
    dim: Vec<u8>,
    tracking: bool,
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
    static CLASS: RefCell<bool> = const { RefCell::new(false) };
}

/// (UI thread) Opens the menu at the cursor; `on_pick` gets the id of the chosen item.
pub fn show(entries: Vec<Entry>, on_pick: fn(u32)) {
    close();
    let sc = crate::dpi_scale();
    let mut tops = Vec::with_capacity(entries.len());
    let mut y = (SHADOW + PAD) * sc;
    for e in &entries {
        tops.push(y);
        y += height(e) * sc;
    }
    let (w, h) = (((W + 2.0 * SHADOW) * sc).round() as usize, (y + (PAD + SHADOW) * sc).round() as usize);
    let hwnd = unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        CLASS.with(|c| {
            if !*c.borrow() {
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(wndproc),
                    hInstance: inst.into(),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    lpszClassName: w!("ArcmixMenu"),
                    ..Default::default()
                };
                RegisterClassExW(&wc);
                *c.borrow_mut() = true;
            }
        });
        let ex = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
        CreateWindowExW(WINDOW_EX_STYLE(ex.0), w!("ArcmixMenu"), None, WS_POPUP, 0, 0, 1, 1, None, None, Some(inst.into()), None).unwrap_or_default()
    };
    if hwnd.is_invalid() {
        return;
    }
    let (main, dim) = text_layers(&entries, &tops, sc, w, h);
    MENU.with(|m| *m.borrow_mut() = Some(Menu { hwnd, entries, tops, hover: None, on_pick, sc, w, h, main, dim, tracking: false }));
    // like the system menu: its lower left corner at the cursor, kept on the monitor
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut mi);
    }
    let wk = mi.rcWork;
    let m = (SHADOW * sc) as i32;
    let x = (pt.x - m).clamp(wk.left - m, wk.right - w as i32 + m);
    let y = (pt.y - h as i32 + m).clamp(wk.top - m, wk.bottom - h as i32 + m);
    present(POINT { x, y });
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE);
        let _ = SetForegroundWindow(hwnd);
    }
}

fn close() {
    if let Some(m) = MENU.with(|m| m.borrow_mut().take()) {
        unsafe {
            let _ = DestroyWindow(m.hwnd);
        }
    }
}

fn pick(i: usize) {
    let pick = MENU.with(|m| {
        let m = m.borrow();
        let m = m.as_ref()?;
        match m.entries.get(i)? {
            Entry::Item { id, .. } => Some((m.on_pick, *id)),
            _ => None,
        }
    });
    if let Some((f, id)) = pick {
        close();
        f(id);
    }
}

fn selectable(m: &Menu) -> Vec<usize> {
    m.entries.iter().enumerate().filter(|(_, e)| matches!(e, Entry::Item { .. })).map(|(i, _)| i).collect()
}

fn set_hover(i: Option<usize>) {
    let changed = MENU.with(|m| {
        let mut m = m.borrow_mut();
        let Some(m) = m.as_mut() else { return false };
        let ch = m.hover != i;
        m.hover = i;
        ch
    });
    if changed {
        present_in_place();
    }
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f64;
            let x = (lp.0 & 0xFFFF) as i16 as f64;
            let (hit, track) = MENU.with(|m| {
                let mut m = m.borrow_mut();
                let Some(m) = m.as_mut() else { return (None, false) };
                let (l, r) = (SHADOW * m.sc, m.w as f64 - SHADOW * m.sc);
                let hit = (x >= l && x < r)
                    .then(|| m.entries.iter().zip(&m.tops).position(|(e, t)| matches!(e, Entry::Item { .. }) && y >= *t && y < t + height(e) * m.sc))
                    .flatten();
                let track = !m.tracking;
                m.tracking = true;
                (hit, track)
            });
            if track {
                let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                unsafe {
                    let _ = TrackMouseEvent(&mut tme);
                }
            }
            set_hover(hit);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            MENU.with(|m| {
                if let Some(m) = m.borrow_mut().as_mut() {
                    m.tracking = false;
                }
            });
            set_hover(None);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if let Some(i) = MENU.with(|m| m.borrow().as_ref().and_then(|m| m.hover)) {
                pick(i);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let k = wp.0 as u16;
            if k == VK_ESCAPE.0 {
                close();
            } else if k == VK_RETURN.0 || k == VK_SPACE.0 {
                if let Some(i) = MENU.with(|m| m.borrow().as_ref().and_then(|m| m.hover)) {
                    pick(i);
                }
            } else if k == VK_UP.0 || k == VK_DOWN.0 {
                let next = MENU.with(|m| {
                    let m = m.borrow();
                    let m = m.as_ref()?;
                    let sel = selectable(m);
                    let n = sel.len() as i32;
                    let at = m.hover.and_then(|h| sel.iter().position(|s| *s == h)).map_or(if k == VK_UP.0 { n } else { -1 }, |p| p as i32);
                    let step = if k == VK_UP.0 { -1 } else { 1 };
                    sel.get((at + step).rem_euclid(n.max(1)) as usize).copied()
                });
                if next.is_some() {
                    set_hover(next);
                }
            }
            LRESULT(0)
        }
        WM_ACTIVATE => {
            if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
                close();
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(h, msg, wp, lp) },
    }
}

/// Theme colours: card, text, muted text, frame, hover, accent — those of the window (sent by the page
/// whenever the theme changes), or a neutral set until it has.
fn colors() -> ([f64; 3], [f64; 3], [f64; 3], [f64; 3], [f64; 3], [f64; 3]) {
    let ui: Vec<[f64; 3]> = config::with(|c| c.ui_colors.iter().filter_map(|x| config::parse_hex(x)).map(rgbf).collect());
    if ui.len() == 6 {
        return (ui[0], ui[1], ui[2], ui[3], ui[4], ui[5]);
    }
    let ((ga, _), light) = config::with(|c| (config::colors_of(&c.theme, c), crate::fly::is_light(c)));
    let mut acc = rgbf(ga);
    if light {
        for v in acc.iter_mut() {
            *v *= 0.7;
        }
        ([0.98, 0.98, 0.99], [0.08, 0.08, 0.10], [0.42, 0.42, 0.46], [0.84, 0.84, 0.87], [0.92, 0.92, 0.94], acc)
    } else {
        ([0.11, 0.105, 0.125], [0.95, 0.94, 0.96], [0.58, 0.56, 0.60], [0.24, 0.23, 0.26], [0.19, 0.18, 0.21], acc)
    }
}

fn render(m: &Menu) -> Vec<u8> {
    let (w, h, sc) = (m.w, m.h, m.sc);
    let (bg, fg, dimc, frame, hov, _acc) = colors();
    let red = [1.0, 0.42, 0.40];
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let (hw, hh) = (cx - SHADOW * sc, cy - SHADOW * sc);
    let mut out = vec![0u8; w * h * 4];
    // the hovered row: its rounded highlight and the colour of its text
    let hover = m.hover.map(|i| (m.tops[i], m.tops[i] + height(&m.entries[i]) * sc, matches!(m.entries[i], Entry::Item { danger: true, .. })));
    // separators: a thin line across the card, inside the padding
    let seps: Vec<f64> = m.entries.iter().zip(&m.tops).filter(|(e, _)| matches!(e, Entry::Separator)).map(|(e, t)| t + height(e) * sc / 2.0).collect();
    let (sx0, sx1) = (cx - hw + (PAD + 6.0) * sc, cx + hw - (PAD + 6.0) * sc);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let d = sd_box(px, py, cx, cy, hw, hh, RADIUS * sc);
            // premultiplied BGRA
            let (mut col, mut a) = ([0.0; 3], 0.0);
            if d > 0.0 {
                // soft shadow outside the card
                let t = clampf(1.0 - d / (SHADOW * sc), 0.0, 1.0);
                a = 0.30 * t * t;
            }
            let cov = clampf(0.5 - d, 0.0, 1.0);
            if cov > 0.0 {
                let mut c = if d > -1.2 * sc { frame } else { bg };
                if px > sx0 && px < sx1 && seps.iter().any(|s| (py - s).abs() < 0.5 * sc.max(1.0)) {
                    c = frame;
                }
                if let Some((t0, t1, _)) = hover {
                    let r = sd_box(px, py, cx, (t0 + t1) / 2.0, hw - PAD * sc, (t1 - t0) / 2.0, 6.0 * sc);
                    let k = clampf(0.5 - r, 0.0, 1.0);
                    c = mix(c, hov, k);
                }
                col = c;
                a = cov + a * (1.0 - cov);
                col = [col[0] * cov, col[1] * cov, col[2] * cov];
            }
            let i = (y * w + x) * 4;
            let tm = m.main[i + 1] as f64 / 255.0 * cov;
            let td = m.dim[i + 1] as f64 / 255.0 * cov;
            let mut text = fg;
            if let Some((t0, t1, danger)) = hover {
                if py >= t0 && py < t1 && danger {
                    text = red;
                }
            }
            for c in 0..3 {
                let v = col[c] * (1.0 - tm - td).max(0.0) + text[c] * tm + dimc[c] * td;
                out[i + 2 - c] = (clampf(v, 0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            out[i + 3] = (clampf(a, 0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    // the app logo at the start of the title line, as in the tray (straight alpha over the card)
    if let Some(t) = m.entries.iter().position(|e| matches!(e, Entry::Title(..))) {
        let n = (18.0 * sc).round() as usize;
        let logo = crate::icon::render(&crate::icon::IconParams {
            size: n,
            pos: 0.0,
            b: 1.0,
            bat: 0.0,
            level: 0,
            nova: false,
            light: false,
            mic: 0,
            cg: crate::icon::GAME,
            cc: crate::icon::CHAT,
        });
        let x0 = ((SHADOW + PAD + 10.0) * sc) as usize;
        let y0 = (m.tops[t] + (height(&m.entries[t]) * sc - n as f64) / 2.0) as usize;
        for y in 0..n {
            for x in 0..n {
                let s = (y * n + x) * 4;
                let a = logo[s + 3] as f64 / 255.0;
                if a <= 0.0 || y0 + y >= h || x0 + x >= w {
                    continue;
                }
                let o = ((y0 + y) * w + x0 + x) * 4;
                for c in 0..3 {
                    out[o + c] = (logo[s + c] as f64 * a + out[o + c] as f64 * (1.0 - a)).round() as u8;
                }
            }
        }
    }
    out
}

fn mix(a: [f64; 3], b: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k]
}

/// Text and icons drawn once per opening: white coverage masks, normal and muted.
fn text_layers(entries: &[Entry], tops: &[f64], sc: f64, w: usize, h: usize) -> (Vec<u8>, Vec<u8>) {
    let (l, r) = (((SHADOW + PAD + 10.0) * sc) as i32, (w as f64 - (SHADOW + PAD + 10.0) * sc) as i32);
    let text_l = l + (26.0 * sc) as i32;
    let row = |e: &Entry, t: f64| RECT { left: l, top: t as i32, right: r, bottom: (t + height(e) * sc) as i32 };
    let (mut main, mut dim): (Vec<Draw>, Vec<Draw>) = (Vec::new(), Vec::new());
    for (e, t) in entries.iter().zip(tops) {
        let rc = row(e, *t);
        match e {
            Entry::Title(name, ver) => {
                // the logo is drawn in its place by render(); the name follows it
                main.push(Draw { s: name.clone(), rc: RECT { left: text_l, ..rc }, font: Font::Bold, align: 0 });
                dim.push(Draw { s: ver.clone(), rc, font: Font::Small, align: 2 });
            }
            Entry::Caption(s) => dim.push(Draw { s: s.clone(), rc, font: Font::Small, align: 0 }),
            Entry::Separator => {}
            Entry::Item { icon, text, hint, .. } => {
                main.push(Draw { s: icon.to_string(), rc, font: Font::Icon, align: 0 });
                main.push(Draw { s: text.clone(), rc: RECT { left: text_l, ..rc }, font: Font::Text, align: 0 });
                if !hint.is_empty() {
                    dim.push(Draw { s: hint.clone(), rc, font: Font::Small, align: 2 });
                }
            }
        }
    }
    (draw_layer(&main, sc, w, h), draw_layer(&dim, sc, w, h))
}

#[derive(Clone, Copy, PartialEq)]
enum Font {
    Text,
    Bold,
    Small,
    Icon,
}

struct Draw {
    s: String,
    rc: RECT,
    font: Font,
    /// DT_LEFT 0, DT_RIGHT 2
    align: u32,
}

fn dib(dc: HDC, w: usize, h: usize) -> Option<(HBITMAP, *mut u8)> {
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

fn draw_layer(draws: &[Draw], sc: f64, w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    unsafe {
        let sdc = GetDC(None);
        let dc = CreateCompatibleDC(Some(sdc));
        let font = |f: Font| {
            let (size, weight, face) = match f {
                Font::Text => (-13.5, 400, w!("Segoe UI")),
                Font::Bold => (-13.5, 700, w!("Segoe UI")),
                Font::Small => (-12.0, 400, w!("Segoe UI")),
                Font::Icon => (-15.0, 400, w!("Segoe MDL2 Assets")),
            };
            CreateFontW((size * sc) as i32, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, DEFAULT_PITCH.0 as u32, face)
        };
        let fonts = [font(Font::Text), font(Font::Bold), font(Font::Small), font(Font::Icon)];
        if let Some((bmp, bits)) = dib(dc, w, h) {
            let old = SelectObject(dc, bmp.into());
            let old_f = SelectObject(dc, fonts[0].into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00FFFFFF));
            for d in draws {
                SelectObject(dc, fonts[d.font as usize].into());
                let mut rc = d.rc;
                let mut s: Vec<u16> = d.s.encode_utf16().collect();
                DrawTextW(dc, &mut s, &mut rc, DRAW_TEXT_FORMAT(d.align) | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            }
            std::ptr::copy_nonoverlapping(bits, out.as_mut_ptr(), w * h * 4);
            SelectObject(dc, old_f);
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
        }
        for f in fonts {
            let _ = DeleteObject(f.into());
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, sdc);
    }
    out
}

/// Draws the menu at a screen position.
fn present(at: POINT) {
    let Some((hwnd, w, h, px)) = MENU.with(|m| m.borrow().as_ref().map(|m| (m.hwnd, m.w, m.h, render(m)))) else { return };
    update(hwnd, w, h, &px, Some(at));
}

/// Draws the menu again where it is (hover changed).
fn present_in_place() {
    let Some((hwnd, w, h, px)) = MENU.with(|m| m.borrow().as_ref().map(|m| (m.hwnd, m.w, m.h, render(m)))) else { return };
    update(hwnd, w, h, &px, None);
}

fn update(hwnd: HWND, w: usize, h: usize, px: &[u8], at: Option<POINT>) {
    unsafe {
        let sdc = GetDC(None);
        let dc = CreateCompatibleDC(Some(sdc));
        if let Some((bmp, bits)) = dib(dc, w, h) {
            std::ptr::copy_nonoverlapping(px.as_ptr(), bits, w * h * 4);
            let old = SelectObject(dc, bmp.into());
            let sz = SIZE { cx: w as i32, cy: h as i32 };
            let src = POINT::default();
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(hwnd, Some(sdc), at.as_ref().map(|p| p as *const POINT), Some(&sz), Some(dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, sdc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The menu as drawn, with the second item hovered: ARCMIX_MENU_SHEET=<file.png> cargo test -- --ignored menu_sheet
    #[test]
    #[ignore]
    fn menu_sheet() {
        let Ok(path) = std::env::var("ARCMIX_MENU_SHEET") else { return };
        // ARCMIX_MENU_COLORS: the six theme colours as the page sends them, comma-separated
        if let Ok(cols) = std::env::var("ARCMIX_MENU_COLORS") {
            let mut c = config::Config::default();
            c.ui_colors = cols.split(',').map(String::from).collect();
            config::replace(c);
        }
        let entries = vec![
            Entry::Title("Arcmix".into(), "1.0.3".into()),
            Entry::Separator,
            Entry::Item { id: 1, icon: '\u{E8A7}', text: "Open window".into(), hint: String::new(), danger: false },
            Entry::Item { id: 2, icon: '\u{E9E9}', text: "Balance: equal".into(), hint: "Ctrl+Shift+↓".into(), danger: false },
            Entry::Caption("Profile".into()),
            Entry::Item { id: 3, icon: '\u{E77B}', text: "Night".into(), hint: String::new(), danger: false },
            Entry::Separator,
            Entry::Item { id: 4, icon: '\u{E7E8}', text: "Quit".into(), hint: String::new(), danger: true },
        ];
        let mut sheet = Vec::new();
        let mut size = (0, 0);
        for hover in [Some(3), Some(7)] {
            let sc = 1.0;
            let mut tops = Vec::new();
            let mut y = (SHADOW + PAD) * sc;
            for e in &entries {
                tops.push(y);
                y += height(e) * sc;
            }
            let (w, h) = (((W + 2.0 * SHADOW) * sc).round() as usize, (y + (PAD + SHADOW) * sc).round() as usize);
            let (main, dim) = text_layers(&entries, &tops, sc, w, h);
            let m = Menu { hwnd: HWND::default(), entries: Vec::new(), tops: tops.clone(), hover, on_pick: |_| {}, sc, w, h, main, dim, tracking: false };
            let m = Menu { entries: entries.iter().map(clone_entry).collect(), ..m };
            sheet.push(render(&m));
            size = (w, h);
        }
        let (w, h) = size;
        let mut img = vec![0u8; w * 2 * h * 4];
        for (k, px) in sheet.iter().enumerate() {
            for y in 0..h {
                for x in 0..w {
                    let i = (y * w + x) * 4;
                    let o = (y * w * 2 + k * w + x) * 4;
                    let a = px[i + 3] as f64 / 255.0;
                    for c in 0..3 {
                        img[o + c] = (px[i + 2 - c] as f64 + 0x55 as f64 * (1.0 - a)).round().min(255.0) as u8;
                    }
                    img[o + 3] = 255;
                }
            }
        }
        std::fs::write(path, crate::png::encode((w * 2) as u32, h as u32, &img)).unwrap();
    }

    fn clone_entry(e: &Entry) -> Entry {
        match e {
            Entry::Title(a, b) => Entry::Title(a.clone(), b.clone()),
            Entry::Caption(s) => Entry::Caption(s.clone()),
            Entry::Separator => Entry::Separator,
            Entry::Item { id, icon, text, hint, danger } => Entry::Item { id: *id, icon: *icon, text: text.clone(), hint: hint.clone(), danger: *danger },
        }
    }
}
