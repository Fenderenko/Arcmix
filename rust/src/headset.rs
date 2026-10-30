//! Headset state, shared by the HID threads (writers) and the UI thread and the server (readers).
//! Every receiver talked to has a state of its own (an Arctis 7+ and an Arctis 7 can be connected at
//! once); the window, the tray and the balance follow the active one.

use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard},
};

#[derive(Default, Clone)]
pub struct Headset {
    /// USB receiver found
    pub connected: bool,
    /// headset is on
    pub powered: bool,
    pub game: i32,
    pub chat: i32,
    /// a new ChatMix position from the dial, not applied yet
    pub mix_new: bool,
    pub batt_pct: i32,
    pub dev_name: String,
    /// -1 unknown, 0/1 — microphone muted with the headset button
    pub mic_muted: i32,
    /// other batteries: left, right, case, dock
    pub extra_batt: BTreeMap<String, i32>,
    pub charging: bool,
    pub have_batt: bool,
    /// true when driven by a device description (devices.json), false for the native Arctis 7+ driver
    pub generic: bool,
    /// the generic device has a software ChatMix API (no hardware Chat device mixing)
    pub generic_soft_chatmix: bool,
    /// the generic device reports itself wireless (or has a battery)
    pub generic_wireless: bool,
    /// the container (the physical device) of the receiver: its audio devices in Windows share it
    pub container: Option<[u8; 16]>,
}

/// No receiver: nothing known (the microphone state unknown too).
const EMPTY: Headset = Headset {
    connected: false,
    powered: false,
    game: 0,
    chat: 0,
    mix_new: false,
    batt_pct: 0,
    dev_name: String::new(),
    mic_muted: -1,
    extra_batt: BTreeMap::new(),
    charging: false,
    have_batt: false,
    generic: false,
    generic_soft_chatmix: false,
    generic_wireless: false,
    container: None,
};

impl Headset {
    /// Works through a wireless receiver (not by cable).
    pub fn is_wireless(&self) -> bool {
        !self.generic || self.generic_wireless
    }
}

/// The receiver talked to by the native Arctis 7+ driver.
pub const SLOT_NATIVE: u8 = 0;
/// The receiver of a headset from the descriptions.
pub const SLOT_GENERIC: u8 = 1;

struct All {
    slots: Vec<(u8, Headset)>,
    active: Option<u8>,
    /// what readers get with no receiver
    none: Headset,
}

static HS: Mutex<All> = Mutex::new(All { slots: Vec::new(), active: None, none: EMPTY });

pub struct Guard {
    g: MutexGuard<'static, All>,
    slot: Option<u8>,
}

impl Guard {
    fn idx(&self) -> Option<usize> {
        self.slot.and_then(|s| self.g.slots.iter().position(|(k, _)| *k == s))
    }
}

impl std::ops::Deref for Guard {
    type Target = Headset;
    fn deref(&self) -> &Headset {
        match self.idx() {
            Some(i) => &self.g.slots[i].1,
            None => &self.g.none,
        }
    }
}

impl std::ops::DerefMut for Guard {
    fn deref_mut(&mut self) -> &mut Headset {
        match self.idx() {
            Some(i) => &mut self.g.slots[i].1,
            None => &mut self.g.none,
        }
    }
}

fn all() -> MutexGuard<'static, All> {
    HS.lock().unwrap_or_else(|p| p.into_inner())
}

/// The active headset.
pub fn lock() -> Guard {
    let g = all();
    let slot = g.active;
    Guard { g, slot }
}

/// The state of one receiver (its session writes there).
pub fn lock_slot(slot: u8) -> Guard {
    Guard { g: all(), slot: Some(slot) }
}

pub fn snapshot() -> Headset {
    lock().clone()
}

/// The active headset works through a wireless receiver (not by cable).
pub fn wireless() -> bool {
    lock().is_wireless()
}

/// A session started with its receiver: a clean state.
pub fn open_slot(slot: u8) {
    let mut g = all();
    g.slots.retain(|(k, _)| *k != slot);
    g.slots.push((slot, EMPTY));
}

/// The receiver is gone (or its session ended).
pub fn close_slot(slot: u8) {
    let mut g = all();
    g.slots.retain(|(k, _)| *k != slot);
    if g.active == Some(slot) {
        g.active = None;
    }
}

/// Which receiver is active: the one whose audio device plays (`playing` — its container), else the
/// active one while its headset is on, else one that is on, else any. True — it changed.
pub fn choose(playing: Option<[u8; 16]>) -> bool {
    let mut g = all();
    let on = |h: &Headset| h.connected && h.powered;
    let by_out = playing.and_then(|c| g.slots.iter().find(|(_, h)| h.connected && h.container == Some(c)).map(|(k, _)| *k));
    let cur = g.active.filter(|a| g.slots.iter().any(|(k, h)| k == a && on(h)));
    let any_on = g.slots.iter().find(|(_, h)| on(h)).map(|(k, _)| *k);
    let kept = g.active.filter(|a| g.slots.iter().any(|(k, _)| k == a));
    let first = g.slots.first().map(|(k, _)| *k);
    let next = by_out.or(cur).or(any_on).or(kept).or(first);
    let changed = g.active != next;
    g.active = next;
    changed
}

/// The headset of the receiver with this container: Some(off) — a wireless headset switched off with
/// its button while its receiver (and the Windows device) is there; None — no such receiver talked to.
pub fn off_at(container: &[u8; 16]) -> Option<bool> {
    let g = all();
    g.slots.iter().find(|(_, h)| h.connected && h.container.as_ref() == Some(container)).map(|(_, h)| !h.powered && h.is_wireless())
}

/// The connected and powered state of every receiver (to notice one that is not active).
pub fn power_sig() -> Vec<(u8, bool, bool)> {
    all().slots.iter().map(|(k, h)| (*k, h.connected, h.powered)).collect()
}

/// The slot of the active headset.
pub fn active() -> Option<u8> {
    all().active
}

/// The headset button's microphone mute of the receiver with this container: Some(muted) when that
/// headset reports it (a described one).
pub fn mic_muted_at(container: &[u8; 16]) -> Option<bool> {
    let g = all();
    g.slots.iter().find(|(_, h)| h.connected && h.powered && h.generic && h.mic_muted >= 0 && h.container.as_ref() == Some(container)).map(|(_, h)| h.mic_muted == 1)
}
