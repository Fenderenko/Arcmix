//! The Arcmix module in the Windows audio engine (rust/apo, embedded here): installing it on an output
//! device (elevated: `arcmix.exe --apomod install|remove <endpoint GUID>`), its state, and the curves it
//! applies — written to a file in %ProgramData%\Arcmix that the module reads (apo/src/shared.rs).
//!
//! The module works after the mix of any output device (never next to Equalizer APO). A device without
//! effects gets it in both post-mix slots (GFX and MFX: one of them runs); a device with effects of its
//! own keeps them: the module is added to a composite list, or takes the post-mix slot and runs the
//! effect that was there inside itself (wrap.txt). Removing puts everything back as it was.

use std::{
    path::PathBuf,
    sync::{Mutex, atomic::Ordering},
};

use windows::Win32::System::SystemInformation::GetTickCount;

#[path = "../apo/src/shared.rs"]
#[allow(dead_code)]
mod shared;

use crate::{apo, config::Config, logf, util};
use shared::{BANDS, ENTRIES, Entry};

/// The module, built from rust/apo before the app (see build.rs).
const DLL: &[u8] = include_bytes!("../apo/target/release/arcmix_apo.dll");
const CLSID: &str = "{A7C3E2F1-5B4D-4E8A-9C21-3F6D8B0A1E57}";
/// IID_IAudioProcessingObject
const APO_IID: &str = "{FD7F2B29-24D0-4B5C-B177-592C39F9CA10}";
const MMDEV: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render\";
const FX: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d}";
/// The post-mix slots: GFX in the legacy model (a driver that reports no signal processing modes, the
/// engine then uses the LFX / GFX pair), MFX in the modern one, and the composite MFX list.
const GFX: &str = ",2";
const MFX: &str = ",6";
const MFX_LIST: &str = ",14";
/// slots of the modern model (SFX, MFX, EFX, the composite lists); ",1" and ",2" are the legacy one;
/// ",3" is only the "Enhancements" page of the device properties and ",0" the association
const MODERN: [&str; 6] = [",5", ",6", ",7", ",13", ",14", ",15"];
/// single slots the module may sit in (",5" and ",7" — the 1.0.2x and 1.0.32 test builds)
const OURS: [&str; 4] = [GFX, ",5", MFX, ",7"];
/// composite lists the module may be in
const LISTS: [&str; 3] = [",13", MFX_LIST, ",15"];
/// Windows' audio engine loads only signed effects unless this value is 1 (Equalizer APO sets it too)
const AUDIO_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Audio";
const UNPROTECTED: &str = "DisableProtectedAudioDG";
/// what that value was before the installer set it (2 — absent); kept only while it is ours to restore
const OUR_KEY: &str = r"SOFTWARE\Arcmix";
const UNPROTECTED_WAS: &str = "ProtectedAudioDGWas";

fn guid_of(id: &str) -> Option<&str> {
    id.rsplit_once('.').map(|(_, g)| g).filter(|g| g.starts_with('{'))
}

fn fx_key(guid: &str) -> String {
    format!(r"{MMDEV}{guid}\FxProperties")
}

/// The single slots holding the module on this device.
fn our_slots(k: &util::RegKey) -> Vec<&'static str> {
    OURS.into_iter().filter(|s| k.string(&format!("{FX}{s}")).is_some_and(|v| v.trim().eq_ignore_ascii_case(CLSID))).collect()
}

/// A composite list (REG_MULTI_SZ) of CLSIDs.
fn read_list(k: &util::RegKey, slot: &str) -> Vec<String> {
    let Some(b) = k.binary(&format!("{FX}{slot}"), 4096) else { return Vec::new() };
    let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&w).split('\0').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()
}

/// The module sits on this device: in a slot or in a composite list.
fn has_module(k: &util::RegKey) -> bool {
    !our_slots(k).is_empty() || LISTS.iter().any(|l| read_list(k, l).iter().any(|c| c.eq_ignore_ascii_case(CLSID)))
}

