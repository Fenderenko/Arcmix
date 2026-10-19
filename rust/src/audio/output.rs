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
            DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, DEVICE_STATE_UNPLUGGED, EDataFlow, ERole, IDeviceTopology, IMMDevice, IMMDeviceEnumerator, IMMNotificationClient,
            IMMNotificationClient_Impl, eCapture, eConsole, eRender,
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
    started: Option<Instant>,
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
                // the headset Arcmix talks to (its receiver says whether it is on)
                let (known, on) = {
                    let h = headset::lock();
                    let n = h.dev_name.to_lowercase();
                    (h.connected && !n.is_empty() && ln.contains(&n), h.powered)
                };
                oi.headset = known || is_headset_name(&oi.name);
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
            }
        }
        let changed = {
            let mut sh = shared();
            let ch = sh.out != oi;
            sh.out = oi;
            ch
        };
        if changed {
            server::notify_subs();
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
        let hs_off = headset_off();
        let list: Vec<OutDevJson> = order
            .iter()
            .map(|d| {
                let state = match eps.iter().find(|e| e.id == d.id) {
                    None => "absent",
                    Some(e) if e.state == DEV_DISABLED => "disabled",
                    Some(e) if e.state == DEV_UNPLUGGED => "unplugged",
                    Some(e) if hs_off && is_headset_name(&e.name) => "hsoff",
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
        // switching: while the app just started, the headset state is unknown yet — wait
        if !c.auto_switch || started.elapsed().as_secs_f64() < 4.0 {
            return;
        }
        let top = out_top(&list);
        if !top.is_empty() && top != def && (force || top != self.devs.top || !on(&def) || lost) {
            let name = list.iter().find(|d| d.id == top).map(|d| d.name.clone()).unwrap_or_default();
            if com::set_default_endpoint(&top) {
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
        let Some(hs_id) = capture.iter().find(|d| is_headset_name(&d.name)).map(|d| d.id.clone()) else { return };
        let mut prev = c.prev_mic.clone();
        let cur = com::default_id(&en, eCapture);
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
