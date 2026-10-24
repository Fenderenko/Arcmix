//! Equalizer APO (installed by the user): the equalizer of everything for any output device, without
//! cables and without delay — it runs inside the Windows audio engine. Arcmix keeps its curves in
//! `arcmix.txt` in Equalizer APO's config folder, included from `config.txt`; the one-time setup
//! (adding the include, giving the user write access to that one file) runs elevated
//! (`arcmix.exe --apo-setup <user>`). Which devices Equalizer APO works on is chosen in its own
//! Device Selector, opened from the window (also elevated): the driver's effect chain is not touched
//! by Arcmix itself.
//!
//! AutoEQ (jaakkopasanen/AutoEq, MIT): a correction curve per headphone model, downloaded on
//! request — the index once (~850 KB), then the GraphicEQ file of the chosen model.

use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, SystemTime},
};

use serde::Serialize;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
        UI::{
            Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::{HSTRING, PCWSTR, w},
};

use crate::{config::Config, logf, util};

const FILE: &str = "arcmix.txt";
const INCLUDE: &str = "Include: arcmix.txt";
/// The project page (listed in server::OPEN_URLS).
pub const DOWNLOAD_PAGE: &str = "https://sourceforge.net/projects/equalizerapo/";
const MMDEV: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render\";
/// FxProperties values that name the effect objects of an endpoint: LFX, GFX, (UI), SFX, MFX, EFX and
/// the composite lists of Windows 8.1+
const FX_SLOTS: [&str; 9] = [",1", ",2", ",3", ",5", ",6", ",7", ",13", ",14", ",15"];
const FX_KEY: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d}";
/// PKEY_AudioEndpoint_Disable_SysFx: Windows' "Audio enhancements" switched off for the device
const SYSFX_OFF: &str = "{1da5d803-d492-4edd-8c23-e0c0ffee7f0e},5";

// ---------- where Equalizer APO is ----------

fn install_dir() -> Option<PathBuf> {
    let k = util::reg_open_hklm64(r"SOFTWARE\EqualizerAPO")?;
    k.string("InstallPath").map(PathBuf::from).filter(|p| p.is_dir())
}

fn config_dir() -> Option<PathBuf> {
    let k = util::reg_open_hklm64(r"SOFTWARE\EqualizerAPO")?;
    k.string("ConfigPath").map(PathBuf::from).or_else(|| install_dir().map(|d| d.join("config"))).filter(|p| p.is_dir())
}

fn selector() -> Option<PathBuf> {
    let d = install_dir()?;
    ["DeviceSelector.exe", "Configurator.exe"].into_iter().map(|f| d.join(f)).find(|p| p.is_file())
}

/// config.txt includes our file and we may write it (the setup has been done).
fn ready() -> bool {
    let Some(dir) = config_dir() else { return false };
    let inc = std::fs::read_to_string(dir.join("config.txt")).is_ok_and(|s| s.lines().any(|l| l.trim().eq_ignore_ascii_case(INCLUDE)));
    inc && std::fs::OpenOptions::new().append(true).open(dir.join(FILE)).is_ok()
}

/// The endpoint GUID of an id "{0.0.0.00000000}.{guid}".
fn guid_of(id: &str) -> Option<&str> {
    id.rsplit_once('.').map(|(_, g)| g).filter(|g| g.starts_with('{'))
}

fn is_apo_clsid(clsid: &str) -> bool {
    let clsid = clsid.trim();
    if !clsid.starts_with('{') {
        return false;
    }
    util::reg_open_hklm64(&format!(r"SOFTWARE\Classes\CLSID\{clsid}\InprocServer32")).and_then(|k| k.string("")).is_some_and(|p| p.to_ascii_lowercase().contains("equalizerapo"))
}

/// Equalizer APO is in the effect chain of this output device.
pub fn active_on(id: &str) -> bool {
    let Some(g) = guid_of(id) else { return false };
    let Some(k) = util::reg_open_hklm64(&format!(r"{MMDEV}{g}\FxProperties")) else { return false };
    FX_SLOTS.iter().any(|s| {
        // REG_SZ or REG_MULTI_SZ: UTF-16 strings split by NUL
        k.binary(&format!("{FX_KEY}{s}"), 4096).is_some_and(|b| {
            let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&w).split('\0').any(is_apo_clsid)
        })
    })
}

