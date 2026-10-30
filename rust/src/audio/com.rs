//! COM helpers for the audio engine: the device enumerator, device ids and names, walking audio
//! sessions, endpoint volume, and the undocumented Windows interfaces for the default device
//! (IPolicyConfig) and per-app output (IAudioPolicyConfigFactory).

use std::ffi::c_void;

use windows::{
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Foundation::PROPERTYKEY,
        Media::Audio::{
            AudioSessionState, DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, EDataFlow, ERole, IAudioSessionControl, IAudioSessionControl2, IAudioSessionEnumerator, IAudioSessionManager2,
            IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eCapture, eConsole, eRender,
        },
        Media::Audio::Endpoints::IAudioEndpointVolume,
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ},
            WinRT::RoGetActivationFactory,
        },
    },
    core::{GUID, HRESULT, HSTRING, IUnknown, Interface, PCWSTR},
};

use super::AudioDev;
use super::outprio::{is_headset_name, is_virtual_name};

/// The device enumerator, created once per thread (it is free-threaded, but each thread keeps its
/// own so the COM initialization order does not matter).
#[derive(Default)]
pub struct Enum(Option<IMMDeviceEnumerator>);

impl Enum {
    pub fn get(&mut self) -> Option<&IMMDeviceEnumerator> {
        if self.0.is_none() {
            match unsafe { CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
                Ok(e) => self.0 = Some(e),
                Err(e) => crate::logf!("MMDeviceEnumerator: {e}"),
            }
        }
        self.0.as_ref()
    }

    pub fn reset(&mut self) {
        self.0 = None;
    }
}

pub fn device_id(dev: &IMMDevice) -> String {
    unsafe {
        match dev.GetId() {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const c_void));
                s
            }
            Err(_) => String::new(),
        }
    }
}

pub fn prop_string(dev: &IMMDevice, key: &PROPERTYKEY) -> String {
    unsafe {
        let Ok(store) = dev.OpenPropertyStore(STGM_READ) else { return String::new() };
        match store.GetValue(key) {
            Ok(v) => v.to_string(),
            Err(_) => String::new(),
        }
    }
}

pub fn friendly_name(dev: &IMMDevice) -> String {
    prop_string(dev, &PKEY_Device_FriendlyName)
}

pub fn get_device(en: &IMMDeviceEnumerator, id: &str) -> Option<IMMDevice> {
    if id.is_empty() {
        return None;
    }
    unsafe { en.GetDevice(&HSTRING::from(id)) }.ok()
}

/// Windows' default device of a flow (eConsole role) — "" when there is none.
pub fn default_id(en: &IMMDeviceEnumerator, flow: EDataFlow) -> String {
    default_role_id(en, flow, eConsole)
}

/// Windows' default device of a flow for a role — "" when there is none.
pub fn default_role_id(en: &IMMDeviceEnumerator, flow: EDataFlow, role: ERole) -> String {
    unsafe { en.GetDefaultAudioEndpoint(flow, role) }.map(|d| device_id(&d)).unwrap_or_default()
}

/// Devices of a flow in the given states.
pub fn devices(en: &IMMDeviceEnumerator, flow: EDataFlow, states: DEVICE_STATE) -> Vec<IMMDevice> {
    let mut out = Vec::new();
    unsafe {
        let Ok(coll) = en.EnumAudioEndpoints(flow, states) else { return out };
        let n = coll.GetCount().unwrap_or(0);
        for i in 0..n {
            if let Ok(d) = coll.Item(i) {
                out.push(d);
            }
        }
    }
    out
}

pub fn device_state(dev: &IMMDevice) -> u32 {
    unsafe { dev.GetState() }.map(|s| s.0).unwrap_or(0)
}

/// One audio session on a playback device.
pub struct Session {
    pub pid: u32,
    /// unique session instance id
    pub key: String,
    pub state: AudioSessionState,
    pub sav: ISimpleAudioVolume,
    pub ctl: IAudioSessionControl,
    pub dev_id: String,
    /// "System sounds"
    pub sys: bool,
}

/// All sessions on all active playback devices; None — the enumerator failed (create it anew).
pub fn sessions(en: &IMMDeviceEnumerator) -> Option<Vec<Session>> {
    let mut out = Vec::new();
    let coll = unsafe { en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }.ok()?;
    let n = unsafe { coll.GetCount() }.unwrap_or(0);
    for i in 0..n {
        if let Ok(dev) = unsafe { coll.Item(i) } {
            let dev_id = device_id(&dev);
            sessions_of(&dev, &dev_id, &mut out);
        }
    }
    Some(out)
}