/// How the module joins a device's effects.
#[derive(Debug, PartialEq)]
enum Place {
    /// slots of their own: nothing was there
    Fresh(Vec<&'static str>),
    /// the slot's effect runs inside the module; it goes back on removal
    Wrap(&'static str, String),
    /// added to the end of a composite list
    Append(&'static str),
}

fn plan(k: Option<&util::RegKey>) -> Place {
    plan_by(|s| k.and_then(|k| k.binary(&format!("{FX}{s}"), 4096)).map(|b| {
        let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&w).split('\0').next().unwrap_or_default().trim().to_string()
    }))
}

/// The placement from the device's slots (`slot` — the first CLSID of a slot, if it has a value).
fn plan_by(slot: impl Fn(&str) -> Option<String>) -> Place {
    let has = |s: &str| slot(s).is_some();
    let slot = |s: &str| slot(s).filter(|v| !v.is_empty());
    let modern = MODERN.iter().any(|s| has(s));
    if !modern && !has(",1") && !has(GFX) {
        return Place::Fresh(vec![GFX, MFX]);
    }
    if modern {
        if has(MFX_LIST) {
            return Place::Append(MFX_LIST);
        }
        return match slot(MFX) {
            Some(c) => Place::Wrap(MFX, c),
            None => Place::Fresh(vec![MFX]),
        };
    }
    match slot(GFX) {
        Some(c) => Place::Wrap(GFX, c),
        None => Place::Fresh(vec![GFX]),
    }
}

/// The module sits in an effect slot of this device.
pub fn installed_on(id: &str) -> bool {
    guid_of(id).and_then(|g| util::reg_open_hklm64(&fx_key(g))).is_some_and(|k| has_module(&k))
}

/// The module can go on this device: on any, unless Equalizer APO is there.
pub fn free_on(id: &str) -> bool {
    guid_of(id).is_some() && !apo::active_on(id)
}

// ---------- the files the module reads and writes ----------

/// %ProgramData%\Arcmix (made and opened to the audio engine by the installer).
fn data_dir() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join(shared::DIR))
}

/// The module processed sound of this device within the last few seconds (it writes live.bin every
/// 2 s while it does).
pub fn live_on(id: &str) -> bool {
    let Some(g) = guid_of(id) else { return false };
    let key = shared::key_of(&shared::guid_field(g));
    let Some(b) = data_dir().and_then(|d| std::fs::read(d.join(shared::LIVE)).ok()) else { return false };
    let now = unsafe { GetTickCount() };
    shared::decode_live(&b).iter().any(|(k, t)| *k == key && now.wrapping_sub(*t) < 5000)
}

/// Writes curves.bin when it changed: a temporary file renamed over it, so the module never reads half.
fn write_curves(entries: &[Entry]) {
    static LAST: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    let b = shared::encode_curves(entries);
    let Ok(mut last) = LAST.lock() else { return };
    if *last == b {
        return;
    }
    let Some(dir) = data_dir().filter(|d| d.is_dir()) else { return };
    let tmp = dir.join("curves.tmp");
    // a failure is logged once until writing works again
    static FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    match std::fs::write(&tmp, &b).and_then(|_| std::fs::rename(&tmp, dir.join(shared::CURVES))) {
        Ok(()) => {
            *last = b;
            FAILED.store(false, Ordering::Relaxed);
        }
        Err(e) => {
            if !FAILED.swap(true, Ordering::Relaxed) {
                logf!("apomod: curves not written: {e}");
            }
            // curves.bin made read-only by an earlier install: its access is mended (administrator
            // permission, once per run), then the curve goes again
            static MENDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if e.kind() == std::io::ErrorKind::PermissionDenied && !MENDED.swap(true, Ordering::Relaxed) {
                if let Ok(exe) = std::env::current_exe() {
                    std::thread::spawn(move || {
                        let r = apo::run_elevated(&exe.to_string_lossy(), "--apomod access -");
                        logf!("apomod: access to curves.bin mended: {r:?}");
                        sync(&crate::config::get());
                    });
                }
            }
        }
    }
}

