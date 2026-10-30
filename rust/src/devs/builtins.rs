//! Payload transforms (payload_transform) — a port of device-config/src/builtins.rs and biquad.rs
//! from Linux-Arctis-Manager.

use std::collections::BTreeMap;

use serde_json::Value;

use super::engine::any_num;

type Args = BTreeMap<String, Value>;
pub type BuiltinFn = fn(Vec<u8>, &Args) -> Vec<Vec<u8>>;

fn arg_f(a: &Args, k: &str) -> Option<f64> {
    a.get(k).and_then(any_num)
}

fn arg_i(a: &Args, k: &str) -> Option<i64> {
    arg_f(a, k).map(|f| f as i64)
}

fn one(b: Vec<u8>) -> Vec<Vec<u8>> {
    vec![b]
}

fn be32f(b: &[u8]) -> f64 {
    f32::from_bits(u32::from_be_bytes([b[0], b[1], b[2], b[3]])) as f64
}

/// Graphic EQ: a 2-byte header + float32 dB → byte round(2·(offset+dB)).
fn graphic_eq_gains(b: Vec<u8>, a: &Args) -> Vec<Vec<u8>> {
    let (Some(off), Some(cl)) = (arg_f(a, "offset_db"), arg_i(a, "clamp_max")) else { return one(b) };
    if b.len() < 2 {
        return one(b);
    }
    let mut out = vec![b[0], b[1]];
    let mut i = 2;
    while i + 4 <= b.len() {
        out.push((2.0 * (off + be32f(&b[i..i + 4]))).round().clamp(0.0, cl as f64) as u8);
        i += 4;
    }
    one(out)
}

fn minutes_to_timer_enum(m: u8) -> u8 {
    match m {
        1 => 1,
        5 => 2,
        10 => 3,
        15 => 4,
        30 => 5,
        60 => 6,
        _ => 0,
    }
}

fn map_byte2(mut b: Vec<u8>, f: impl Fn(u8) -> u8) -> Vec<Vec<u8>> {
    if b.len() >= 3 {
        b[2] = f(b[2]);
    }
    one(b)
}

/// Band gain encoding.
#[derive(Clone, Copy)]
struct GainEnc {
    signed_tenths: bool,
    off: f64,
    clamp: i64,
}

impl GainEnc {
    fn enc(self, db: f64) -> u8 {
        if self.signed_tenths {
            (db * 10.0).round().clamp(-128.0, 127.0) as i8 as u8
        } else {
            (2.0 * (self.off + db)).round().clamp(0.0, self.clamp as f64) as u8
        }
    }
}

fn gain_enc_from_args(a: &Args) -> Option<GainEnc> {
    match a.get("gain_flavour").and_then(|v| v.as_str()) {
        Some("signed_tenths_db") => Some(GainEnc { signed_tenths: true, off: 0.0, clamp: 0 }),
        Some("unsigned_half_db_offset") => Some(GainEnc { signed_tenths: false, off: arg_f(a, "offset_db")?, clamp: arg_i(a, "clamp_max")? }),
        _ => None,
    }
}

/// u16 frequency + u8 type + f32 gain + f32 Q
const PEQ_BAND_SRC: usize = 11;

/// The headset takes every multi-byte band field little-endian (Arctis Sound Manager, checked against a
/// Nova 7P Gen 2); the description is serialized big-endian, so the frequency is turned round.
fn encode_peq_bands(out: &mut Vec<u8>, b: &[u8], off: usize, n: usize, g: GainEnc) {
    for i in 0..n {
        let base = off + i * PEQ_BAND_SRC;
        out.extend(u16::from_be_bytes([b[base], b[base + 1]]).to_le_bytes());
        out.push(b[base + 2]);
        out.push(g.enc(be32f(&b[base + 3..base + 7])));
        let q = (be32f(&b[base + 7..base + 11]) * 1000.0).round().clamp(0.0, 65535.0) as u16;
        out.extend(q.to_le_bytes());
    }
}

fn header_band(a: &Args) -> Option<(usize, usize)> {
    Some((arg_i(a, "header_len")?.max(0) as usize, arg_i(a, "band_count")?.max(0) as usize))
}