pub fn sessions_of(dev: &IMMDevice, dev_id: &str, out: &mut Vec<Session>) {
    unsafe {
        let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { return };
        let Ok(se): windows::core::Result<IAudioSessionEnumerator> = mgr.GetSessionEnumerator() else { return };
        let n = se.GetCount().unwrap_or(0);
        for i in 0..n {
            let Ok(ctl) = se.GetSession(i) else { continue };
            let (mut pid, mut key, mut sys) = (0u32, String::new(), false);
            if let Ok(c2) = ctl.cast::<IAudioSessionControl2>() {
                pid = c2.GetProcessId().unwrap_or(0);
                sys = c2.IsSystemSoundsSession() == HRESULT(0); // S_OK — yes
                if let Ok(p) = c2.GetSessionInstanceIdentifier() {
                    key = p.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(p.0 as *const c_void));
                }
            }
            if key.is_empty() {
                key = pid.to_string();
            }
            let state = ctl.GetState().unwrap_or_default();
            if let Ok(sav) = ctl.cast::<ISimpleAudioVolume>() {
                out.push(Session { pid, key, state, sav, ctl, dev_id: dev_id.to_string(), sys });
            }
        }
    }
}

pub fn get_vol(sav: &ISimpleAudioVolume) -> f32 {
    unsafe { sav.GetMasterVolume() }.unwrap_or(0.0)
}

pub fn set_vol(sav: &ISimpleAudioVolume, v: f32) {
    unsafe {
        let _ = sav.SetMasterVolume(v.clamp(0.0, 1.0), std::ptr::null());
    }
}

pub fn endpoint_volume(dev: &IMMDevice) -> Option<IAudioEndpointVolume> {
    unsafe { dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()
}

// ---------- device lists ----------

/// Active playback (eRender) or recording (eCapture) devices: physical ones first, then virtual cables.
pub fn list_flow(en: &IMMDeviceEnumerator, flow: EDataFlow) -> Vec<AudioDev> {
    let def = default_id(en, flow);
    let mut l: Vec<AudioDev> = devices(en, flow, DEVICE_STATE_ACTIVE)
        .iter()
        .filter_map(|d| {
            let id = device_id(d);
            if id.is_empty() {
                return None;
            }
            let name = friendly_name(d);
            Some(AudioDev { is_virtual: is_virtual_name(&name), default: id == def, id, name, ..Default::default() })
        })
        .collect();
    l.sort_by_key(|d| d.is_virtual); // stable: Windows' order inside each kind
    l
}

pub fn paren_part(n: &str) -> String {
    match (n.rfind('('), n.rfind(')')) {
        (Some(i), Some(j)) if j > i => n[i + 1..j].trim().to_string(),
        _ => String::new(),
    }
}

/// Active playback and recording devices; cables get their pair and group, the microphone behind
/// "" gets `auto`.
pub fn list_audio_devices(en: &IMMDeviceEnumerator) -> (Vec<AudioDev>, Vec<AudioDev>) {
    let mut render = list_flow(en, eRender);
    let mut capture = list_flow(en, eCapture);
    if let Some(id) = auto_mic(&capture) {
        for c in capture.iter_mut() {
            c.auto = c.id == id;
        }
    }
    // the recording ends of cables, also those switched off in Windows (the pair is known then too)
    let mut ends: Vec<AudioDev> = capture.iter().filter(|c| c.is_virtual).cloned().collect();
    ends.extend(off_cables(en, eCapture));
    let mut groups: std::collections::HashMap<String, usize> = Default::default();
    for r in render.iter_mut().filter(|r| r.is_virtual) {
        let pp = paren_part(&r.name);
        r.group = pp.clone();
        *groups.entry(pp.clone()).or_default() += 1;
        if let Some(c) = cable_pair(&r.name, &ends) {
            r.pair = c.name.clone();
        }
    }
    // VB-CABLE also has "CABLE In 16ch" — the same cable (16 channels); both cannot be used at once,
    // so the lists keep the usual input
    render.retain(|d| {
        if d.is_virtual && !d.group.is_empty() && groups.get(&d.group).copied().unwrap_or(0) > 1 && d.name.to_lowercase().contains("16ch") {
            *groups.get_mut(&d.group).unwrap() -= 1;
            return false;
        }
        true
    });
    (render, capture)
}

/// Virtual cables switched off in Windows ("Disable" in the sound settings), marked `off`.
pub fn off_cables(en: &IMMDeviceEnumerator, flow: EDataFlow) -> Vec<AudioDev> {
    devices(en, flow, DEVICE_STATE_DISABLED)
        .iter()
        .filter_map(|d| {
            let (id, name) = (device_id(d), friendly_name(d));
            (!id.is_empty() && is_virtual_name(&name)).then_some(AudioDev { id, name, is_virtual: true, off: true, ..Default::default() })
        })
        .collect()
}

/// The recording end of a cable: of the same name (Virtual Audio Cable: "Line 1 (Virtual Audio Cable)"
/// both ways), "… Input" ↔ "… Output" (VB-CABLE, Voicemeeter), else the first of the same cable (the
/// part in brackets).
pub fn cable_pair<'a>(render: &str, ends: &'a [AudioDev]) -> Option<&'a AudioDev> {
    let output = render.replacen(" Input", " Output", 1);
    ends.iter().find(|c| c.name == render).or_else(|| ends.iter().find(|c| c.name == output)).or_else(|| {
        let pp = paren_part(render);
        ends.iter().find(|c| !pp.is_empty() && paren_part(&c.name) == pp)
    })
}

