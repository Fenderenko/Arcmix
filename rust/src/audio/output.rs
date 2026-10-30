//! Output devices: what plays the Windows sound now and how it is connected (the window header),
//! and all outputs in the user's order — sound goes to the highest connected one. Windows reports
//! device changes itself (IMMNotificationClient) — no polling.
//!
//! The name comes from Windows (as in the sound settings), the connection type from the device
//! path: Bluetooth, USB or a built-in jack. A wireless headset with a receiver (2.4 GHz) shows up as
//! the USB receiver only, so the headset itself tells whether it is on.

use std::{collections::HashMap, time::Instant};

use windows::{
    Win32::{
        Devices::FunctionDiscovery::PKEY_DeviceInterface_FriendlyName,
        Foundation::PROPERTYKEY,
        Media::Audio::{
            AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, DEVICE_STATE_UNPLUGGED, EDataFlow, ERole, IDeviceTopology, IMMDevice,
            IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
            Endpoints::{IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl},
            eCapture, eCommunications, eConsole, eRender,
        },
        System::Com::{CLSCTX_ALL, CoTaskMemFree},
    },
    core::{PCWSTR, implement},
};

use super::{
    Engine, OutDevJson, OutInfo, com,
    outprio::{DEV_DISABLED, DEV_UNPLUGGED, OutEp, is_headset_name, is_virtual_name, merge_out_order, move_out, out_top, reorder_out},
    shared,
};
use crate::{config, headset, logf, server};

/// Device id → bus (the bus of a device does not change — Windows is asked once per device).
#[derive(Default)]
pub(super) struct DevWatch {
    bus: HashMap<String, String>,
    notify: Option<IMMNotificationClient>,
    /// the enumerator the notifications are registered with
    registered: Option<IMMDeviceEnumerator>,
    /// the highest available device last time
    top: String,
    /// the default device last time
    def: String,
    /// devices available last time (to tell a newly connected one)
    on: Vec<String>,
    started: Option<Instant>,
    /// the user put sound on the headset while its receiver says it is off: it counts as on until the
    /// receiver state (connected, powered — as it was then) changes or the user picks another device
    keep_hs: Option<(bool, bool)>,
}

/// The volume or mute of the current output changed (keys, Windows, another app): the card in the
/// window follows. Only a signal — the audio thread reads the device itself.
#[implement(IAudioEndpointVolumeCallback)]
struct VolNotify;

