//! "As in Windows" theme: the accent colour and light or dark mode from Windows personalization.
//! Follows Windows (WM_SETTINGCHANGE / DWM colour change). The window is always opaque.

use std::sync::{
    Mutex,
    atomic::{AtomicU8, Ordering},
};

use serde::Serialize;

use crate::util::reg_open;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct WinThemeInfo {
    /// accent shades (AccentPalette), lightest to darkest
    pub pal: [String; 8],
    /// light app mode
    pub light: bool,
}

static CUR: Mutex<Option<WinThemeInfo>> = Mutex::new(None);

pub fn read() -> WinThemeInfo {
    let mut w = WinThemeInfo::default();
    // AccentPalette: 8 colours of 4 bytes (R, G, B, A) — how Windows draws the accent itself
    let pal = reg_open(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent")
        .and_then(|k| k.binary("AccentPalette", 32))
        .filter(|b| b.len() >= 28);
    if let Some(p) = pal {
        for i in 0..8 {
            let o = (i * 4).min(p.len() - 4);
            w.pal[i] = format!("#{:02X}{:02X}{:02X}", p[o], p[o + 1], p[o + 2]);
        }
    } else {
        // older Windows: only the main colour (DWM AccentColor, 0xAABBGGRR) — shades made here
        let mut base = [0x00u8, 0x78, 0xD4];
        if let Some(v) = reg_open(r"Software\Microsoft\Windows\DWM").and_then(|k| k.dword("AccentColor")) {
            base = [v as u8, (v >> 8) as u8, (v >> 16) as u8];
        }
        for (i, l) in [0.86, 0.72, 0.58, -1.0, 0.36, 0.26, 0.17, 0.45].iter().enumerate() {
            let mut c = base;
            if *l >= 0.0 {
                let (h, s, _) = rgb_hsl(base);
                c = hsl_rgb(h, s, *l);
            }
            w.pal[i] = format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
        }
    }
    w.light = reg_open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.dword("AppsUseLightTheme"))
        == Some(1);
    w
}

/// The last read Windows look (read at start and when Windows changes it).
pub fn get() -> WinThemeInfo {
    with(|w| w.clone())
}

fn with<R>(f: impl FnOnce(&WinThemeInfo) -> R) -> R {
    let mut g = CUR.lock().unwrap();
    f(g.get_or_insert_with(read))
}

/// Light app mode (without copying the palette: called on every animation frame).
pub fn light() -> bool {
    with(|w| w.light)
}

/// Re-reads the Windows look; true when it changed.
pub fn refresh() -> bool {
    let w = read();
    let mut g = CUR.lock().unwrap();
    let changed = g.as_ref() != Some(&w);
    *g = Some(w);
    changed
}

/// The taskbar mode: 0 — not read yet, 1 — dark, 2 — light.
static SYS_LIGHT: AtomicU8 = AtomicU8::new(0);

/// Whether the taskbar is light (SystemUsesLightTheme); read once, then on Windows' change reports.
pub fn system_light() -> bool {
    match SYS_LIGHT.load(Ordering::Relaxed) {
        0 => refresh_system_light(),
        v => v == 2,
    }
}

/// Reads the taskbar mode again (Windows reported a settings change).
pub fn refresh_system_light() -> bool {
    let l = reg_open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize").and_then(|k| k.dword("SystemUsesLightTheme")) == Some(1);
    SYS_LIGHT.store(if l { 2 } else { 1 }, Ordering::Relaxed);
    l
}

/// Game colour — the Windows accent (a light shade for a dark background), chat colour — the opposite
/// hue, muted. The window (web/index.html) uses the same rule.
pub fn colors() -> ([u8; 3], [u8; 3]) {
    let (a, base) = with(|w| (crate::config::parse_hex(&w.pal[1]), crate::config::parse_hex(&w.pal[3])));
    let a = a.unwrap_or([0x4C, 0xC2, 0xFF]);
    let (h, _, _) = rgb_hsl(base.unwrap_or([0x00, 0x78, 0xD4]));
    (a, hsl_rgb((h + 0.5) % 1.0, 0.5, 0.72))
}

pub fn rgb_hsl(c: [u8; 3]) -> (f64, f64, f64) {
    let (r, g, b) = (c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0);
    let (mx, mn) = (r.max(g.max(b)), r.min(g.min(b)));
    let l = (mx + mn) / 2.0;
    let d = mx - mn;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
    let h = if mx == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

pub fn hsl_rgb(h: f64, s: f64, l: f64) -> [u8; 3] {
    let f = |p: f64, q: f64, t: f64| {
        let t = (t + 1.0) % 1.0;
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let (r, g, b) = if s == 0.0 {
        (l, l, l)
    } else {
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        (f(p, q, h + 1.0 / 3.0), f(p, q, h), f(p, q, h - 1.0 / 3.0))
    };
    let u = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [u(r), u(g), u(b)]
}