pub(crate) fn sysfx_off(id: &str) -> bool {
    let Some(g) = guid_of(id) else { return false };
    ["FxProperties", "Properties"].iter().any(|sub| util::reg_open_hklm64(&format!(r"{MMDEV}{g}\{sub}")).and_then(|k| k.dword(SYSFX_OFF)).is_some_and(|v| v == 1))
}

/// The name of a connected headset whose own equalizer Arcmix drives (lowercase), if any.
fn hw_headset() -> Option<String> {
    let d = crate::devices::state();
    (d.native && d.eq.is_some() && !d.name.is_empty()).then(|| d.name.to_lowercase())
}

pub(crate) fn is_hw_headset(name: &str) -> bool {
    hw_headset().is_some_and(|h| name.to_lowercase().contains(&h))
}

/// Equalizer APO's curves are left out on this device: the headset's own equalizer works there and
/// the user has not asked for both.
pub fn skips(c: &Config, name: &str) -> bool {
    !c.apo_hs && is_hw_headset(name)
}

/// The curves of Arcmix are applied by Equalizer APO on this output device.
pub fn applies_on(c: &Config, id: &str, name: &str) -> bool {
    active_on(id) && !skips(c, name)
}

/// For the window, about the current output device.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct ApoStatus {
    pub installed: bool,
    /// the one-time setup is done
    pub ready: bool,
    /// Equalizer APO works on this device
    pub dev: bool,
    /// Windows' audio enhancements are off for it (Equalizer APO is then skipped)
    #[serde(rename = "sysfxOff")]
    pub sysfx_off: bool,
    /// this device is a headset with its own equalizer (then Equalizer APO works on it only if asked)
    pub hs: bool,
    /// the sample settings of Equalizer APO (−6 dB, a bass boost) still work
    pub samples: bool,
    /// the AutoEQ correction of this device: model and measurement source
    #[serde(rename = "autoEq", skip_serializing_if = "Option::is_none")]
    pub auto_eq: Option<(String, String)>,
}

pub fn status(c: &Config, out_id: &str, out_name: &str) -> ApoStatus {
    let installed = config_dir().is_some();
    if !installed {
        return ApoStatus::default();
    }
    ApoStatus {
        installed,
        ready: ready(),
        dev: active_on(out_id),
        sysfx_off: sysfx_off(out_id),
        hs: is_hw_headset(out_name),
        samples: samples_on(),
        auto_eq: c.auto_eq.get(out_id).map(|a| (a.name.clone(), a.src.clone())),
    }
}

// ---------- elevated steps ----------

/// Runs a program elevated (UAC prompt) and waits for it; Some(exit code), None if refused.
pub fn run_elevated(file: &str, params: &str) -> Option<u32> {
    let (f, p) = (HSTRING::from(file), HSTRING::from(params));
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(f.as_ptr()),
        lpParameters: PCWSTR(p.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut sei) }.ok()?;
    let h: HANDLE = sei.hProcess;
    if h.is_invalid() {
        return Some(0);
    }
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(h, INFINITE);
        let _ = GetExitCodeProcess(h, &mut code);
        let _ = CloseHandle(h);
    }
    Some(code)
}

/// (background thread) The one-time setup, elevated; the window follows through the state.
pub fn setup() {
    let Ok(exe) = std::env::current_exe() else { return };
    let user = format!("{}\\{}", std::env::var("USERDOMAIN").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default());
    let r = run_elevated(&exe.to_string_lossy(), &format!("--apo-setup \"{user}\""));
    logf!("apo: setup {}", match r {
        Some(0) => "done".to_string(),
        Some(c) => format!("failed ({c})"),
        None => "cancelled".to_string(),
    });
    LAST.lock().unwrap().clear(); // written anew
    sync(&crate::config::get());
}

