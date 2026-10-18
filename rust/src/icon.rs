//! Tray icon frames: the game and chat bars (merged in balance), the battery number morph and the
//! microphone badge. Signed distance fields on a 32x32 grid, rendered at the system icon size.

use std::sync::LazyLock;

use windows::Win32::{
    Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, GetDC, ReleaseDC},
    UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO},
};

/// Game and chat colours: those of the app icon (tools/mkicon.py), the same with every theme.
pub const GAME: [u8; 3] = [0x9D, 0x44, 0xE6];
pub const CHAT: [u8; 3] = [0xEE, 0x74, 0xD8];

// The small app logo (tools/mkicon.py, sizes under 40 px): a sky tile and three layers of a concave
// triangle. Geometry in units of the icon side: wing tip inset, back apex, back wing, layer step, bow.
const LOGO_SKY: [[u8; 3]; 2] = [[0x0C, 0x0E, 0x2C], [0x1E, 0x24, 0x60]];
const LOGO_LAYERS: [[u8; 3]; 3] = [[0x6A, 0x36, 0xD8], [0xC2, 0x4F, 0xE0], [0xFF, 0xCD, 0xEB]];
const LOGO_GEO: (f64, f64, f64, f64, f64) = (0.13, 0.15, 0.62, 0.075, 0.08);

