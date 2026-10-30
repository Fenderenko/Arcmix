//! Battery of Bluetooth headphones, as Windows shows it in Settings → Bluetooth & devices: the
//! battery property of the Bluetooth device node in the same container (the same physical device)
//! as the audio endpoint.

use windows::{
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::{
                CM_GETIDLIST_FILTER_PRESENT, CM_Get_DevNode_PropertyW, CM_Get_Device_ID_List_SizeW, CM_Get_Device_ID_ListW, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW,
                CR_SUCCESS,
            },
            Properties::{DEVPKEY_Device_ContainerId, DEVPROPTYPE},
        },
        Foundation::DEVPROPKEY,
    },
    core::{GUID, PCWSTR},
};

use std::time::{Duration, Instant};

use crate::util;

/// The battery of the current Bluetooth output: the device search runs once per endpoint, the value
/// is read again at most once a minute (Windows itself updates it rarely).
#[derive(Default)]
pub struct Cache {
    endpoint: String,
    inst: Option<String>,
    read_at: Option<Instant>,
    pct: i32,
}

impl Cache {
    /// (audio thread) Percent, or -1 when Windows does not know it.
    pub fn get(&mut self, endpoint: &str) -> i32 {
        if self.endpoint != endpoint {
            *self = Cache { endpoint: endpoint.to_string(), inst: find(endpoint), read_at: None, pct: -1 };
        }
        if self.read_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
            self.read_at = Some(Instant::now());
            self.pct = self.inst.as_deref().and_then(read).map_or(-1, i32::from);
        }
        self.pct
    }
}

/// {104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2: battery percent of a Bluetooth device (one byte).
const BATTERY: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2 };

fn devnode(id: &str) -> Option<u32> {
    let w = util::wide(id);
    let mut dn = 0u32;
    (unsafe { CM_Locate_DevNodeW(&mut dn, PCWSTR(w.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) } == CR_SUCCESS).then_some(dn)
}

fn prop(dn: u32, key: &DEVPROPKEY, buf: &mut [u8]) -> Option<usize> {
    let mut t = DEVPROPTYPE::default();
    let mut n = buf.len() as u32;
    (unsafe { CM_Get_DevNode_PropertyW(dn, key, &mut t, Some(buf.as_mut_ptr()), &mut n, 0) } == CR_SUCCESS).then_some(n as usize)
}

/// (any thread) The container (the physical device) of a device node: a USB receiver and its audio
/// devices in Windows share it.
pub(crate) fn container(dn: u32) -> Option<[u8; 16]> {
    let mut b = [0u8; 16];
    (prop(dn, &DEVPKEY_Device_ContainerId, &mut b)? == 16).then_some(b)
}

/// (any thread) The container of an audio endpoint ("{0.0.0.00000000}.{…}"), remembered per endpoint.
pub(crate) fn endpoint_container(id: &str) -> Option<[u8; 16]> {
    static CACHE: std::sync::Mutex<Option<std::collections::HashMap<String, Option<[u8; 16]>>>> = std::sync::Mutex::new(None);
    if id.is_empty() {
        return None;
    }
    let mut g = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let m = g.get_or_insert_with(Default::default);
    if let Some(c) = m.get(id) {
        return *c;
    }
    let c = devnode(&format!(r"SWD\MMDEVAPI\{id}")).and_then(container);
    if m.len() > 64 {
        m.clear();
    }
    m.insert(id.to_string(), c);
    c
}

/// (any thread) Battery percent of the device instance found by `find`.
pub fn read(inst: &str) -> Option<u8> {
    let mut b = [0u8; 4];
    let n = prop(devnode(inst)?, &BATTERY, &mut b)?;
    (n >= 1 && b[0] <= 100).then_some(b[0])
}

/// (any thread; walks all present devices, so the caller keeps the result) The device instance
/// with a battery in the container of an audio endpoint ("{0.0.0.00000000}.{…}").
pub fn find(endpoint_id: &str) -> Option<String> {
    let want = container(devnode(&format!(r"SWD\MMDEVAPI\{endpoint_id}"))?)?;
    present_ids().into_iter().find(|id| devnode(id).is_some_and(|dn| read(id).is_some() && container(dn) == Some(want)))
}

fn present_ids() -> Vec<String> {
    let mut len = 0u32;
    if unsafe { CM_Get_Device_ID_List_SizeW(&mut len, PCWSTR::null(), CM_GETIDLIST_FILTER_PRESENT) } != CR_SUCCESS || len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u16; len as usize];
    if unsafe { CM_Get_Device_ID_ListW(PCWSTR::null(), &mut buf, CM_GETIDLIST_FILTER_PRESENT) } != CR_SUCCESS {
        return Vec::new();
    }
    buf.split(|c| *c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Devices with a battery that Windows knows: cargo test --release -- --ignored bt_batteries --nocapture
    #[test]
    #[ignore]
    fn bt_batteries() {
        for id in present_ids() {
            if let Some(p) = read(&id) {
                println!("{p:>3}%  {id}");
            }
        }
        for id in present_ids().iter().filter(|id| id.to_uppercase().starts_with(r"SWD\MMDEVAPI") || read(id).is_some()) {
            println!("{id} {:?}", devnode(id).and_then(container));
        }
        // playback endpoints and the battery device found for each
        for id in present_ids().iter().filter_map(|id| id.strip_prefix(r"SWD\MMDEVAPI\")).filter(|e| e.starts_with("{0.0.0.")) {
            if let Some(inst) = find(id) {
                println!("{id} -> {inst} {:?}", read(&inst));
            }
        }
    }
}