fn peq_name(b: Vec<u8>, a: &Args) -> Vec<Vec<u8>> {
    let Some((hl, n)) = header_band(a) else { return one(b) };
    if b.len() < 4 {
        return one(b);
    }
    let no = hl + n * PEQ_BAND_SRC;
    let mut out = b[..4].to_vec();
    if b.len() > no {
        out.extend(&b[no..]);
    }
    one(out)
}

fn peq_bands(b: Vec<u8>, a: &Args) -> Vec<Vec<u8>> {
    let (Some((hl, n)), Some(g)) = (header_band(a), gain_enc_from_args(a)) else { return one(b) };
    if b.len() < hl + n * PEQ_BAND_SRC || b.len() < 5 {
        return one(b);
    }
    let mut out = vec![b[0], b[4], b[2]];
    encode_peq_bands(&mut out, &b, hl, n, g);
    one(out)
}

fn peq_commit(b: Vec<u8>, a: &Args) -> Vec<Vec<u8>> {
    match arg_i(a, "commit_byte_offset") {
        Some(o) if (o as usize) < b.len() => one(vec![b[0], b[o as usize]]),
        _ => one(b),
    }
}

fn gamebuds_bands(b: Vec<u8>, a: &Args) -> Vec<Vec<u8>> {
    let Some((hl, n)) = header_band(a) else { return one(b) };
    if b.len() < hl + n * PEQ_BAND_SRC || b.len() < 5 {
        return one(b);
    }
    let mut out = vec![b[0], b[4]];
    encode_peq_bands(&mut out, &b, hl, n, GainEnc { signed_tenths: true, off: 0.0, clamp: 0 });
    one(out)
}

fn peq_named_slot(b: Vec<u8>, a: &Args) -> Vec<Vec<u8>> {
    let (Some((hl, n)), Some(g)) = (header_band(a), gain_enc_from_args(a)) else { return one(b) };
    if b.len() < hl + n * PEQ_BAND_SRC {
        return one(b);
    }
    let mut out = b[..hl].to_vec();
    encode_peq_bands(&mut out, &b, hl, n, g);
    one(out)
}

fn named_slot_signed_gains(b: Vec<u8>, a: &Args) -> Vec<Vec<u8>> {
    let Some(hl) = arg_i(a, "header_len").map(|v| v.max(0) as usize) else { return one(b) };
    if b.len() < hl {
        return one(b);
    }
    let mut out = b[..hl].to_vec();
    let mut i = hl;
    while i + 4 <= b.len() {
        out.push(GainEnc { signed_tenths: true, off: 0.0, clamp: 0 }.enc(be32f(&b[i..i + 4])));
        i += 4;
    }
    one(out)
}

// ---------- biquad filters (AV6X02 — Arctis 7/1 Wireless; CX20892 — Arctis 5) ----------

#[derive(Clone, Copy)]
enum Shape {
    Low,
    High,
    Peak,
}

fn biquad(shape: Shape, fs: f64, f0: f64, gain: f64, q: f64) -> [f64; 6] {
    if gain == 0.0 {
        return [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    }
    let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
    let (cw, sw) = (w0.cos(), w0.sin());
    let al = sw / 2.0 / q;
    let a = 10f64.powf(gain / 40.0);
    let t = 2.0 * a.sqrt() * al;
    match shape {
        Shape::Low => [
            a * ((a + 1.0) - (a - 1.0) * cw + t),
            2.0 * a * (a - 1.0 - (a + 1.0) * cw),
            a * ((a + 1.0) - (a - 1.0) * cw - t),
            (a + 1.0) + (a - 1.0) * cw + t,
            -2.0 * ((a - 1.0) + (a + 1.0) * cw),
            (a + 1.0) + (a - 1.0) * cw - t,
        ],
        Shape::High => [
            a * ((a + 1.0) + (a - 1.0) * cw + t),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cw),
            a * ((a + 1.0) + (a - 1.0) * cw - t),
            (a + 1.0) - (a - 1.0) * cw + t,
            2.0 * (a - 1.0 - (a + 1.0) * cw),
            (a + 1.0) - (a - 1.0) * cw - t,
        ],
        Shape::Peak => [1.0 + al * a, -2.0 * cw, 1.0 - al * a, 1.0 + al / a, -2.0 * cw, 1.0 - al / a],
    }
}