/// The recording device "" (as in Windows) stands for: Windows' default, unless it is a virtual
/// cable — apps listen to a cable, it is not a microphone; then the headset microphone, otherwise
/// the first real one. With virtual devices only, Windows' default is kept.
pub fn auto_mic(capture: &[AudioDev]) -> Option<String> {
    let (mut def, mut headset, mut first) = (None, None, None);
    for d in capture {
        if d.default && !d.is_virtual {
            return Some(d.id.clone());
        }
        if d.default {
            def = Some(d.id.clone());
        } else if !d.is_virtual && headset.is_none() && is_headset_name(&d.name) {
            headset = Some(d.id.clone());
        }
        if first.is_none() && !d.is_virtual {
            first = Some(d.id.clone());
        }
    }
    headset.or(first).or(def)
}

pub fn auto_mic_id(en: &IMMDeviceEnumerator) -> String {
    auto_mic(&list_flow(en, eCapture)).unwrap_or_default()
}

// ---------- undocumented interfaces (raw vtable calls) ----------

/// A raw COM object kept alive by its IUnknown.
pub struct Raw(IUnknown);

impl Raw {
    fn ptr(&self) -> *mut c_void {
        self.0.as_raw()
    }

    /// The method at vtable index idx, typed as F.
    unsafe fn method<F: Copy>(&self, idx: usize) -> F {
        unsafe {
            let vt = *(self.ptr() as *const *const usize);
            std::mem::transmute_copy(&*vt.add(idx))
        }
    }
}

fn query(unk: &IUnknown, iid: &GUID) -> Option<Raw> {
    let mut p: *mut c_void = std::ptr::null_mut();
    unsafe {
        if unk.query(iid, &mut p).is_err() || p.is_null() {
            return None;
        }
        Some(Raw(IUnknown::from_raw(p)))
    }
}

const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
const IID_POLICY_CONFIG: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);

/// IPolicyConfig: the interface the Windows sound settings use (undocumented, stable since Windows 7).
fn policy_config() -> Option<Raw> {
    let unk = unsafe { CoCreateInstance::<_, IUnknown>(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL) };
    match unk {
        Ok(u) => query(&u, &IID_POLICY_CONFIG),
        Err(e) => {
            crate::logf!("IPolicyConfig: {e}");
            None
        }
    }
}

/// The same as "Set as default" in the sound settings (for all roles).
pub fn set_default_endpoint(id: &str) -> bool {
    set_default_roles(id, &[0, 1, 2])
}

/// "Set as default" for some roles only (0 eConsole, 1 eMultimedia, 2 eCommunications).
pub fn set_default_roles(id: &str, roles: &[u32]) -> bool {
    let Some(pc) = policy_config() else { return false };
    let h = HSTRING::from(id);
    let mut ok = true;
    for &role in roles {
        let r = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, PCWSTR, u32) -> HRESULT = pc.method(13); // SetDefaultEndpoint
            f(pc.ptr(), PCWSTR(h.as_ptr()), role)
        };
        if r.is_err() {
            crate::logf!("SetDefaultEndpoint: 0x{:08x}", r.0 as u32);
            ok = false;
        }
    }
    ok
}

/// "Enable" / "Disable" a device in the Windows sound settings.
pub fn set_endpoint_enabled(id: &str, on: bool) -> bool {
    let Some(pc) = policy_config() else { return false };
    let h = HSTRING::from(id);
    let r = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT = pc.method(14); // SetEndpointVisibility
        f(pc.ptr(), PCWSTR(h.as_ptr()), on as i32)
    };
    if r.is_err() {
        crate::logf!("devices: SetEndpointVisibility({on}) 0x{:08x}", r.0 as u32);
    }
    r.is_ok()
}

/// Routing apps to a device by category — the same as "Settings → Sound → Volume mixer → app
/// output device". Windows has no documented API for it; the same internal interface as the
/// well-known EarTrumpet is used. Not available (old or changed Windows) — the feature is off.
pub struct AppRouting(Option<Raw>);