/// For the window, about the current output device.
#[derive(serde::Serialize, Clone, Debug, Default, PartialEq)]
pub struct ModStatus {
    /// installed on this device
    pub inst: bool,
    /// it processes its sound now
    pub live: bool,
    /// a slot is free for it: the module can be installed
    pub free: bool,
    /// Windows' "Audio enhancements" are off for this device: no effect runs there
    pub sysfx: bool,
    /// a headset with its own equalizer: the curve of everything is added there only if asked (apoHs)
    pub hs: bool,
    /// "install" | "remove" while that runs ("" otherwise): the audio services restart meanwhile and
    /// the output device is gone for a few seconds, so the window keeps the module row
    #[serde(skip_serializing_if = "String::is_empty")]
    pub busy: String,
    /// the device it runs on
    #[serde(rename = "busyName", skip_serializing_if = "String::is_empty")]
    pub busy_name: String,
    /// the physical output devices present that can have it, and those that have it
    #[serde(rename = "allDevs")]
    pub all_devs: u32,
    #[serde(rename = "onDevs")]
    pub on_devs: u32,
}

/// The install or removal in progress: the operation and the device name.
static BUSY: Mutex<Option<(&'static str, String)>> = Mutex::new(None);

pub fn status(out_id: &str, out_name: &str) -> ModStatus {
    let (busy, busy_name) = BUSY.lock().ok().and_then(|b| b.clone()).map(|(op, n)| (op.to_string(), n)).unwrap_or_default();
    let (mut all_devs, mut on_devs) = (0, 0);
    for d in crate::audio::outs_state().iter().filter(|d| !d.is_virtual && (d.state == "on" || d.state == "hsoff")) {
        if installed_on(&d.id) {
            on_devs += 1;
            all_devs += 1;
        } else if free_on(&d.id) {
            all_devs += 1;
        }
    }
    ModStatus { inst: installed_on(out_id), live: live_on(out_id), free: free_on(out_id), sysfx: apo::sysfx_off(out_id), hs: apo::is_hw_headset(out_name), busy, busy_name, all_devs, on_devs }
}

/// The module applies the "everything" curve on this device.
pub fn applies_on(c: &Config, id: &str, name: &str) -> bool {
    installed_on(id) && !apo::skips(c, name)
}

/// Writes the curves of the devices that have the module (on a thread: the device list comes from the
/// audio thread).
pub fn sync(c: &Config) {
    let c = c.clone();
    std::thread::spawn(move || {
        let devs: Vec<(String, String)> = crate::audio::list_devices().0.into_iter().filter(|d| installed_on(&d.id)).map(|d| (d.id, d.name)).collect();
        if devs.is_empty() {
            return;
        }
        let user = c.soft_eq.get("all").filter(|s| s.bands.len() == 10 && s.preset != "flat" && s.bands.iter().any(|b| *b != 0.0));
        let freqs = [32.0f32, 64.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
        let mut entries = Vec::new();
        for (id, name) in devs.iter().take(ENTRIES) {
            let Some(g) = guid_of(id) else { continue };
            let mut e = Entry::EMPTY;
            e.guid = shared::guid_field(g);
            if let (Some(se), false) = (user, apo::skips(&c, name)) {
                e.on = 1;
                e.n = 10.min(BANDS as u32);
                for i in 0..10 {
                    e.freq[i] = freqs[i];
                    e.gain_db[i] = se.bands[i] as f32;
                    e.q[i] = 1.41;
                }
                // the peaks stay below 0 dBFS
                e.preamp_db = -(se.bands.iter().cloned().fold(0.0f64, f64::max) as f32);
            }
            entries.push(e);
        }
        write_curves(&entries);
    });
}

// ---------- installing (the elevated copy) ----------

fn dll_path() -> Result<PathBuf, String> {
    let dir = PathBuf::from(std::env::var("ProgramFiles").map_err(|e| e.to_string())?).join("Arcmix");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let main = dir.join("ArcmixApo.dll");
    if std::fs::read(&main).is_ok_and(|b| b == DLL) {
        return Ok(main);
    }
    match std::fs::write(&main, DLL) {
        Ok(()) => Ok(main),
        // loaded by the audio engine: a new build goes next to it under its own name
        Err(_) => {
            let h = DLL.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3));
            let alt = dir.join(format!("ArcmixApo-{h:016x}.dll"));
            std::fs::write(&alt, DLL).map_err(|e| e.to_string())?;
            Ok(alt)
        }
    }
}