fn norm_biquad(c: [f64; 6]) -> [f64; 6] {
    [c[0] / c[3], c[1] / c[3], c[2] / c[3], 1.0, c[4] / c[3], c[5] / c[3]]
}

// AV6X02 (Arctis 7): coefficient writes as SteelSeries GG sends them (captured from the receiver's echo):
// float32 little-endian, a1/a2 negated, each band then switched on (payload 1) or off (0); the right
// channel's filter numbers follow the left ones at +42 for the equalizer bands, +28 for the fixed stages.
const AV6_BANDS: [(Shape, f64, f64); 6] =
    [(Shape::Low, 64.0, 1.1), (Shape::Peak, 180.0, 0.8), (Shape::Peak, 500.0, 0.8), (Shape::Peak, 1400.0, 0.8), (Shape::Peak, 3900.0, 0.7), (Shape::High, 11000.0, 1.1)];
const AV6_BASE: [u8; 6] = [1, 8, 15, 22, 29, 36];

fn av6_msg(ch: u8, addr: u8, p: [u8; 4], num: u8) -> Vec<u8> {
    vec![0x06, 0x28, ch, addr, p[0], p[1], p[2], p[3], num]
}

fn av6_channel(ch: u8, band: u8, c: [f64; 6], base: u8) -> Vec<Vec<u8>> {
    let bits = band << 4;
    if c[0] == 1.0 && c[3] == 1.0 {
        return vec![av6_msg(ch, bits, [0; 4], base + 6)];
    }
    let mut out: Vec<Vec<u8>> = (0..6).map(|i| av6_msg(ch, bits | 0x08 | i as u8, (c[i] as f32).to_le_bytes(), base + i as u8)).collect();
    out.push(av6_msg(ch, bits, [1, 0, 0, 0], base + 6));
    out
}

fn av6_band(band: u8, n: [f64; 6], base: u8, right: u8) -> Vec<Vec<u8>> {
    let c = [n[0], n[1], n[2], n[3], -n[4], -n[5]];
    let mut out = av6_channel(3, band, c, base);
    out.extend(av6_channel(4, band, c, base + right));
    out
}

/// The equalizer band (0..5) an AV6X02 coefficient or switch message is for; None for any other message.
pub fn av6_band_of(m: &[u8]) -> Option<usize> {
    (m.len() > 3 && m[0] == 0x06 && m[1] == 0x28 && (m[2] == 3 || m[2] == 4) && m[3] >> 4 < 6).then(|| (m[3] >> 4) as usize)
}

fn av6_eq(b: Vec<u8>, _: &Args) -> Vec<Vec<u8>> {
    if b.len() < 2 + 6 * 4 {
        return one(b);
    }
    let mut out = Vec::new();
    for (i, (sh, f, q)) in AV6_BANDS.iter().enumerate() {
        let g = be32f(&b[2 + i * 4..6 + i * 4]);
        out.extend(av6_band(i as u8, norm_biquad(biquad(*sh, 48000.0, *f, g, *q)), AV6_BASE[i], 42));
    }
    out
}

fn av6_init(_: Vec<u8>, _: &Args) -> Vec<Vec<u8>> {
    let mut out = vec![av6_msg(0x18, 0, [0; 4], 0), av6_msg(0x05, 0, [0x20, 0, 0, 0], 141), av6_msg(0x06, 0, [0x20, 0, 0, 0], 142)];
    let lp = [0.0003297046, 0.0006594092, 0.0003297046, 1.0, -1.9479868, 0.9493058];
    let hp = [0.97432315, -1.9486463, 0.97432315, 1.0, -1.9479868, 0.9493058];
    out.extend(av6_band(6, lp, 85, 28));
    out.extend(av6_band(7, lp, 92, 28));
    out.extend(av6_band(8, hp, 99, 28));
    out.extend(av6_band(9, hp, 106, 28));
    out.extend([
        av6_msg(0x07, 0x10, [0x67, 0, 0, 0], 145),
        av6_msg(0x07, 0x20, [0xF6, 0, 0, 0], 146),
        av6_msg(0x07, 0x02, [0; 4], 147),
        av6_msg(0x07, 0x11, [0x67, 0, 0, 0], 150),
        av6_msg(0x07, 0x21, [0xF6, 0, 0, 0], 151),
        av6_msg(0x07, 0x03, [0; 4], 152),
        // the four stage settings GG sends after them (not in the Linux-Arctis-Manager port)
        av6_msg(0x07, 0x08, [0x7F, 0, 0, 0], 143),
        av6_msg(0x07, 0x04, [0, 0x7F, 0, 0], 144),
        av6_msg(0x07, 0x09, [0x7F, 0, 0, 0], 148),
        av6_msg(0x07, 0x05, [0, 0x7F, 0, 0], 149),
    ]);
    out
}