const IID_POLICY_FACTORY: GUID = GUID::from_u128(0xab3d4648_e242_459f_b02f_541c70306324); // Windows 10 21H2+, 11
const IID_POLICY_FACTORY_OLD: GUID = GUID::from_u128(0x2a59116d_6c4f_45e0_a74f_707e3fef9258); // older Windows 10
/// IInspectable (6) + 19 methods before SetPersistedDefaultAudioEndpoint
const POLICY_SET_PERSISTED: usize = 25;

impl AppRouting {
    /// Only on the audio engine thread (COM MTA is initialized there).
    pub fn new() -> Self {
        let name = HSTRING::from("Windows.Media.Internal.AudioPolicyConfig");
        let f = unsafe { RoGetActivationFactory::<IUnknown>(&name) }.ok();
        let raw = f.and_then(|u| query(&u, &IID_POLICY_FACTORY).or_else(|| query(&u, &IID_POLICY_FACTORY_OLD)));
        if raw.is_some() {
            crate::logf!("app output routing available");
        } else {
            crate::logf!("app output routing is not available on this Windows");
        }
        AppRouting(raw)
    }

    pub fn ok(&self) -> bool {
        self.0.is_some()
    }

    /// Routes the app (process pid) to device dev_id; "" — back to the default.
    pub fn set(&self, pid: u32, dev_id: &str) -> bool {
        let Some(f) = &self.0 else { return false };
        let full = if dev_id.is_empty() { String::new() } else { format!(r"\\?\SWD#MMDEVAPI#{dev_id}#{{e6327cad-dcec-4949-ae8a-991e976a79d2}}") };
        let h = HSTRING::from(full.as_str());
        let hraw: *mut c_void = if full.is_empty() { std::ptr::null_mut() } else { unsafe { std::mem::transmute_copy(&h) } };
        let mut ok = true;
        for role in [1i32, 0, 2] {
            // eMultimedia, eConsole, eCommunications (chat apps that use the communications device)
            let r = unsafe {
                let m: unsafe extern "system" fn(*mut c_void, u32, i32, i32, *mut c_void) -> HRESULT = f.method(POLICY_SET_PERSISTED);
                m(f.ptr(), pid, eRender.0, role, hraw)
            };
            if r.is_err() && role != 2 {
                crate::logf!("route app {pid} → {dev_id:?}: 0x{:08x}", r.0 as u32);
                ok = false;
            }
        }
        drop(h);
        ok
    }
}

#[allow(dead_code)]
pub fn role_console() -> ERole {
    eConsole
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_mic_rules() {
        let d = |id: &str, name: &str, virt: bool, def: bool| AudioDev { id: id.into(), name: name.into(), is_virtual: virt, default: def, ..Default::default() };
        let cable = |def| d("cable", "CABLE Output (VB-Audio Virtual Cable)", true, def);
        let focus = |def| d("focus", "Analogue 1 + 2 (3- Focusrite USB Audio)", false, def);
        let arctis = d("arctis", "Microphone (Arctis 7+)", false, false);
        assert_eq!(auto_mic(&[cable(false), focus(true), arctis.clone()]).as_deref(), Some("focus"), "real default kept");
        assert_eq!(auto_mic(&[cable(true), focus(false), arctis.clone()]).as_deref(), Some("arctis"), "cable default → headset");
        assert_eq!(auto_mic(&[cable(true), focus(false)]).as_deref(), Some("focus"), "cable default → first real");
        assert_eq!(auto_mic(&[cable(true)]).as_deref(), Some("cable"), "only cables → Windows default");
        assert_eq!(auto_mic(&[cable(false), focus(false), arctis]).as_deref(), Some("arctis"), "no default → headset");
        assert_eq!(auto_mic(&[]), None);
    }

    /// The recording end of each cable: Virtual Audio Cable lines share the part in brackets.
    #[test]
    fn cable_pairs() {
        let e = |n: &str| AudioDev { id: n.into(), name: n.into(), is_virtual: true, ..Default::default() };
        let ends = [e("Line 1 (Virtual Audio Cable)"), e("Line 2 (Virtual Audio Cable)"), e("CABLE Output (VB-Audio Virtual Cable)"), e("VoiceMeeter Output (VB-Audio VoiceMeeter VAIO)")];
        let p = |r: &str| cable_pair(r, &ends).map(|c| c.name.as_str());
        assert_eq!(p("Line 2 (Virtual Audio Cable)"), Some("Line 2 (Virtual Audio Cable)"));
        assert_eq!(p("Line 1 (Virtual Audio Cable)"), Some("Line 1 (Virtual Audio Cable)"));
        assert_eq!(p("CABLE Input (VB-Audio Virtual Cable)"), Some("CABLE Output (VB-Audio Virtual Cable)"));
        assert_eq!(p("VoiceMeeter Input (VB-Audio VoiceMeeter VAIO)"), Some("VoiceMeeter Output (VB-Audio VoiceMeeter VAIO)"));
        assert_eq!(p("Speakers (Realtek)"), None);
    }
}