/// (background thread) Equalizer APO's Device Selector, elevated: the user ticks the devices.
pub fn choose_devices() {
    if let Some(s) = selector() {
        let r = run_elevated(&s.to_string_lossy(), "");
        logf!("apo: device selector closed ({r:?})");
    }
}

fn is_sample(l: &str) -> bool {
    let l = l.trim();
    l.eq_ignore_ascii_case("Preamp: -6 dB") || l.eq_ignore_ascii_case("Include: example.txt")
}

/// A flat graphic equalizer (every gain 0), as Equalizer APO's editor leaves it.
fn is_flat_graphic(l: &str) -> bool {
    l.trim().strip_prefix("GraphicEQ:").is_some_and(|r| r.split(';').all(|p| p.split_whitespace().nth(1).is_none_or(|g| g.parse::<f64>().is_ok_and(|g| g == 0.0))))
}

/// config.txt holds only the samples, a flat curve and our include — nothing of the user's own.
fn samples_only(text: &str) -> bool {
    let body: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    body.iter().any(|l| is_sample(l)) && body.iter().all(|l| is_sample(l) || is_flat_graphic(l) || l.eq_ignore_ascii_case(INCLUDE))
}

/// The samples of Equalizer APO still work (a −6 dB preamp, a bass boost) next to our curve.
fn samples_on() -> bool {
    config_dir().and_then(|d| std::fs::read_to_string(d.join("config.txt")).ok()).is_some_and(|t| samples_only(&t))
}

