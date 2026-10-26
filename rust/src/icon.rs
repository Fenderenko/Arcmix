//! Tray icon frames: the app mark in balance, two capsules off it, the battery number morph and the
//! microphone badge, in one colour. Signed distance fields on a 32x32 grid, rendered at the system icon
//! size. Also the mark alone (the tray menu).

use std::sync::LazyLock;

use windows::Win32::{
    Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, GetDC, ReleaseDC},
    UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO},
};

// The app mark (as tools/mkicon.py and the window header): a concave triangle, the "A" of Arcmix —
// straight sides from the apex and a bottom edge bowed up (a quadratic curve). On a 24-unit box: apex
// (12, 3), wing tips (3, 20) and (21, 20), control point of the bottom (12, 14). Kept in units of the
// side: wing inset, apex, wing tips, control point.
const MARK: (f64, f64, f64, f64) = (3.0 / 24.0, 3.0 / 24.0, 20.0 / 24.0, 14.0 / 24.0);
/// Scale of the mark in the tray icon and the menu: it fills the small icon as the system ones do.
const MARK_Z: f64 = 1.15;
/// One colour, as the system tray icons: white on a dark taskbar, near black on a light one.
const INK_DARK: [f64; 3] = [1.0, 1.0, 1.0];
const INK_LIGHT: [f64; 3] = [0.09, 0.09, 0.09];
/// Relative luminance of the taskbar (Windows 11: #1C1C1C dark, #EEEEEE light).
const BAR_DARK: f64 = 0.011;
const BAR_LIGHT: f64 = 0.855;

pub struct IconParams {
    pub size: usize,
    /// -100..100 (plus = quieter game, louder chat)
    pub pos: f64,
    /// 0..1: the mark (balance) versus two capsules
    pub b: f64,
    /// 0..1: progress of the mark → battery number morph (linear in time)
    pub bat: f64,
    /// battery, percent
    pub level: i32,
    /// the battery LED steps of the Arctis Nova line (otherwise of the older Arctis)
    pub nova: bool,
    /// light taskbar
    pub light: bool,
    /// microphone badge: 0 none, 1 muted (red), 2 talking (green)
    pub mic: i32,
    /// the game and chat colours of the theme (the capsules off balance)
    pub cg: [u8; 3],
    pub cc: [u8; 3],
}