fn register(path: &std::path::Path) -> Result<(), String> {
    let p = path.to_string_lossy();
    let k = util::reg_create_hklm64(&format!(r"SOFTWARE\Classes\CLSID\{CLSID}")).ok_or("CLSID key")?;
    k.set_string("", "Arcmix APO");
    let k = util::reg_create_hklm64(&format!(r"SOFTWARE\Classes\CLSID\{CLSID}\InprocServer32")).ok_or("InprocServer32 key")?;
    if !(k.set_string("", &p) && k.set_string("ThreadingModel", "Both")) {
        return Err("InprocServer32 values".into());
    }
    let k = util::reg_create_hklm64(&format!(r"SOFTWARE\Classes\AudioEngine\AudioProcessingObjects\{CLSID}")).ok_or("APO key")?;
    let ok = k.set_string("FriendlyName", "Arcmix")
        && k.set_string("Copyright", "Fenderenko, GPL-3.0")
        && k.set_dword("MajorVersion", 1)
        && k.set_dword("MinorVersion", 0)
        && k.set_dword("Flags", 12) // as FLAGS in the module: not in place
        && k.set_dword("MinInputConnections", 1)
        && k.set_dword("MaxInputConnections", 1)
        && k.set_dword("MinOutputConnections", 1)
        && k.set_dword("MaxOutputConnections", 1)
        && k.set_dword("MaxInstances", u32::MAX)
        && k.set_dword("NumAPOInterfaces", 1)
        && k.set_string("APOInterface0", APO_IID);
    if ok { Ok(()) } else { Err("APO values".into()) }
}

/// Lets the audio engine load unsigned effects; the previous value is noted when it was not set.
fn allow_unsigned() -> bool {
    let Some(k) = util::reg_create_hklm64(AUDIO_KEY) else { return false };
    let cur = k.dword(UNPROTECTED);
    if cur == Some(1) {
        return true;
    }
    if let Some(o) = util::reg_create_hklm64(OUR_KEY) {
        if o.dword(UNPROTECTED_WAS).is_none() {
            o.set_dword(UNPROTECTED_WAS, cur.unwrap_or(2));
        }
    }
    k.set_dword(UNPROTECTED, 1)
}

/// Puts the protection back as it was before the installer, once no device has the module and
/// Equalizer APO is on none (it needs the same value).
fn restore_protection() {
    let Some(o) = util::reg_create_hklm64(OUR_KEY) else { return };
    let Some(was) = o.dword(UNPROTECTED_WAS) else { return };
    let guids = util::reg_open_hklm64(MMDEV.trim_end_matches('\\')).map(|k| k.subkeys()).unwrap_or_default();
    let used = guids.iter().any(|g| util::reg_open_hklm64(&fx_key(g)).is_some_and(|k| has_module(&k)) || apo::active_on(&format!("x.{g}")));
    if used {
        return;
    }
    if let Some(k) = util::reg_create_hklm64(AUDIO_KEY) {
        if was == 2 {
            k.delete(UNPROTECTED);
        } else {
            k.set_dword(UNPROTECTED, was);
        }
    }
    o.delete(UNPROTECTED_WAS);
}

/// %ProgramData%\Arcmix, open to the audio engine: LocalService, write-restricted tokens and its sandbox
/// (all application packages, all restricted ones); users read it and create their own files there (the
/// app's curves.bin), as ProgramData allows.
fn open_data_dir() {
    let Some(dir) = data_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let grants = ["*S-1-5-19:(OI)(CI)M", "*S-1-5-33:(OI)(CI)M", "*S-1-15-2-1:(OI)(CI)M", "*S-1-15-2-2:(OI)(CI)M", "*S-1-5-32-545:(OI)(CI)RX"];
    let _ = std::process::Command::new("icacls").arg(&dir).arg("/grant").args(grants).creation_flags_no_window().status();
    // the sandbox runs at low integrity: without a low label it may read the folder but not write there
    let _ = std::process::Command::new("icacls").arg(&dir).args(["/setintegritylevel", "(OI)(CI)L"]).creation_flags_no_window().status();
    // curves.bin made here (by this elevated copy) would be read-only for the app, which replaces it with
    // every curve: users may change this one file
    let curves = dir.join(shared::CURVES);
    if !curves.exists() {
        let _ = std::fs::write(&curves, []);
    }
    let _ = std::process::Command::new("icacls").arg(&curves).args(["/grant", "*S-1-5-32-545:M"]).creation_flags_no_window().status();
}