/// (the elevated copy: `arcmix.exe --apo-setup <user>`) Adds the include to config.txt and gives the
/// user write access to arcmix.txt. Returns the exit code.
pub fn elevated_setup(user: &str) -> i32 {
    let Some(dir) = config_dir() else { return 2 };
    let cfg = dir.join("config.txt");
    let text = std::fs::read_to_string(&cfg).unwrap_or_default();
    // the sample settings of a fresh install (a preamp and example.txt) would change the sound too: they
    // are commented out when nothing else of the user's is there; anything the user wrote stays as it is
    let mut out: String = if samples_only(&text) {
        text.lines().map(|l| if is_sample(l) { format!("# {l}") } else { l.to_string() }).collect::<Vec<_>>().join("\r\n")
    } else {
        text.clone()
    };
    if !text.lines().any(|l| l.trim().eq_ignore_ascii_case(INCLUDE)) {
        out.push_str("\r\n# Arcmix: the equalizer set in Arcmix\r\n");
        out.push_str(INCLUDE);
        out.push_str("\r\n");
    }
    if out != text && std::fs::write(&cfg, out).is_err() {
        return 3;
    }
    let file = dir.join(FILE);
    if !file.exists() && std::fs::write(&file, "# Arcmix\r\n").is_err() {
        return 4;
    }
    let ok = std::process::Command::new("icacls")
        .arg(&file)
        .arg("/grant")
        .arg(format!("{user}:M"))
        .creation_flags_no_window()
        .status()
        .is_ok_and(|s| s.success());
    if ok { 0 } else { 5 }
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

// ---------- the curves ----------

/// The 10 bands of the software equalizer (dsp.rs: peaking filters, Q 1.41).
const FREQS: [f64; 10] = [32.0, 64.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

static LAST: Mutex<String> = Mutex::new(String::new());

/// The text of arcmix.txt: for each device Equalizer APO works on (id, left out), the "everything" curve and
/// its AutoEQ correction. Preamps keep the peaks below 0 dBFS.
fn render(c: &Config, devs: &[(String, bool)]) -> String {
    let mut s = String::from("# Written by Arcmix: changes here are replaced. Your own settings belong in config.txt.\r\n");
    let user = c.soft_eq.get("all").filter(|s| s.bands.len() == 10 && s.preset != "flat" && s.bands.iter().any(|b| *b != 0.0));
    let user_peak = user.map_or(0.0, |se| se.bands.iter().cloned().fold(0.0f64, f64::max));
    for (id, skip) in devs {
        let Some(g) = guid_of(id) else { continue };
        if *skip {
            continue;
        }
        let aeq = c.auto_eq.get(id).filter(|a| a.on && !a.pts.is_empty());
        if user.is_none() && aeq.is_none() {
            continue;
        }
        let aeq_peak = aeq.map_or(0.0, |a| a.pts.iter().map(|p| p[1] as f64).fold(0.0f64, f64::max));
        s.push_str(&format!("Device: {g}\r\nPreamp: {:.1} dB\r\n", -(user_peak + aeq_peak)));
        if let Some(se) = user {
            for (f, gain) in FREQS.iter().zip(&se.bands) {
                if *gain != 0.0 {
                    s.push_str(&format!("Filter: ON PK Fc {f} Hz Gain {gain:.1} dB Q 1.41\r\n"));
                }
            }
        }
        if let Some(a) = aeq {
            s.push_str("GraphicEQ: ");
            s.push_str(&a.pts.iter().map(|p| format!("{} {:.1}", p[0], p[1])).collect::<Vec<_>>().join("; "));
            s.push_str("\r\n");
        }
    }
    s
}

/// Brings arcmix.txt in line with the settings (written only when it changes). The device list comes
/// from the audio thread: done on a thread of its own, never blocking the caller.
pub fn sync(c: &Config) {
    if config_dir().is_none() {
        return;
    }
    let c = c.clone();
    std::thread::spawn(move || {
        let Some(dir) = config_dir() else { return };
        let devs: Vec<(String, bool)> = crate::audio::list_devices().0.into_iter().filter(|d| !d.is_virtual && active_on(&d.id)).map(|d| {
            let skip = skips(&c, &d.name);
            (d.id, skip)
        }).collect();
        let text = render(&c, &devs);
        let mut last = LAST.lock().unwrap();
        if *last == text {
            return;
        }
        if let Err(e) = std::fs::write(dir.join(FILE), &text) {
            logf!("apo: {FILE} not written: {e}");
        }
        *last = text; // not again until something changes
    });
}

// ---------- AutoEQ ----------

const AEQ_HOST: &str = "raw.githubusercontent.com";
const AEQ_ROOT: &str = "/jaakkopasanen/AutoEq/master/results/";

fn index_path() -> PathBuf {
    util::data_dir().join("autoeq").join("INDEX.md")
}

/// The index, downloaded once and refreshed after a month.
fn index() -> Result<String, String> {
    let p = index_path();
    let fresh = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|age| age < Duration::from_secs(30 * 86400));
    if !fresh {
        match crate::update::get_bytes(AEQ_HOST, &format!("{AEQ_ROOT}INDEX.md"), 4 << 20) {
            Ok(b) => {
                let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
                let _ = std::fs::write(&p, &b);
            }
            Err(e) if !p.exists() => return Err(e),
            Err(_) => {}
        }
    }
    std::fs::read_to_string(&p).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct Model {
    /// the headphones
    pub n: String,
    /// who measured them (and on what)
    pub s: String,
    /// the folder in the results, URL-encoded as in the index
    pub p: String,
}

/// Parses "- [Name](./path) by Source on Rig".
fn parse(line: &str) -> Option<Model> {
    let l = line.strip_prefix("- [")?;
    let (n, rest) = l.split_once("](./")?;
    // a model name may hold brackets itself: the path ends at the last ") by "
    let (p, s) = match rest.rsplit_once(") by ") {
        Some((p, s)) => (p, s.trim()),
        None => (rest.trim_end().strip_suffix(')')?, ""),
    };
    Some(Model { n: n.to_string(), s: s.to_string(), p: p.to_string() })
}

/// Models whose name holds every word of the query (at most 40).
pub fn search(q: &str) -> Result<Vec<Model>, String> {
    let idx = index()?;
    let words: Vec<String> = q.to_lowercase().split_whitespace().map(String::from).collect();
    Ok(idx
        .lines()
        .filter_map(parse)
        .filter(|m| {
            let n = m.n.to_lowercase();
            words.iter().all(|w| n.contains(w.as_str()))
        })
        .take(40)
        .collect())
}

/// Downloads the GraphicEQ curve of a model: [frequency, gain] pairs.
pub fn fetch_curve(path: &str) -> Result<Vec<[f32; 2]>, String> {
    // only a folder of the index: no way out of the results
    if path.contains("..") || path.starts_with('/') || path.contains(['?', '#', '\\']) {
        return Err("bad path".into());
    }
    let last = path.rsplit('/').next().unwrap_or(path);
    let b = crate::update::get_bytes(AEQ_HOST, &format!("{AEQ_ROOT}{path}/{last}%20GraphicEQ.txt"), 256 << 10)?;
    let t = String::from_utf8_lossy(&b);
    let line = t.lines().find_map(|l| l.trim().strip_prefix("GraphicEQ:")).ok_or("no GraphicEQ line")?;
    let pts: Vec<[f32; 2]> = line
        .split(';')
        .filter_map(|p| {
            let mut it = p.split_whitespace();
            let f: f32 = it.next()?.parse().ok()?;
            let g: f32 = it.next()?.parse().ok()?;
            (f > 0.0 && f < 30000.0 && g.abs() < 40.0).then_some([f, g])
        })
        .collect();
    if pts.len() < 10 {
        return Err("too few points".into());
    }
    Ok(pts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autoeq_index_lines() {
        let m = parse("- [Sony LinkBuds S (ANC on)](./DHRME/in-ear/Sony%20LinkBuds%20S%20(ANC%20on)) by DHRME").unwrap();
        assert_eq!(m.n, "Sony LinkBuds S (ANC on)");
        assert_eq!(m.p, "DHRME/in-ear/Sony%20LinkBuds%20S%20(ANC%20on)");
        let m = parse("- [SteelSeries Arctis 7+](./Rtings/HMS%20II.3%20over-ear/SteelSeries%20Arctis%207+) by Rtings on HMS II.3").unwrap();
        assert_eq!(m.s, "Rtings on HMS II.3");
        assert!(parse("# Index").is_none());
    }

    /// The real download of a curve: cargo test --release -- --ignored autoeq_live
    #[test]
    #[ignore]
    fn autoeq_live() {
        let pts = fetch_curve("Rtings/HMS%20II.3%20over-ear/SteelSeries%20Arctis%207+").unwrap();
        assert!(pts.len() > 50, "{}", pts.len());
    }

    #[test]
    fn samples_are_found() {
        assert!(samples_only("Preamp: -6 dB\nInclude: example.txt\nGraphicEQ: 25 0; 40 0; 63 0\n# Arcmix\nInclude: arcmix.txt"));
        assert!(!samples_only("Preamp: -6 dB\nInclude: example.txt\nGraphicEQ: 25 3; 40 0"), "the user's own curve");
        assert!(!samples_only("# Preamp: -6 dB\nInclude: arcmix.txt"), "commented already");
    }

    #[test]
    fn curve_file_text() {
        let mut c = Config::default();
        c.soft_eq.insert("all".into(), crate::config::SoftEq { bands: vec![0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, -2.0, 0.0, 0.0], preset: "bass".into(), custom: vec![] });
        c.auto_eq.insert("{0.0.0.00000000}.{abc}".into(), crate::config::AutoEq { name: "X".into(), src: "Y".into(), pts: vec![[20.0, 2.0], [1000.0, -1.5]], on: true });
        let t = render(&c, &[("{0.0.0.00000000}.{abc}".into(), false), ("{0.0.0.00000000}.{def}".into(), false), ("{0.0.0.00000000}.{hs}".into(), true)]);
        assert!(t.contains("Device: {abc}\r\nPreamp: -5.0 dB\r\nFilter: ON PK Fc 64 Hz Gain 3.0 dB Q 1.41\r\nFilter: ON PK Fc 4000 Hz Gain -2.0 dB Q 1.41\r\nGraphicEQ: 20 2.0; 1000 -1.5"));
        assert!(t.contains("Device: {def}\r\nPreamp: -3.0 dB\r\nFilter: ON PK Fc 64 Hz"));
        assert!(!t.contains("{hs}"), "the headset with its own equalizer is left out");
    }
}