impl IAudioEndpointVolumeCallback_Impl for VolNotify_Impl {
    fn OnNotify(&self, _data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        super::refresh_out();
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DevNotify;

impl IMMNotificationClient_Impl for DevNotify_Impl {
    // in notifications audio devices must not be touched — only a signal
    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> windows::core::Result<()> {
        super::notify_dev_changed();
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        super::notify_dev_changed();
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        super::notify_dev_changed();
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _id: &PCWSTR) -> windows::core::Result<()> {
        if flow == eRender && role == eConsole {
            super::notify_dev_changed(); // the device in the window header
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

/// The device bus — "bt", "usb", "jack" (built-in sound card), "virtual" or "".
fn endpoint_bus(dev: &IMMDevice) -> String {
    unsafe {
        let Ok(topo) = dev.Activate::<IDeviceTopology>(CLSCTX_ALL, None) else { return String::new() };
        let Ok(conn) = topo.GetConnector(0) else { return String::new() };
        let Ok(p) = conn.GetDeviceIdConnectedTo() else { return String::new() };
        let id = p.to_string().unwrap_or_default().to_lowercase();
        CoTaskMemFree(Some(p.0 as *const _));
        if id.contains("bth") {
            "bt"
        } else if id.contains("usb") {
            "usb"
        } else if ["hdaudio", "intelaudio", "acpi", "pci"].iter().any(|k| id.contains(k)) {
            "jack"
        } else if id.contains("root#") || id.contains("swd#") {
            "virtual"
        } else {
            ""
        }
        .into()
    }
}

/// "3- Focusrite USB Audio" → "Focusrite USB Audio" (the driver instance number in Windows).
pub fn trim_inst_prefix(s: &str) -> &str {
    let i = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if i > 0 && s.as_bytes().get(i) == Some(&b'-') { s[i + 1..].trim() } else { s }
}

/// An audio device of the active headset: the same container as its receiver (by the name while no
/// receiver is found). With two headsets connected the name alone would take both for one.
pub(super) fn is_our_headset(id: &str, name: &str) -> bool {
    let c = headset::lock().container;
    match c {
        Some(c) => super::btbatt::endpoint_container(id) == Some(c),
        None => is_headset_name(name),
    }
}

/// The "Game" device of the same headset in place of its "Chat" one, when it is there and on; the device
/// itself otherwise.
fn game_side(list: &[OutDevJson], id: &str) -> String {
    let Some(d) = list.iter().find(|d| d.id == id) else { return id.into() };
    let cont = super::btbatt::endpoint_container(id);
    if d.is_virtual || cont.is_none() || !d.name.to_lowercase().contains("chat") {
        return id.into();
    }
    list.iter()
        .find(|x| x.id != id && x.state == "on" && !x.is_virtual && x.name.to_lowercase().contains("game") && super::btbatt::endpoint_container(&x.id) == cont)
        .map(|x| x.id.clone())
        .unwrap_or_else(|| id.into())
}

/// An audio device of a wireless headset switched off with its button (any receiver talked to).
fn off_at(id: &str, name: &str) -> bool {
    if let Some(off) = super::btbatt::endpoint_container(id).and_then(|c| headset::off_at(&c)) {
        return off;
    }
    headset::lock().container.is_none() && is_headset_name(name) && headset_off()
}

/// A wireless headset switched off with its button, though its receiver (and the Windows device)
/// is there.
fn headset_off() -> bool {
    let off = {
        let h = headset::lock();
        h.connected && !h.powered
    };
    off && headset::wireless()
}

impl Engine {
    /// (audio thread) The current Windows output device.
    pub(crate) fn refresh_out(&mut self) {
        let mut oi = OutInfo { batt: -1, ..Default::default() };
        if let Some(en) = self.en.get().cloned() {
            if let Ok(dev) = unsafe { en.GetDefaultAudioEndpoint(eRender, eConsole) } {
                oi.id = com::device_id(&dev);
                oi.name = com::friendly_name(&dev);
                oi.short = trim_inst_prefix(com::prop_string(&dev, &PKEY_DeviceInterface_FriendlyName).trim()).to_string();
                if oi.short.is_empty() {
                    oi.short = oi.name.clone();
                }
                let bus = match self.devs.bus.get(&oi.id) {
                    Some(b) => b.clone(),
                    None => {
                        let b = endpoint_bus(&dev);
                        if self.devs.bus.len() > 32 {
                            self.devs.bus.clear();
                        }
                        self.devs.bus.insert(oi.id.clone(), b.clone());
                        b
                    }
                };
                let ln = oi.name.to_lowercase();
                // the headset Arcmix talks to (its receiver says whether it is on); its receiver is the one
                // to talk to when several are connected
                let cont = super::btbatt::endpoint_container(&oi.id);
                crate::devs::hid::prefer(cont);
                let (known, on) = {
                    let h = headset::lock();
                    (h.connected && h.container.is_some() && h.container == cont, h.powered)
                };
                oi.headset = known || (headset::lock().container.is_none() && is_headset_name(&oi.name));
                // the Windows volume of this output, and a watch on it while it stays the output
                if let Some(epv) = com::endpoint_volume(&dev) {
                    unsafe {
                        oi.vol = (epv.GetMasterVolumeLevelScalar().unwrap_or(0.0) * 100.0).round() as i32;
                        oi.mute = epv.GetMute().map(|b| b.as_bool()).unwrap_or(false);
                    }
                    if self.out_vol.as_ref().is_none_or(|w| w.0 != oi.id) {
                        if let Some((_, old, cb)) = self.out_vol.take() {
                            let _ = unsafe { old.UnregisterControlChangeNotify(&cb) };
                        }
                        let cb: IAudioEndpointVolumeCallback = VolNotify.into();
                        if unsafe { epv.RegisterControlChangeNotify(&cb) }.is_ok() {
                            self.out_vol = Some((oi.id.clone(), epv, cb));
                        }
                    }
                }
                oi.conn = if bus == "bt" || ln.contains("hands-free") {
                    "bt"
                } else if bus == "virtual" || is_virtual_name(&oi.name) {
                    "virtual"
                } else if known {
                    match (headset::wireless(), on) {
                        (false, _) => "wire",
                        (true, true) => "24g",
                        (true, false) => "off",
                    }
                } else if bus == "jack" {
                    "wire"
                } else {
                    // USB: a cable and a 2.4 GHz receiver look the same to Windows
                    ""
                }
                .into();
                if oi.conn == "bt" {
                    oi.batt = self.bt_batt.get(&oi.id);
                }
                // the headset's "Chat" device next to the playing one (the same receiver, else by the names):
                // chat apps go there (Engine::route_app)
                let eps: Vec<(String, String)> = com::list_flow(&en, eRender).into_iter().filter(|d| !d.is_virtual).map(|d| (d.id, d.name)).collect();
                let same = |id: &str| cont.is_some() && super::btbatt::endpoint_container(id) == cont;
                let by_cont = (!ln.contains("chat")).then(|| eps.iter().find(|(id, name)| *id != oi.id && name.to_lowercase().contains("chat") && same(id))).flatten();
                if let Some((id, name)) = by_cont.or_else(|| super::outprio::chat_endpoint(&eps, &oi.id, &oi.name)) {
                    (self.hw_chat, oi.chat) = (id.clone(), name.clone());
                }
            }
        }
        if oi.chat.is_empty() {
            self.hw_chat.clear();
        }
        self.sync_comm(&oi.id);
        let (changed, tray) = {
            let mut sh = shared();
            let ch = sh.out != oi;
            // the tray shows the battery of the device that plays (not every volume change)
            let tr = sh.out.id != oi.id || sh.out.headset != oi.headset || sh.out.batt != oi.batt;
            sh.out = oi;
            (ch, tr)
        };
        if changed {
            server::notify_subs();
        }
        if tray {
            crate::util::post_ui(crate::WM_MIC_CHG, 0);
        }
    }

    /// (audio thread) Subscribe to device notifications (and again when the enumerator was recreated).
    pub(super) fn watch_devices(&mut self) {
        let Some(en) = self.en.get().cloned() else { return };
        if self.devs.registered.as_ref() == Some(&en) {
            return;
        }
        let obj = self.devs.notify.get_or_insert_with(|| DevNotify.into()).clone();
        if let Err(e) = unsafe { en.RegisterEndpointNotificationCallback(&obj) } {
            logf!("devices: RegisterEndpointNotificationCallback {e}");
            return;
        }
        self.devs.registered = Some(en);
    }

    /// All output devices — active, disabled in Windows and with an empty jack.
    fn list_outputs(&mut self) -> Option<Vec<OutEp>> {
        let en = self.en.get()?.clone();
        let devs = com::devices(&en, eRender, DEVICE_STATE(DEVICE_STATE_ACTIVE.0 | DEVICE_STATE_DISABLED.0 | DEVICE_STATE_UNPLUGGED.0));
        Some(
            devs.iter()
                .filter_map(|d| {
                    let id = com::device_id(d);
                    (!id.is_empty()).then(|| OutEp { id, name: com::friendly_name(d), state: com::device_state(d) })
                })
                .collect(),
        )
    }

    /// Hardware ChatMix: while the headset's "Game" side plays, Windows' default communications device is its
    /// "Chat" (chat apps that use that device land where the dial mixes them as chat); otherwise it is the
    /// output again — only when it is still the one set here.
    fn sync_comm(&mut self, out_id: &str) {
        let Some(en) = self.en.get().cloned() else { return };
        let c = config::get();
        let want = if c.chat_to_hs && !self.hw_chat.is_empty() && crate::devices::hardware_chatmix() { self.hw_chat.clone() } else { String::new() };
        let cur = com::default_role_id(&en, eRender, eCommunications);
        if !want.is_empty() {
            if cur != want && com::set_default_roles(&want, &[2]) {
                logf!("communications device → the headset's Chat");
                self.comm_ours = want;
            }
        } else if !self.comm_ours.is_empty() {
            if cur == self.comm_ours && !out_id.is_empty() && com::set_default_roles(out_id, &[2]) {
                logf!("communications device → the output again");
            }
            self.comm_ours.clear();
        }
    }

    /// (audio thread) The list for the window and, if on, switching to the highest available
    /// device. force — apply the order even without changes (switching was just turned on).
    pub(crate) fn refresh_outs(&mut self, force: bool) {
        let started = *self.devs.started.get_or_insert_with(Instant::now);
        let Some(eps) = self.list_outputs() else { return };
        let Some(en) = self.en.get().cloned() else { return };
        let c = config::get();
        let def = com::default_id(&en, eRender);
        let (order, changed) = merge_out_order(&c.out_devs, &eps, &[c.prev_out.clone(), def.clone()]);
        if changed || !c.prev_out.is_empty() {
            let o = order.clone();
            config::set(|c| {
                c.out_devs = o;
                c.prev_out.clear();
            });
        }
        // the receiver may report the headset off while it plays (after a PC restart, until the headset
        // is switched off and on): a device the user picked by hand stays, priority does not move it
        let rx = {
            let h = headset::lock();
            (h.connected, h.powered)
        };
        if self.devs.keep_hs.is_some_and(|k| k != rx) {
            self.devs.keep_hs = None;
            logf!("output priority: the headset state changed, the order applies again");
        }
        let moved = !self.devs.def.is_empty() && self.devs.def != def;
        let def_is_hs = eps.iter().any(|e| e.id == def && is_our_headset(&e.id, &e.name));
        // the previous device is still there: the change was the user's, not Windows' fallback
        let prev_there = eps.iter().any(|e| e.id == self.devs.def && e.state != DEV_DISABLED && e.state != DEV_UNPLUGGED);
        let mut picked_away = false;
        if moved && self.devs.keep_hs.is_some() && !def_is_hs {
            self.devs.keep_hs = None;
            picked_away = true;
            logf!("output priority: another device picked, the headset follows its receiver again");
        } else if moved && def_is_hs && prev_there && headset_off() && self.devs.keep_hs.is_none() {
            self.devs.keep_hs = Some(rx);
            logf!("output priority: the headset picked by hand while its receiver says it is off — kept");
        }
        // EQ Windows everywhere: the module on the present devices that lack it (one prompt for all)
        let present: Vec<(String, String)> = eps.iter().filter(|e| e.state == DEVICE_STATE_ACTIVE.0).map(|e| (e.id.clone(), e.name.clone())).collect();
        crate::apomod::auto_install(&present);
        let kept = self.devs.keep_hs.is_some();
        let list: Vec<OutDevJson> = order
            .iter()
            .map(|d| {
                let state = match eps.iter().find(|e| e.id == d.id) {
                    None => "absent",
                    Some(e) if e.state == DEV_DISABLED => "disabled",
                    Some(e) if e.state == DEV_UNPLUGGED => "unplugged",
                    Some(e) if off_at(&e.id, &e.name) && !(kept && is_our_headset(&e.id, &e.name)) => "hsoff",
                    Some(_) => "on",
                };
                OutDevJson { id: d.id.clone(), name: d.name.clone(), state: state.into(), default: d.id == def, is_virtual: super::outprio::is_virtual_name(&d.name) }
            })
            .collect();
        let same = {
            let mut sh = shared();
            let same = sh.outs == list;
            sh.outs = list.clone();
            same
        };
        if !same {
            server::notify_subs();
        }
        // the playing device vanished or was disabled (Windows picked some other) — by the order too
        let on = |id: &str| list.iter().find(|d| d.id == id).is_some_and(|d| d.state == "on");
        let lost = !self.devs.def.is_empty() && self.devs.def != def && !on(&self.devs.def);
        self.devs.def = def.clone();
        // a physical device that has just become available (not on the first look)
        let now_on: Vec<String> = list.iter().filter(|d| d.state == "on").map(|d| d.id.clone()).collect();
        let fresh = (!self.devs.on.is_empty()).then(|| list.iter().find(|d| d.state == "on" && !d.is_virtual && !self.devs.on.contains(&d.id)).map(|d| d.id.clone())).flatten();
        self.devs.on = now_on;
        // switching: while the app just started, the headset state is unknown yet — wait
        if !c.auto_switch || started.elapsed().as_secs_f64() < 4.0 {
            return;
        }
        let top = out_top(&list);
        if picked_away {
            // the device the user just picked stays; the order applies from the next change on
            self.devs.top = top;
            return;
        }
        // the playing device gone: the next by the order, or where Windows put it (outOnLost); a new one:
        // by the order — when it is above the others, always, or never (outOnNew)
        let gone = !on(&def) || lost;
        let pick = if force {
            top.clone()
        } else if gone {
            if c.out_on_lost { top.clone() } else { String::new() }
        } else if let Some(f) = fresh.filter(|_| c.out_on_new == "always") {
            f
        } else if c.out_on_new == "order" && top != self.devs.top {
            top.clone()
        } else {
            String::new()
        };
        // a hardware ChatMix headset plays on its "Game" side; "Chat" is for the chat apps (route_app)
        let pick = game_side(&list, &pick);
        if !pick.is_empty() && pick != def {
            let name = list.iter().find(|d| d.id == pick).map(|d| d.name.clone()).unwrap_or_default();
            if com::set_default_endpoint(&pick) {
                logf!("output priority: sound → {name}");
            }
        }
        self.devs.top = top;
    }

    /// (from the window) up | down | order (ids — the new order) | on | off | forget.
    pub(super) fn out_dev_op(&mut self, op: &str, id: &str, ids: Vec<String>) {
        match op {
            "order" => config::set(|c| c.out_devs = reorder_out(&c.out_devs, &ids)),
            "up" | "down" => {
                let d = if op == "up" { -1 } else { 1 };
                config::set(|c| c.out_devs = move_out(&c.out_devs, id, d));
            }
            "forget" => config::set(|c| c.out_devs.retain(|x| x.id != id)),
            "on" | "off" => {
                if com::set_endpoint_enabled(id, op == "on") {
                    logf!("devices: output {} by user", if op == "on" { "enabled" } else { "disabled" });
                }
            }
            _ => {}
        }
        self.refresh_out();
        self.refresh_outs(false);
    }

    /// (audio thread) The headset was switched on — its microphone becomes the default; off — the
    /// previous one comes back. (Output follows the device order.)
    pub(super) fn switch_mic(&mut self, on: bool) {
        let c = config::get();
        let Some(en) = self.en.get().cloned() else { return };
        if !c.auto_switch || !c.switch_mic {
            return;
        }
        let capture = com::list_flow(&en, eCapture);
        let Some(hs_id) = capture.iter().find(|d| !d.is_virtual && is_our_headset(&d.id, &d.name)).map(|d| d.id.clone()) else { return };
        let mut prev = c.prev_mic.clone();
        let cur = com::default_id(&en, eCapture);
        if capture.iter().any(|d| d.id == cur && d.is_virtual) {
            // apps listen to the processed microphone in the cable: the cable stays Windows' default and the
            // app's own microphone (processing, "listen to yourself") moves instead
            let mine = c.mic_dev.clone();
            let mut prev = c.prev_mic.clone();
            let next = if on {
                if mine == hs_id {
                    return;
                }
                prev = mine;
                logf!("auto switch: app mic → headset");
                hs_id.clone()
            } else if mine == hs_id && !prev.is_empty() && prev != hs_id && capture.iter().any(|d| d.id == prev) {
                logf!("auto switch: app mic → previous device");
                prev.clone()
            } else {
                return;
            };
            config::set(|c| {
                c.mic_dev = next;
                c.prev_mic = prev;
            });
            self.refresh_mic();
            std::thread::spawn(|| crate::router::sync(&config::get()));
            crate::server::notify_subs();
            return;
        }
        if on {
            if cur != hs_id {
                prev = cur;
                if com::set_default_endpoint(&hs_id) {
                    logf!("auto switch: mic → headset");
                }
            }
        } else if cur == hs_id && !prev.is_empty() && prev != hs_id && capture.iter().any(|d| d.id == prev) && com::set_default_endpoint(&prev) {
            logf!("auto switch: mic → previous device");
        }
        if prev != c.prev_mic {
            config::set(|c| c.prev_mic = prev);
        }
    }
}