/// PKEY_FX_Association: the kind of pin the effects belong to; KSNODETYPE_ANY. A device whose driver
/// brings no effects of its own (a pro-audio interface, a virtual cable) has none, and without it the
/// engine builds no effect chain there at all.
const FX_ASSOC: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},0";
const KSNODETYPE_ANY: &str = "{00000000-0000-0000-0000-000000000000}";
/// PKEY_MFX_ProcessingModes_Supported_For_Streaming: the mode effect runs in the default mode
const MFX_MODES: &str = "{d3993a3f-99c2-4402-b5ec-a92a0367664b},6";
const MODE_DEFAULT: &str = "{C18E2F7E-933D-4965-B7D1-1EEF228D2AF3}";

/// What the installer did on a device, in HKLM\SOFTWARE\Arcmix, so that removing undoes it: values it
/// added ("Added {guid}": names split by '|'), the slot it wrapped ("Wrapped {guid}": slot|CLSID), the
/// composite list it joined ("Appended {guid}": slot).
fn note_name(what: &str, guid: &str) -> String {
    format!("{what} {guid}")
}

/// Writes what the engine needs to run the module: the association on a device without effects of its
/// own, the streaming mode of a mode slot it adds; returns what it added.
fn add_chain_values(k: &util::RegKey, slots: &[&str], bare: bool) -> Vec<&'static str> {
    let mut added = Vec::new();
    // a device with effects of its own has its association from the driver
    if bare && !k.has(FX_ASSOC) && k.set_string(FX_ASSOC, KSNODETYPE_ANY) {
        added.push(FX_ASSOC);
    }
    if slots.contains(&MFX) && !k.has(MFX_MODES) && k.set_multi_string(MFX_MODES, &[MODE_DEFAULT]) {
        added.push(MFX_MODES);
    }
    added
}

/// wrap.txt for the module: every wrapped device and its effect. The installer always writes it, so the
/// file belongs to the administrators and users cannot put another effect there.
fn write_wrap_file() {
    let Some(dir) = data_dir() else { return };
    let o = util::reg_open_hklm64(OUR_KEY);
    let guids = util::reg_open_hklm64(MMDEV.trim_end_matches('\\')).map(|k| k.subkeys()).unwrap_or_default();
    let mut text = String::new();
    for g in guids {
        if let Some((_, clsid)) = o.as_ref().and_then(|o| o.string(&note_name("Wrapped", &g))).as_deref().and_then(|v| v.split_once('|')) {
            text.push_str(&format!("{} {clsid}\r\n", g.to_uppercase()));
        }
    }
    let _ = std::fs::write(dir.join(shared::WRAP), text);
}