const A5_BANDS: [(Shape, f64, f64, u8); 5] =
    [(Shape::Low, 62.5, 1.1, 0x20), (Shape::Peak, 250.0, 0.8, 0x2B), (Shape::Peak, 1000.0, 0.8, 0x36), (Shape::Peak, 3600.0, 0.7, 0x41), (Shape::High, 12000.0, 1.1, 0x4C)];

fn short_floor(f: f64) -> i16 {
    (f * 32768.0 + 0.5).floor().clamp(-32767.0, 32767.0) as i16
}

fn short_trunc(f: f64) -> i16 {
    (f * 32768.0).floor().clamp(-32767.0, 32767.0) as i16
}

fn check_den(b: i16, c: i16) -> bool {
    let a = 0x4000 as f64;
    let (bb, cc) = (-(b as f64), -(c as f64));
    let d = bb * bb - 4.0 * a * cc;
    let (r1, r2) = if d >= 0.0 {
        let s = d.sqrt();
        let (x1, x2) = ((s - bb) / (2.0 * a), (-s - bb) / (2.0 * a));
        (x1 * x1, x2 * x2)
    } else {
        let s = (-d).sqrt();
        let (re, im) = (-bb / (2.0 * a), s / (2.0 * a));
        let r = re * re + im * im;
        (r, r)
    };
    r1 < 1.0 && r2 < 1.0
}

fn a5_stable(a1: f64, a2: f64) -> (i16, i16) {
    let (fa1, fa2) = (short_floor(a1), short_floor(a2));
    let (ta1, ta2) = (short_trunc(a1), short_trunc(a2));
    let c = [(fa1, fa2), (ta1.wrapping_add(1), ta2), (ta1, ta2.wrapping_add(1)), (ta1, ta2), (ta1.wrapping_add(1), ta2.wrapping_add(1))];
    c.iter().copied().find(|x| check_den(x.0, x.1)).unwrap_or(c[4])
}

fn a5_eq(b: Vec<u8>, _: &Args) -> Vec<Vec<u8>> {
    if b.len() < 2 + 5 * 4 {
        return one(b);
    }
    let mut out = Vec::new();
    for (i, (sh, f, q, addr)) in A5_BANDS.iter().enumerate() {
        let g = be32f(&b[2 + i * 4..6 + i * 4]);
        let n = norm_biquad(biquad(*sh, 48000.0, *f, g, *q));
        let (b0, b1, b2, a1c, a2c) = (n[0] / 2.0, n[1] / 2.0, n[2] / 2.0, n[4] / -2.0, n[5] / -2.0);
        let (a1, a2) = a5_stable(a1c, a2c);
        let mut m = vec![0x04, 0x40, 0x0B, 0x10, *addr];
        for v in [short_floor(b0), short_floor(b1), short_floor(b2), a1, a2] {
            m.extend((v as u16).to_be_bytes());
        }
        m.push(0x03);
        out.push(m);
    }
    out
}

