//! Headset state, shared by the HID threads (writers) and the UI thread and the server (readers).

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
}

static HS: Mutex<Option<Headset>> = Mutex::new(None);

pub struct Guard(MutexGuard<'static, Option<Headset>>);

impl std::ops::Deref for Guard {
    type Target = Headset;
    fn deref(&self) -> &Headset {
        self.0.as_ref().unwrap()
    }
}

impl std::ops::DerefMut for Guard {
    fn deref_mut(&mut self) -> &mut Headset {
        self.0.as_mut().unwrap()
    }
}

pub fn lock() -> Guard {
    let mut g = HS.lock().unwrap_or_else(|p| p.into_inner());
    if g.is_none() {
        *g = Some(Headset { mic_muted: -1, ..Default::default() });
    }
    Guard(g)
}

pub fn snapshot() -> Headset {
    lock().clone()
}

/// The headset works through a wireless receiver (not by cable).
pub fn wireless() -> bool {
    let h = lock();
    !h.generic || h.generic_wireless
}