/// Takes the module off a device and puts back what it changed (no restart).
fn take_off(guid: &str) {
    let Some(k) = util::reg_create_hklm64(&fx_key(guid)) else { return };
    let o = util::reg_create_hklm64(OUR_KEY);
    let note = |what: &str| o.as_ref().and_then(|o| o.string(&note_name(what, guid)));
    let forget = |what: &str| {
        if let Some(o) = &o {
            o.delete(&note_name(what, guid));
        }
    };
    // the wrapped effect goes back into its slot
    if let Some((slot, clsid)) = note("Wrapped").as_deref().and_then(|v| v.split_once('|')) {
        let name = format!("{FX}{slot}");
        if k.string(&name).is_some_and(|v| v.trim().eq_ignore_ascii_case(CLSID)) {
            k.set_string(&name, clsid);
        }
    }
    forget("Wrapped");
    for s in OURS {
        let name = format!("{FX}{s}");
        if k.string(&name).is_some_and(|v| v.trim().eq_ignore_ascii_case(CLSID)) {
            k.delete(&name);
        }
    }
    for l in LISTS {
        let list = read_list(&k, l);
        if list.iter().any(|c| c.eq_ignore_ascii_case(CLSID)) {
            let rest: Vec<&str> = list.iter().map(String::as_str).filter(|c| !c.eq_ignore_ascii_case(CLSID)).collect();
            if rest.is_empty() {
                k.delete(&format!("{FX}{l}"));
            } else {
                k.set_multi_string(&format!("{FX}{l}"), &rest);
            }
        }
    }
    forget("Appended");
    if let Some(list) = note("Added") {
        for name in list.split('|').filter(|x| !x.is_empty()) {
            k.delete(name);
        }
    }
    forget("Added");
}

/// (after installing) What the engine did: whether audiodg has the module loaded and the device's
/// effect values, into %ProgramData%\Arcmix\diag.txt for the app's log. Read-only.
fn write_diag(guid: &str, place: &str) {
    let Some(dir) = data_dir() else { return };
    std::thread::sleep(std::time::Duration::from_secs(6)); // a stream starts again after the restart
    let run = |cmd: &str, args: &[&str]| std::process::Command::new(cmd).args(args).creation_flags_no_window().output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let modules = run("tasklist", &["/m", "/fi", "imagename eq audiodg.exe", "/fo", "list"]);
    let ours: Vec<&str> = modules.lines().filter(|l| l.to_ascii_lowercase().contains("arcmix")).collect();
    let fx = run("reg", &["query", &format!(r"HKLM\{MMDEV}{guid}\FxProperties")]);
    let text = format!("audiodg modules with \"arcmix\": {}\r\nplacement: {place}\r\n\r\n{modules}\r\n{fx}", if ours.is_empty() { "none".to_string() } else { ours.join(", ") });
    let _ = std::fs::write(dir.join("diag.txt"), text);
}

/// PKEY_AudioEndpoint_Disable_SysFx: Windows sets it after 10 failed effect loads (the earlier builds of
/// the module failed in the protected engine); installing means effects are wanted there again.
const SYSFX_OFF: &str = "{1da5d803-d492-4edd-8c23-e0c0ffee7f0e},5";

fn enable_effects(guid: &str) {
    for sub in ["FxProperties", "Properties"] {
        if let Some(k) = util::reg_create_hklm64(&format!(r"{MMDEV}{guid}\{sub}")) {
            if k.dword(SYSFX_OFF) == Some(1) {
                k.delete(SYSFX_OFF);
            }
        }
    }
}

/// Deletes the copies of the module left by earlier builds (a new build goes under its own name while the
/// engine holds the old file); the registered one stays, one still loaded fails and waits for next time.
fn clean_old_dlls() {
    let Some(cur) = util::reg_open_hklm64(&format!(r"SOFTWARE\Classes\CLSID\{CLSID}\InprocServer32")).and_then(|k| k.string("")).map(PathBuf::from) else { return };
    let Some(dir) = cur.parent() else { return };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("arcmixapo") && name.ends_with(".dll") && !e.path().as_os_str().eq_ignore_ascii_case(cur.as_os_str()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Restarts the Windows audio services so the device picks the change up (sound stops for a second).
/// The endpoint builder caches the effect properties of devices: it restarts as well.
fn restart_audio() {
    let steps: [&[&str]; 4] = [&["stop", "audiosrv", "/y"], &["stop", "AudioEndpointBuilder", "/y"], &["start", "AudioEndpointBuilder"], &["start", "audiosrv"]];
    for args in steps {
        let _ = std::process::Command::new("net").args(args).creation_flags_no_window().status();
    }
}

trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

impl NoWindow for std::process::Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
    }
}