pub struct IconParams {
    pub size: usize,
    /// -100..100 (plus = quieter game, louder chat)
    pub pos: f64,
    /// 0..1: bars merged (balance) versus two separate bars
    pub b: f64,
    /// 0..1: progress of the bars → battery number morph (linear in time)
    pub bat: f64,
    /// battery, percent
    pub level: i32,
    /// light taskbar
    pub light: bool,
    /// microphone badge: 0 none, 1 muted (red), 2 talking (green)
    pub mic: i32,
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

/// Approximate signed distance (grid units) to logo layer i: outside both corner superellipses
/// (power 1.5) and above the bowed bottom edge, as in tools/mkicon.py.
fn sd_layer(px: f64, py: f64, i: usize) -> f64 {
    let (l, a0, w0, step, sag) = LOGO_GEO;
    let (x, y) = (px / 32.0, py / 32.0);
    let (apex, wing) = (a0 + i as f64 * step, w0 + i as f64 * step);
    let (a, b) = (0.5 - l, wing - apex);
    let mut d = f64::MIN;
    for cx in [l, 1.0 - l] {
        let (dx, dy) = ((x - cx).abs() / a, (y - apex).abs() / b);
        let e = dx.powf(1.5) + dy.powf(1.5);
        let g = (1.5 * dx.sqrt() / a).hypot(1.5 * dy.sqrt() / b).max(1e-9);
        d = d.max(-(e - 1.0) / g);
    }
    let t = (x - 0.5) / (0.5 - l);
    let bottom = wing - sag * (1.0 - t * t);
    let slope = 2.0 * sag * t / (0.5 - l);
    d.max((y - bottom) / 1f64.hypot(slope)).max(l - x).max(x - (1.0 - l)).max(apex - y) * 32.0
}

/// Over 50 % — green, 26–50 % — yellow, up to 25 % — red (darker for a light taskbar).
fn battery_color(level: i32, light: bool) -> [f64; 3] {
    match (level, light) {
        (..=25, true) => [0.80, 0.18, 0.15],
        (..=25, false) => [1.0, 0.42, 0.40],
        (..=50, true) => [0.66, 0.47, 0.0],
        (..=50, false) => [1.0, 0.80, 0.30],
        (_, true) => [0.10, 0.53, 0.24],
        (_, false) => [0.40, 0.86, 0.50],
    }
}

/// BGRA (straight alpha), size*size.
pub fn render(p: &IconParams) -> Vec<u8> {
    let n = p.size;
    let mut out = vec![0u8; n * n * 4];
    let k = n as f64 / 32.0;

    let (mut cg, mut cc) = (rgbf(p.cg), rgbf(p.cc));
    if p.light {
        for i in 0..3 {
            cg[i] *= 0.62;
            cc[i] *= 0.62;
        }
    }
    // at the end of the animation the battery number takes the level colour
    let stat_c = battery_color(p.level, p.light);
    let stat_t = smoother((p.bat - 0.75) / 0.25);
    let game_pct = 100.0 - p.pos.max(0.0);
    let chat_pct = 100.0 - (-p.pos).max(0.0);
    let hh_g = (10.0 + 20.0 * game_pct / 100.0) / 2.0;
    let hh_c = (10.0 + 20.0 * chat_pct / 100.0) / 2.0;
    let dim_g = 0.40 + 0.60 * game_pct / 100.0;
    let dim_c = 0.40 + 0.60 * chat_pct / 100.0;

    let label = p.level.clamp(0, 100).to_string();
    let glyphs = layout_number(label.as_bytes());
    let morph = p.bat > 0.001;

    // in balance the icon is the app logo, off balance two capsules, in two phases: the capsules merge
    // into one drop (b 0…0.5), then the drop turns into the front layer of the logo while the tile and
    // the back layers come in (0.5…1); those also fade while the battery shows
    let bb = clampf(p.b * 2.0, 0.0, 1.0);
    let e = smoother((p.b - 0.5) * 2.0);
    let logo_a = e * (1.0 - smoother(p.bat / 0.4));
    let (sky0, sky1) = (rgbf(LOGO_SKY[0]), rgbf(LOGO_SKY[1]));
    let layers = LOGO_LAYERS.map(rgbf);

    for y in 0..n {
        for x in 0..n {
            let px = (x as f64 + 0.5) / k;
            let py = (y as f64 + 0.5) / k;

            // two bars; in balance they come together and merge into one, like two drops
            let hw = lerpf(6.0, 7.4, bb);
            let pl = sd_box(px, py, lerpf(8.5, 9.6, bb), 16.0, hw, hh_g, 6.0);
            let pr = sd_box(px, py, lerpf(23.5, 22.4, bb), 16.0, hw, hh_c, 6.0);
            let kk = lerpf(0.01, 6.5, bb);
            let hcol = clampf(0.5 + (pl - pr) / (2.0 * kk.max(2.0)), 0.0, 1.0);
            let grad = lerp3(cg, cc, clampf((px - 2.2) / 27.6, 0.0, 1.0));
            let mc = lerp3(lerp3(cg, cc, hcol), grad, bb);
            let dim_mark = lerpf(dim_g, dim_c, hcol);
            let num_col = lerp3(grad, stat_c, stat_t);

            let mut d = Pix::default();
            if logo_a > 0.001 {
                let ta = coverage(sd_box(px, py, 16.0, 16.0, 15.04, 15.04, 6.88), k) * logo_a;
                d.over(lerp3(sky0, sky1, py / 32.0), ta);
                for (i, c) in layers.iter().take(2).enumerate() {
                    d.over(*c, coverage(sd_layer(px, py, i), k) * ta);
                }
            }
            let sd_mark = lerpf(smin(pl, pr, kk), sd_layer(px, py, 2), e);
            let mc = lerp3(mc, layers[2], e);
            let dim_mark = lerpf(dim_mark, 1.0, e);
            if !morph {
                d.over(mc, coverage(sd_mark, k) * dim_mark);
            } else {
                // bars → a ball → digits that thin down from "drops"
                let ball = (px - 16.0).hypot(py - 16.0) - 8.5;
                let (sd, w) = if p.bat < 0.4 {
                    let ww = smoother(p.bat / 0.4);
                    (lerpf(sd_mark, ball, ww), ww * 0.4)
                } else {
                    let e = smoother((p.bat - 0.4) / 0.3);
                    let fat = if p.bat >= 1.0 { 0.0 } else { 1.0 - smoother((p.bat - 0.55) / 0.45) };
                    (lerpf(ball, number_sd(&glyphs, px, py, fat, 1.0), e), 0.4 + 0.6 * e)
                };
                d.over(lerp3(mc, num_col, w), coverage(sd, k) * lerpf(dim_mark, 1.0, w));
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

    fn params(size: usize, b: f64, pos: f64, bat: f64, light: bool) -> IconParams {
        IconParams { size, pos, b, bat, level: 75, light, mic: 0, cg: GAME, cc: CHAT }
    }

    #[test]
    fn balance_shows_the_logo_tile() {
        let alpha = |p: &IconParams, x: usize, y: usize| render(p)[(y * p.size + x) * 4 + 3];
        // the left edge of the tile is opaque in balance and empty with two capsules
        assert!(alpha(&params(32, 1.0, 0.0, 0.0, false), 1, 16) > 200);
        assert!(alpha(&params(32, 0.0, 50.0, 0.0, false), 1, 16) < 30);
        // the battery number shows without the tile
        assert!(alpha(&params(32, 1.0, 0.0, 1.0, false), 1, 16) < 30);
    }

    /// Frames of the tray icon for a look: ARCMIX_ICON_SHEET=<file.png> cargo test -- --ignored icon_sheet
    #[test]
    #[ignore]
    fn icon_sheet() {
        let Ok(path) = std::env::var("ARCMIX_ICON_SHEET") else { return };
        let (n, z, cols) = (32usize, 4usize, 8usize);
        let rows: Vec<(Vec<IconParams>, [u8; 3])> = vec![
            ((0..cols).map(|i| params(n, 1.0 - i as f64 / 7.0, 40.0, 0.0, false)).collect(), [0x20, 0x20, 0x20]),
            ((0..cols).map(|i| params(n, 1.0, 0.0, i as f64 / 7.0, false)).collect(), [0x20, 0x20, 0x20]),
            ((0..cols).map(|i| params(n, 1.0 - i as f64 / 7.0, 40.0, 0.0, true)).collect(), [0xF3, 0xF3, 0xF3]),
            ((0..cols).map(|i| params([16, 20, 24, 32, 16, 20, 24, 32][i], (i < 4) as i32 as f64, 40.0, 0.0, false)).collect(), [0x20, 0x20, 0x20]),
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