pub fn clampf(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

fn lerpf(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn smoother(t: f64) -> f64 {
    let t = clampf(t, 0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Signed distance to a rounded rectangle.
pub fn sd_box(px: f64, py: f64, cx: f64, cy: f64, hw: f64, hh: f64, r: f64) -> f64 {
    let qx = (px - cx).abs() - hw + r;
    let qy = (py - cy).abs() - hh + r;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

fn sd_segment(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = clampf(((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy), 0.0, 1.0);
    (px - ax - dx * t).hypot(py - ay - dy * t)
}

fn smin(a: f64, b: f64, k: f64) -> f64 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

fn coverage(sd: f64, k: f64) -> f64 {
    clampf(0.5 - sd * k, 0.0, 1.0)
}

/// Premultiplied pixel, 0..1.
#[derive(Default, Clone, Copy)]
struct Pix {
    r: f64,
    g: f64,
    b: f64,
    a: f64,
}

impl Pix {
    fn over(&mut self, c: [f64; 3], a: f64) {
        if a <= 0.0 {
            return;
        }
        self.r = c[0] * a + self.r * (1.0 - a);
        self.g = c[1] * a + self.g * (1.0 - a);
        self.b = c[2] * a + self.b * (1.0 - a);
        self.a = a + self.a * (1.0 - a);
    }
}

pub fn rgbf(c: [u8; 3]) -> [f64; 3] {
    [c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0]
}

fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Relative luminance (WCAG) of an sRGB colour, 0..1.
fn luminance(c: [f64; 3]) -> f64 {
    let lin = |v: f64| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// A theme colour readable on the taskbar: moved towards white (dark taskbar) or black (light one)
/// until its contrast with the taskbar is at least 3:1.
fn on_taskbar(c: [f64; 3], light: bool) -> [f64; 3] {
    let (bar, to) = if light { (BAR_LIGHT, [0.0; 3]) } else { (BAR_DARK, [1.0; 3]) };
    let mut x = c;
    for _ in 0..30 {
        let l = luminance(x);
        if (l.max(bar) + 0.05) / (l.min(bar) + 0.05) >= 3.0 {
            break;
        }
        x = lerp3(x, to, 0.08);
    }
    x
}

// ---------- digits (stroke glyphs, SDF) ----------

#[derive(Clone, Copy)]
struct Placed {
    ch: u8,
    /// left edge and width of the centre line
    x0: f64,
    w: f64,
    /// top and bottom of the centre line, half stroke width
    y0: f64,
    y1: f64,
    r: f64,
}

fn arc_pts(cx: f64, cy: f64, rx: f64, ry: f64, a0: f64, a1: f64, step: f64) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    let step = if a1 < a0 { -step } else { step };
    let mut a = a0;
    while (step > 0.0 && a < a1) || (step < 0.0 && a > a1) {
        let r = a.to_radians();
        out.push([cx + rx * r.cos(), cy + ry * r.sin()]);
        a += step;
    }
    let r = a1.to_radians();
    out.push([cx + rx * r.cos(), cy + ry * r.sin()]);
    out
}

static GLYPHS: LazyLock<[Vec<[f64; 2]>; 10]> = LazyLock::new(|| {
    let mut g: [Vec<[f64; 2]>; 10] = Default::default();
    g[1] = vec![[0.0, 0.22], [1.0, 0.0], [1.0, 1.0]];
    g[7] = vec![[0.0, 0.0], [1.0, 0.0], [0.38, 1.0]];
    let mut two = arc_pts(0.5, 0.30, 0.48, 0.30, -170.0, 40.0, 10.0);
    two.extend([[0.02, 1.0], [1.0, 1.0]]);
    g[2] = two;
    let mut five = vec![[0.95, 0.0], [0.12, 0.0], [0.06, 0.42]];
    five.extend(arc_pts(0.46, 0.69, 0.52, 0.31, -140.0, 140.0, 10.0));
    g[5] = five;
    let mut three = arc_pts(0.5, 0.26, 0.44, 0.26, -160.0, 90.0, 10.0);
    three.extend(arc_pts(0.5, 0.74, 0.48, 0.26, -90.0, 160.0, 10.0));
    g[3] = three;
    g[4] = vec![[0.74, 1.0], [0.74, 0.0], [0.0, 0.68], [1.0, 0.68]];
    let mut six = arc_pts(0.5, 0.5, 0.48, 0.5, -45.0, -180.0, 10.0);
    six.extend(arc_pts(0.5, 0.7, 0.48, 0.3, 180.0, 540.0, 15.0));
    g[6] = six;
    let mut eight = arc_pts(0.5, 0.26, 0.4, 0.26, 90.0, 450.0, 15.0);
    eight.extend(arc_pts(0.5, 0.73, 0.48, 0.27, -90.0, 270.0, 15.0));
    g[8] = eight;
    let mut nine = arc_pts(0.5, 0.3, 0.48, 0.3, 0.0, 360.0, 15.0);
    nine.extend(arc_pts(0.5, 0.5, 0.48, 0.5, 0.0, 135.0, 10.0));
    g[9] = nine;
    g
});

fn layout_number(label: &[u8]) -> Vec<Placed> {
    let p = |ch, x0, w, y0, y1, r| Placed { ch, x0, w, y0, y1, r };
    match label.len() {
        1 => vec![p(label[0], 10.5, 11.0, 5.5, 26.5, 2.3)],
        2 => vec![p(label[0], 4.6, 8.8, 5.5, 26.5, 2.0), p(label[1], 18.6, 8.8, 5.5, 26.5, 2.0)],
        _ => vec![p(label[0], 2.6, 3.2, 6.0, 26.0, 1.7), p(label[1], 11.0, 6.6, 6.0, 26.0, 1.7), p(label[2], 22.1, 6.6, 6.0, 26.0, 1.7)],
    }
}

/// Distance to the glyph centre line (without stroke width).
fn glyph_skel(g: &Placed, px: f64, py: f64) -> f64 {
    if g.ch == b'0' {
        let (hw, hh) = (g.w / 2.0, (g.y1 - g.y0) / 2.0);
        return sd_box(px, py, g.x0 + hw, (g.y0 + g.y1) / 2.0, hw, hh, hw).abs();
    }
    let pts = &GLYPHS[(g.ch - b'0') as usize];
    let h = g.y1 - g.y0;
    let mut best = 1e9;
    for i in 0..pts.len().saturating_sub(1) {
        let d = sd_segment(px, py, g.x0 + pts[i][0] * g.w, g.y0 + pts[i][1] * h, g.x0 + pts[i + 1][0] * g.w, g.y0 + pts[i + 1][1] * h);
        best = f64::min(best, d);
    }
    best
}

/// Number with a varying stroke: fat=1 — almost solid "drops", fat=0 — plain digits.
fn number_sd(gs: &[Placed], px: f64, py: f64, fat: f64, grow: f64) -> f64 {
    let mut best = 40.0;
    for g in gs {
        let d = glyph_skel(g, px, py) - lerpf(g.r, 5.2, fat) * grow;
        best = f64::min(best, d);
    }
    best
}

/// Approximate signed distance to the mark in a unit box, scaled by z about the centre; the middle of
/// the mark (between the apex and the wing tips) sits at the centre. Inside both sides and above the
/// bottom curve; the sharp wing tips come from the plain maximum.
fn sd_mark(x: f64, y: f64, z: f64) -> f64 {
    let (l, apex, wing, ctl) = MARK;
    let (x, y) = (0.5 + (x - 0.5) / z, (apex + wing) / 2.0 + (y - 0.5) / z);
    let side = ((x - 0.5).abs() * (wing - apex) - (y - apex) * (0.5 - l)) / (wing - apex).hypot(0.5 - l);
    // the control point is over the middle, so x runs linearly with the curve parameter
    let t = (x - l) / (1.0 - 2.0 * l);
    let bottom = wing - 2.0 * t * (1.0 - t) * (wing - ctl);
    let slope = -2.0 * (wing - ctl) * (1.0 - 2.0 * t) / (1.0 - 2.0 * l);
    side.max((y - bottom) / 1f64.hypot(slope)) * z
}

/// The mark alone in one colour (the tray menu title): BGRA, straight alpha, n*n.
pub fn mark(n: usize, c: [f64; 3]) -> Vec<u8> {
    let mut out = vec![0u8; n * n * 4];
    let k = n as f64;
    for y in 0..n {
        for x in 0..n {
            let a = coverage(sd_mark((x as f64 + 0.5) / k, (y as f64 + 0.5) / k, MARK_Z), k);
            if a > 0.0 {
                let o = (y * n + x) * 4;
                out[o..o + 4].copy_from_slice(&[(c[2] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[0] * 255.0 + 0.5) as u8, (a * 255.0 + 0.5) as u8]);
            }
        }
    }
    out
}

/// As the headset LED (darker for a light taskbar). Older Arctis (7 / 7+ / 7P…): green from 50 %,
/// yellow 20–49 %, red below 20 %. Arctis Nova: green from 50 %, yellow 15–49 %, orange 6–14 %, red 0–5 %.
fn battery_color(level: i32, light: bool, nova: bool) -> [f64; 3] {
    let red = if nova { level <= 5 } else { level <= 19 };
    let orange = nova && level <= 14;
    let yellow = level <= 49;
    match (red, orange, yellow, light) {
        (true, _, _, true) => [0.80, 0.18, 0.15],
        (true, _, _, false) => [1.0, 0.42, 0.40],
        (_, true, _, true) => [0.78, 0.36, 0.04],
        (_, true, _, false) => [1.0, 0.60, 0.30],
        (_, _, true, true) => [0.66, 0.47, 0.0],
        (_, _, true, false) => [1.0, 0.80, 0.30],
        (_, _, _, true) => [0.10, 0.53, 0.24],
        (_, _, _, false) => [0.40, 0.86, 0.50],
    }
}

/// BGRA (straight alpha), size*size.
pub fn render(p: &IconParams) -> Vec<u8> {
    let n = p.size;
    let mut out = vec![0u8; n * n * 4];
    let k = n as f64 / 32.0;

    let ink = if p.light { INK_LIGHT } else { INK_DARK };
    // at the end of the animation the battery number takes the level colour
    let stat_c = battery_color(p.level, p.light, p.nova);
    let stat_t = smoother((p.bat - 0.75) / 0.25);
    let num_col = lerp3(ink, stat_c, stat_t);
    let label = p.level.clamp(0, 100).to_string();
    let glyphs = layout_number(label.as_bytes());
    let morph = p.bat > 0.001;
    // off balance two capsules in the theme's game and chat colours, the quieter one shorter; towards
    // balance they merge into one drop (b 0…0.5), then the drop turns into the mark (0.5…1)
    let game_pct = 100.0 - p.pos.max(0.0);
    let chat_pct = 100.0 - (-p.pos).max(0.0);
    let (hh_g, hh_c) = ((10.0 + 20.0 * game_pct / 100.0) / 2.0, (10.0 + 20.0 * chat_pct / 100.0) / 2.0);
    let (game, chat) = (on_taskbar(rgbf(p.cg), p.light), on_taskbar(rgbf(p.cc), p.light));
    let bb = clampf(p.b * 2.0, 0.0, 1.0);
    let e = smoother((p.b - 0.5) * 2.0);
    let (hw, xl, xr, kk) = (lerpf(6.0, 7.4, bb), lerpf(8.5, 9.6, bb), lerpf(23.5, 22.4, bb), lerpf(0.01, 6.5, bb));

    for y in 0..n {
        for x in 0..n {
            let px = (x as f64 + 0.5) / k;
            let py = (y as f64 + 0.5) / k;
            let (pl, pr) = (sd_box(px, py, xl, 16.0, hw, hh_g, 6.0), sd_box(px, py, xr, 16.0, hw, hh_c, 6.0));
            let sd_m = lerpf(smin(pl, pr, kk), sd_mark(px / 32.0, py / 32.0, MARK_Z) * 32.0, e);
            let side = clampf(0.5 + (pl - pr) / (2.0 * kk.max(2.0)), 0.0, 1.0);
            let grad = lerp3(game, chat, clampf((px - 2.2) / 27.6, 0.0, 1.0));
            let col = lerp3(lerp3(lerp3(game, chat, side), grad, bb), ink, e);

            let mut d = Pix::default();
            if !morph {
                d.over(col, coverage(sd_m, k));
            } else {
                // mark → a ball → digits that thin down from "drops"
                let ball = (px - 16.0).hypot(py - 16.0) - 8.5;
                let (sd, w) = if p.bat < 0.4 {
                    let ww = smoother(p.bat / 0.4);
                    (lerpf(sd_m, ball, ww), ww * 0.4)
                } else {
                    let e = smoother((p.bat - 0.4) / 0.3);
                    let fat = if p.bat >= 1.0 { 0.0 } else { 1.0 - smoother((p.bat - 0.55) / 0.45) };
                    (lerpf(ball, number_sd(&glyphs, px, py, fat, 1.0), e), 0.4 + 0.6 * e)
                };
                d.over(lerp3(col, num_col, w), coverage(sd, k));
            }

            if p.mic != 0 {
                // a dot in the lower right corner with a dark outline
                let dd = (px - 25.5).hypot(py - 25.5);
                d.over([0.08, 0.08, 0.09], coverage(dd - 6.6, k) * 0.92);
                let mcol = if p.mic == 2 { [0.3, 0.86, 0.45] } else { [1.0, 0.27, 0.27] };
                d.over(mcol, coverage(dd - 4.9, k));
            }
            let o = (y * n + x) * 4;
            if d.a > 0.001 {
                out[o] = (clampf(d.b / d.a, 0.0, 1.0) * 255.0 + 0.5) as u8;
                out[o + 1] = (clampf(d.g / d.a, 0.0, 1.0) * 255.0 + 0.5) as u8;
                out[o + 2] = (clampf(d.r / d.a, 0.0, 1.0) * 255.0 + 0.5) as u8;
                out[o + 3] = (clampf(d.a, 0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    out
}

/// HICON from a BGRA buffer (caller destroys it).
pub fn make_hicon(n: usize, bgra: &[u8]) -> HICON {
    unsafe {
        let hdr = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: n as i32,
                biHeight: -(n as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let hdc = GetDC(None);
        let color = CreateDIBSection(Some(hdc), &hdr, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None, hdc);
        let Ok(color) = color else { return HICON::default() };
        if bits.is_null() {
            let _ = DeleteObject(color.into());
            return HICON::default();
        }
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len());
        let mask = CreateBitmap(n as i32, n as i32, 1, 1, None);
        let ii = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let h = CreateIconIndirect(&ii).unwrap_or_default();
        let _ = DeleteObject(color.into());
        if !mask.is_invalid() {
            let _ = DeleteObject(mask.into());
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(size: usize, bat: f64, light: bool) -> IconParams {
        // the colours of the Black hole theme
        IconParams { size, pos: 0.0, b: 1.0, bat, level: 75, nova: false, light, mic: 0, cg: [0xE9, 0xEA, 0xEE], cc: [0x8F, 0xB2, 0xE3] }
    }

    #[test]
    fn the_mark_follows_the_taskbar() {
        let px = |p: &IconParams, x: usize, y: usize| {
            let o = (y * p.size + x) * 4;
            let v = render(p);
            (v[o + 2], v[o + 3])
        };
        // inside the mark (low in the middle of a wing), outside (a top corner, under the bow)
        let (dark, light) = (params(32, 0.0, false), params(32, 0.0, true));
        assert!(px(&dark, 9, 22).1 > 200 && px(&dark, 9, 22).0 > 240);
        assert!(px(&light, 9, 22).1 > 200 && px(&light, 9, 22).0 < 40);
        assert!(px(&dark, 2, 2).1 == 0 && px(&dark, 16, 26).1 < 30);
        // the menu mark is the same shape
        let m = mark(32, [1.0, 1.0, 1.0]);
        assert!(m[(22 * 32 + 9) * 4 + 3] > 200 && m[(2 * 32 + 2) * 4 + 3] == 0);
        // off balance: two capsules in the theme colours, the gap between them empty
        let caps = IconParams { pos: -60.0, b: 0.0, ..params(32, 0.0, false) };
        assert!(px(&caps, 16, 16).1 < 30 && px(&caps, 8, 16).1 > 200 && px(&caps, 23, 16).1 > 200);
        assert!(px(&caps, 8, 16).0 > 220 && px(&caps, 23, 16).0 < 170);
        // on a light taskbar the near-white game colour turns dark enough to be seen
        let caps_l = IconParams { light: true, ..caps };
        assert!(px(&caps_l, 8, 16).0 < 140);
        for (c, light) in [([0xE9, 0xEA, 0xEE], true), ([0x14, 0x14, 0x30], false), ([0xFF, 0x2E, 0x97], true)] {
            let l = luminance(on_taskbar(rgbf(c), light));
            let bar = if light { BAR_LIGHT } else { BAR_DARK };
            assert!((l.max(bar) + 0.05) / (l.min(bar) + 0.05) >= 3.0);
        }
    }

    #[test]
    fn battery_colours_follow_the_headset_led() {
        let c = |level, nova| battery_color(level, false, nova);
        let (green, yellow, red) = (c(100, false), c(49, false), c(19, false));
        // older Arctis: 100–50 green, 49–20 yellow, 19–0 red
        assert_eq!((c(50, false), c(20, false), c(1, false)), (green, yellow, red));
        assert!(green != yellow && yellow != red);
        // Nova: 100–50 green, 49–15 yellow, 14–6 orange, 5–0 red
        let orange = c(14, true);
        assert_eq!((c(50, true), c(15, true), c(6, true), c(5, true), c(0, true)), (green, yellow, orange, red, red));
        assert!(orange != yellow && orange != red);
    }

    /// Frames of the tray icon for a look: ARCMIX_ICON_SHEET=<file.png> cargo test -- --ignored icon_sheet
    #[test]
    #[ignore]
    fn icon_sheet() {
        let Ok(path) = std::env::var("ARCMIX_ICON_SHEET") else { return };
        let (n, z, cols) = (32usize, 4usize, 8usize);
        const THEMES: [([u8; 3], [u8; 3]); 4] = [([0xE9, 0xEA, 0xEE], [0x8F, 0xB2, 0xE3]), ([0xFF, 0x2E, 0x97], [0x19, 0xD3, 0xFF]), ([0x1E, 0xD7, 0x60], [0xB3, 0xB3, 0xB3]), ([0xFF, 0x00, 0x33], [0x3E, 0xA6, 0xFF])];
        let rows: Vec<(Vec<IconParams>, [u8; 3])> = vec![
            ((0..cols).map(|i| params(n, i as f64 / 7.0, false)).collect(), [0x20, 0x20, 0x20]),
            ((0..cols).map(|i| params(n, i as f64 / 7.0, true)).collect(), [0xF3, 0xF3, 0xF3]),
            ((0..cols).map(|i| params([16, 20, 24, 32, 16, 20, 24, 32][i], 0.0, i >= 4)).collect(), [0x20, 0x20, 0x20]),
            ((0..cols).map(|i| IconParams { pos: 50.0, b: 1.0 - i as f64 / 7.0, ..params(n, 0.0, false) }).collect(), [0x20, 0x20, 0x20]),
            ((0..cols).map(|i| IconParams { pos: -50.0, b: 1.0 - i as f64 / 7.0, ..params(n, 0.0, true) }).collect(), [0xF3, 0xF3, 0xF3]),
            ((0..cols).map(|i| IconParams { pos: [50.0, -50.0, 80.0, -80.0][i % 4], b: 0.0, ..params([16, 20, 24, 32, 16, 20, 24, 32][i], 0.0, false) }).collect(), [0x20, 0x20, 0x20]),
            // theme colours: Black hole, Nebula, Spotify, YouTube at 32 and 20 px, a dark taskbar, then a light one
            ((0..cols).map(|i| {
                let (cg, cc) = THEMES[i % 4];
                IconParams { pos: 50.0, b: 0.0, cg, cc, ..params([32, 20][i / 4], 0.0, false) }
            }).collect(), [0x20, 0x20, 0x20]),
            ((0..cols).map(|i| {
                let (cg, cc) = THEMES[i % 4];
                IconParams { pos: 50.0, b: 0.0, cg, cc, ..params([32, 20][i / 4], 0.0, true) }
            }).collect(), [0xF3, 0xF3, 0xF3]),
        ];
        let (w, h) = (cols * n * z, rows.len() * n * z);
        let mut img = vec![0u8; w * h * 4];
        for (r, (frames, bg)) in rows.iter().enumerate() {
            for (c, p) in frames.iter().enumerate() {
                let px = render(p);
                for y in 0..n * z {
                    for x in 0..n * z {
                        let (sx, sy) = (x / z, y / z);
                        let o = ((r * n * z + y) * w + c * n * z + x) * 4;
                        let (a, rgb) = if sx < p.size && sy < p.size {
                            let i = (sy * p.size + sx) * 4;
                            (px[i + 3] as f64 / 255.0, [px[i + 2], px[i + 1], px[i]])
                        } else {
                            (0.0, [0, 0, 0])
                        };
                        for ch in 0..3 {
                            img[o + ch] = (rgb[ch] as f64 * a + bg[ch] as f64 * (1.0 - a) + 0.5) as u8;
                        }
                        img[o + 3] = 255;
                    }
                }
            }
        }
        std::fs::write(path, crate::png::encode(w as u32, h as u32, &img)).unwrap();
    }
}