/// (the elevated copy) Exit codes: 0 — done, 2 — bad request, 3 — files, 4 — registration,
/// 5 — the device's effect keys, 6 — unsigned effects could not be allowed, 10 — Equalizer APO is on the
/// device (not touched). `guids` — one or several devices, comma-separated (one prompt, one restart of
/// the audio services); with several, the first failure is returned and the rest still go on.
pub fn elevated(op: &str, guids: &str) -> i32 {
    // the data folder only: curves.bin left read-only by an earlier install (before 1.3.0)
    if op == "access" {
        open_data_dir();
        return 0;
    }
    let gs: Vec<&str> = guids.split(',').collect();
    if !gs.iter().all(|g| g.starts_with('{') && g.ends_with('}') && g.len() == 38) {
        return 2;
    }
    match op {
        "install" => {
            let path = match dll_path() {
                Ok(p) => p,
                Err(_) => return 3,
            };
            if register(&path).is_err() {
                return 4;
            }
            open_data_dir();
            if !allow_unsigned() {
                return 6;
            }
            let mut code = 0;
            let mut done = Vec::new();
            for g in gs {
                match install_on(g) {
                    Ok(place) => done.push((g, place)),
                    Err(c) if code == 0 => code = c,
                    Err(_) => {}
                }
            }
            if !done.is_empty() {
                // the module logs errors only: what the test builds wrote before goes
                if let Some(d) = data_dir() {
                    let _ = std::fs::remove_file(d.join("apo.log"));
                }
                restart_audio();
                clean_old_dlls();
                for (g, place) in done {
                    write_diag(g, &place);
                }
            }
            code
        }
        "remove" => {
            for g in gs {
                take_off(g);
            }
            write_wrap_file();
            restore_protection();
            restart_audio();
            clean_old_dlls();
            0
        }
        _ => 2,
    }
}

/// (the elevated copy) Puts the module into one device's effect keys; the placement for the diagnostics.
fn install_on(guid: &str) -> Result<String, i32> {
    if apo::active_on(&format!("x.{guid}")) {
        return Err(10);
    }
    // an earlier placement (another build, another model) is undone first
    take_off(guid);
    let Some(k) = util::reg_create_hklm64(&fx_key(guid)) else { return Err(5) };
    let place = plan(Some(&k));
    let o = util::reg_create_hklm64(OUR_KEY);
    let note = |what: &str, v: &str| {
        if let Some(o) = &o {
            o.set_string(&note_name(what, guid), v);
        }
    };
    let ok = match &place {
        Place::Fresh(slots) => {
            let ok = slots.iter().all(|s| k.set_string(&format!("{FX}{s}"), CLSID));
            let added = add_chain_values(&k, slots, slots.len() == 2);
            if !added.is_empty() {
                note("Added", &added.join("|"));
            }
            ok
        }
        Place::Wrap(slot, clsid) => {
            note("Wrapped", &format!("{slot}|{clsid}"));
            k.set_string(&format!("{FX}{slot}"), CLSID)
        }
        Place::Append(slot) => {
            let mut list = read_list(&k, slot);
            list.push(CLSID.to_string());
            note("Appended", slot);
            k.set_multi_string(&format!("{FX}{slot}"), &list.iter().map(String::as_str).collect::<Vec<_>>())
        }
    };
    write_wrap_file();
    if !ok {
        take_off(guid);
        write_wrap_file();
        return Err(5);
    }
    enable_effects(guid);
    Ok(format!("{place:?}"))
}

/// The output devices the module can still go on: physical, present, without it and without Equalizer APO.
fn missing_on(outs: &[(String, String)]) -> Vec<(String, String)> {
    outs.iter().filter(|(id, name)| !crate::audio::outprio::is_virtual_name(name) && guid_of(id).is_some() && !installed_on(id) && free_on(id)).cloned().collect()
}

/// (audio thread, with every device list) EQ Windows works everywhere by itself: the module goes on the
/// output devices that do not have it yet, all of them with one administrator prompt (config apoAuto,
/// chosen in the introduction; not before it is passed). A device asked about once is not asked again —
/// "Install" in the window stays for it.
pub fn auto_install(outs: &[(String, String)]) {
    let c = crate::config::get();
    if !c.apo_auto || !c.onboarded || BUSY.lock().is_ok_and(|b| b.is_some()) {
        return;
    }
    let want: Vec<(String, String)> = missing_on(outs).into_iter().filter(|(id, _)| !c.apo_asked.contains(id)).collect();
    if want.is_empty() {
        return;
    }
    let ids: Vec<String> = want.iter().map(|(id, _)| id.clone()).collect();
    crate::config::set(|c| c.apo_asked.extend(ids));
    logf!("apomod: installing on {} device(s) by itself", want.len());
    std::thread::spawn(move || run_on("install", want));
}