pub fn get(name: &str) -> Option<BuiltinFn> {
    Some(match name {
        "builtin:graphic_eq_gains" => graphic_eq_gains,
        "builtin:high_gain_write" => |b, _| map_byte2(b, |v| match v {
            0 => 1,
            1 => 2,
            v => v,
        }),
        "builtin:dim_timer_write" | "builtin:power_timer_write" => |b, _| map_byte2(b, minutes_to_timer_enum),
        "builtin:muted_mic_brightness_write" => |b, _| map_byte2(b, |v| [0, 1, 4, 10][(v as usize).min(3)]),
        "builtin:nova_pro_omni_mic_noise_reduction_write" => |b, _| {
            if b.len() < 3 {
                return one(b);
            }
            one(vec![b[0], b[1], (b[2] != 0) as u8, b[2].max(1)])
        },
        "builtin:parametric_eq_name" => peq_name,
        "builtin:parametric_eq_bands" => peq_bands,
        "builtin:parametric_eq_commit" => peq_commit,
        "builtin:gamebuds_eq_bands" => gamebuds_bands,
        "builtin:parametric_eq_named_slot" => peq_named_slot,
        "builtin:named_slot_signed_gains" => named_slot_signed_gains,
        "builtin:gamebuds_sidetone_write" => |b, _| map_byte2(b, |v| [0, 3, 6, 10][(v as usize).min(3)]),
        "builtin:gamebuds_transparency_write" => |b, _| map_byte2(b, |v| match v {
            1 => 3,
            2 => 6,
            _ => 10,
        }),
        "builtin:av6x02_eq_gains" => av6_eq,
        "builtin:av6x02_init" => av6_init,
        "builtin:arctis5_eq_gains" => a5_eq,
        "builtin:arctis5_commit_settings" => |_, _| one(vec![0x04, 0x40, 0x01, 0x11, 0x54, 0x9B]),
        "builtin:first_byte_only" => |b, _| one(b[..b.len().min(1)].to_vec()),
        "builtin:last_byte_only" => |b, _| one(b[b.len().saturating_sub(1)..].to_vec()),
        "builtin:arctis_pro_wireless_eq_bands" => |b, _| {
            if b.len() < 9 {
                return one(b);
            }
            let mut out = b[..9].to_vec();
            out.extend([0u8; 16]);
            one(out)
        },
        "builtin:arctis_pro_wireless_eq_preset" => |b, _| match b.last() {
            Some(&l) => one(vec![0x00, 0x2E, 0xAA, l]),
            None => one(b),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(m: &[u8]) -> f32 {
        f32::from_le_bytes([m[4], m[5], m[6], m[7]])
    }

    #[test]
    fn av6x02_matches_steelseries_gg() {
        // band 0 at +7.3 dB as GG wrote it to an Arctis 7 receiver (its echo, left channel)
        let gg: [(u8, [u8; 4], u8); 7] = [
            (0x08, [0x29, 0x35, 0x80, 0x3f], 0x01),
            (0x09, [0xde, 0x33, 0xff, 0xbf], 0x02),
            (0x0a, [0x66, 0x04, 0x7e, 0x3f], 0x03),
            (0x0b, [0x00, 0x00, 0x80, 0x3f], 0x04),
            (0x0c, [0xda, 0x34, 0xff, 0x3f], 0x05),
            (0x0d, [0xb7, 0x6c, 0x7e, 0xbf], 0x06),
            (0x00, [0x01, 0x00, 0x00, 0x00], 0x07),
        ];
        let mut b = vec![0x06, 0x28];
        for g in [7.3f32, 0.0, 0.0, 0.0, 0.0, 0.0] {
            b.extend(g.to_bits().to_be_bytes());
        }
        let out = av6_eq(b, &Args::new());
        let left: Vec<&Vec<u8>> = out.iter().filter(|m| m[2] == 3 && m[3] >> 4 == 0).collect();
        assert_eq!(left.len(), 7);
        for (m, (addr, p, num)) in left.iter().zip(gg.iter()) {
            assert_eq!((m[3], m[8]), (*addr, *num));
            let want = f32::from_le_bytes(*p);
            assert!((le(m) - want).abs() <= want.abs() * 1e-6 + 1e-9, "addr {addr:02x}: {} vs {want}", le(m));
        }
        // the right channel at +42, a flat band switched off with one message per channel
        assert!(out.iter().any(|m| m[2] == 4 && m[3] == 0x00 && m[4] == 1 && m[8] == 0x31));
        assert!(out.iter().any(|m| m[2] == 3 && m[3] == 0x10 && m[4..8] == [0; 4] && m[8] == 0x0e));
        // the fixed stages of the setup: right channel at +28, the constants byte for byte
        let init = av6_init(Vec::new(), &Args::new());
        assert!(init.iter().any(|m| m[..9] == [0x06, 0x28, 0x04, 0x68, 0x34, 0xdc, 0xac, 0x39, 0x71]));
        assert!(init.iter().any(|m| m[..9] == [0x06, 0x28, 0x07, 0x04, 0x00, 0x7f, 0x00, 0x00, 0x90]));
    }
}
