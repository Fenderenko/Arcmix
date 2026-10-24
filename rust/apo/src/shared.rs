//! What the Arcmix module in the Windows audio engine and the app exchange (both include this file):
//! two small files in %ProgramData%\Arcmix. The audio engine runs in a sandbox (a less privileged
//! AppContainer) that can open neither the app's objects nor a Global section of its own; the installer
//! opens that folder to it.
//!
//! curves.bin — written by the app (a temporary file renamed over it): a header and the curve of each
//! device. live.bin — written by the module while it processes sound: the endpoints and the tick count
//! of their last buffer.
//!
//! Inside the module the curves sit in process memory under a seqlock: its file thread makes `seq` odd,
//! writes, makes it even again; a reader copies its entry and takes it only if `seq` was even and
//! unchanged. Each module instance claims a slot of `seen` (its endpoint GUID) and stamps it with the
//! tick count while it processes sound.

use std::sync::atomic::{AtomicU32, AtomicU64};

/// The folder under %ProgramData%.
pub const DIR: &str = "Arcmix";
pub const CURVES: &str = "curves.bin";
pub const LIVE: &str = "live.bin";
/// wrap.txt — written by the installer (elevated, so users cannot change it): per line an endpoint GUID
/// and the effect the module took the slot from; the module runs that effect inside itself.
pub const WRAP: &str = "wrap.txt";
pub const MAGIC: u32 = 0x5041_5841; // "AXAP"
pub const VERSION: u32 = 2;
pub const ENTRIES: usize = 16;
pub const BANDS: usize = 20;
/// Endpoint GUIDs as "{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}", UTF-16, zero-padded.
pub const GUID_LEN: usize = 40;

/// The curve of one output device.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Entry {
    pub guid: [u16; GUID_LEN],
    /// 0 — pass the sound unchanged
    pub on: u32,
    pub preamp_db: f32,
    /// peaking filters in use (≤ BANDS)
    pub n: u32,
    pub freq: [f32; BANDS],
    pub gain_db: [f32; BANDS],
    pub q: [f32; BANDS],
}

#[allow(dead_code)] // used by the app
impl Entry {
    pub const EMPTY: Entry = Entry { guid: [0; GUID_LEN], on: 0, preamp_db: 0.0, n: 0, freq: [0.0; BANDS], gain_db: [0.0; BANDS], q: [0.0; BANDS] };
}

/// A module instance that processes sound on an endpoint.
#[repr(C)]
pub struct Seen {
    /// a hash of the endpoint GUID; 0 — free
    pub key: AtomicU64,
    /// GetTickCount of the last processed buffer
    pub tick: AtomicU32,
    pub _pad: u32,
}

/// The module's copy of the curves and its instances (process memory, zeroed at start).
#[repr(C)]
pub struct Shared {
    pub seq: AtomicU32,
    pub count: AtomicU32,
    pub entries: [Entry; ENTRIES],
    pub seen: [Seen; ENTRIES],
}

/// curves.bin: magic, version, count, then ENTRIES entries.
const HEAD: usize = 16;
pub const CURVES_SIZE: usize = HEAD + ENTRIES * std::mem::size_of::<Entry>();
/// live.bin: per endpoint a key (8 bytes) and a tick (4) with padding (4).
const LIVE_ITEM: usize = 16;

#[allow(dead_code)] // used by the app
pub fn encode_curves(entries: &[Entry]) -> Vec<u8> {
    let n = entries.len().min(ENTRIES);
    let mut b = vec![0u8; CURVES_SIZE];
    b[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    b[4..8].copy_from_slice(&VERSION.to_le_bytes());
    b[8..12].copy_from_slice(&(n as u32).to_le_bytes());
    for (i, e) in entries.iter().take(n).enumerate() {
        let at = HEAD + i * std::mem::size_of::<Entry>();
        // SAFETY: Entry is plain repr(C) data without padding
        let raw = unsafe { std::slice::from_raw_parts(e as *const Entry as *const u8, std::mem::size_of::<Entry>()) };
        b[at..at + raw.len()].copy_from_slice(raw);
    }
    b
}

/// The entries of curves.bin; None when the file is not ours or is cut short.
pub fn decode_curves(b: &[u8]) -> Option<Vec<Entry>> {
    if b.len() < CURVES_SIZE || u32::from_le_bytes(b[0..4].try_into().ok()?) != MAGIC || u32::from_le_bytes(b[4..8].try_into().ok()?) != VERSION {
        return None;
    }
    let n = (u32::from_le_bytes(b[8..12].try_into().ok()?) as usize).min(ENTRIES);
    Some(
        (0..n)
            .map(|i| {
                let at = HEAD + i * std::mem::size_of::<Entry>();
                // SAFETY: the range is inside the checked length; any bit pattern is a valid Entry
                unsafe { std::ptr::read_unaligned(b[at..].as_ptr() as *const Entry) }
            })
            .collect(),
    )
}

pub fn encode_live(items: &[(u64, u32)]) -> Vec<u8> {
    let mut b = Vec::with_capacity(items.len() * LIVE_ITEM);
    for (k, t) in items.iter().take(ENTRIES) {
        b.extend_from_slice(&k.to_le_bytes());
        b.extend_from_slice(&t.to_le_bytes());
        b.extend_from_slice(&[0; 4]);
    }
    b
}

#[allow(dead_code)] // used by the app
pub fn decode_live(b: &[u8]) -> Vec<(u64, u32)> {
    b.chunks_exact(LIVE_ITEM).take(ENTRIES).map(|c| (u64::from_le_bytes(c[0..8].try_into().unwrap_or_default()), u32::from_le_bytes(c[8..12].try_into().unwrap_or_default()))).collect()
}

/// The effect wrapped on this endpoint, from wrap.txt ("{endpoint} {effect CLSID}" per line).
#[allow(dead_code)] // used by the module
pub fn wrap_lookup(text: &str, endpoint: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let mut p = l.split_whitespace();
        let (e, c) = (p.next()?, p.next()?);
        e.eq_ignore_ascii_case(endpoint).then(|| c.to_string())
    })
}

/// FNV-1a of a GUID string, case-insensitive; never 0.
pub fn key_of(guid: &[u16]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &c in guid.iter().take_while(|c| **c != 0) {
        let c = if (b'a' as u16..=b'z' as u16).contains(&c) { c - 32 } else { c };
        h ^= c as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h.max(1)
}

/// The GUID string into the fixed field (upper case, as the registry keys are named).
pub fn guid_field(s: &str) -> [u16; GUID_LEN] {
    let mut f = [0u16; GUID_LEN];
    for (d, c) in f.iter_mut().zip(s.to_uppercase().encode_utf16().take(GUID_LEN - 1)) {
        *d = c;
    }
    f
}