/// (from the introduction) EQ Windows on: the module on every output device now, with one prompt (also
/// those asked about before), and on new ones by itself later.
pub fn install_all() {
    crate::config::set(|c| c.apo_auto = true);
    let outs: Vec<(String, String)> = crate::audio::list_devices().0.into_iter().map(|d| (d.id, d.name)).collect();
    let want = missing_on(&outs);
    if !want.is_empty() {
        run_on("install", want);
    }
}

/// (background thread) Installs or removes the module on the current output device, elevated; the window
/// follows through the state.
pub fn run(op: &'static str, id: String) {
    if op == "remove" {
        // taken off by hand: not put back by itself (auto_install)
        let k = id.clone();
        crate::config::set(|c| {
            if !c.apo_asked.contains(&k) {
                c.apo_asked.push(k);
            }
        });
    }
    let o = crate::audio::out_state();
    let name = if o.id == id { if o.short.is_empty() { o.name } else { o.short } } else { String::new() };
    run_on(op, vec![(id, name)]);
}

/// (background thread) The same on several devices with one prompt.
fn run_on(op: &'static str, devs: Vec<(String, String)>) {
    let gs: Vec<String> = devs.iter().filter_map(|(id, _)| guid_of(id).map(String::from)).collect();
    if gs.is_empty() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    {
        let Ok(mut b) = BUSY.lock() else { return };
        if b.is_some() {
            return; // one at a time: a second click while it runs does nothing
        }
        let names: Vec<&str> = devs.iter().map(|(_, n)| n.as_str()).filter(|n| !n.is_empty()).collect();
        *b = Some((op, names.join(", ")));
    }
    crate::server::notify_subs();
    let r = apo::run_elevated(&exe.to_string_lossy(), &format!("--apomod {op} {}", gs.join(",")));
    logf!("apomod: {op} on {}: {r:?}", gs.join(","));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    sync(&crate::config::get());
    if let Ok(mut b) = BUSY.lock() {
        *b = None;
    }
    crate::server::notify_subs();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(slots: &[(&str, &str)]) -> Place {
        plan_by(|s| slots.iter().find(|(k, _)| *k == s).map(|(_, v)| v.to_string()))
    }

    #[test]
    fn placement_per_device() {
        // Focusrite: no effects at all
        assert_eq!(place(&[]), Place::Fresh(vec![GFX, MFX]));
        // LinkBuds, Arctis 7+: Windows' stream and mode effects
        let wm = [(",3", "{5860E1C5-F95C-4a7a-8EC8-8AEF24F379A1}"), (",5", "{C9453E73-8C5C-4463-9984-AF8BAB2F5447}"), (",6", "{13AB3EBD-137E-4903-9D89-60BE8277FD17}")];
        assert_eq!(place(&wm), Place::Wrap(MFX, "{13AB3EBD-137E-4903-9D89-60BE8277FD17}".into()));
        // a driver with a composite mode list
        assert_eq!(place(&[(",13", "{A}"), (",14", "{B}")]), Place::Append(MFX_LIST));
        // a stream effect only
        assert_eq!(place(&[(",5", "{A}")]), Place::Fresh(vec![MFX]));
        // the legacy pair
        assert_eq!(place(&[(",1", "{A}"), (",2", "{B}")]), Place::Wrap(GFX, "{B}".into()));
        assert_eq!(place(&[(",1", "{A}")]), Place::Fresh(vec![GFX]));
        // the Enhancements page alone is no effect
        assert_eq!(place(&[(",3", "{5860E1C5-F95C-4a7a-8EC8-8AEF24F379A1}")]), Place::Fresh(vec![GFX, MFX]));
    }
}
