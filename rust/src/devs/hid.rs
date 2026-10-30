//! HID: finding SteelSeries interfaces, the native Arctis 7+ driver and sessions with headsets
//! described in devices.json (all but the Arctis 7+ family, which has its own tested driver).

use std::{
    collections::{HashMap, VecDeque},
    ffi::c_void,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::{
                DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVINFO_DATA, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
                SetupDiGetDeviceInterfaceDetailW,
            },
            HumanInterfaceDevice::{HIDP_CAPS, HidD_FreePreparsedData, HidD_GetFeature, HidD_GetHidGuid, HidD_GetPreparsedData, HidD_SetFeature, HidP_GetCaps, PHIDP_PREPARSED_DATA},
        },
        Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE},
        Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile, WriteFile},
        System::IO::CancelIoEx,
    },
    core::{GUID, HSTRING},
};

use super::engine::{DevConfig, FVal, device_pids, parse_devices, simple_call};
use crate::{config, devices::DevEq, devices::DevSetting, headset, logf, util};

const DEVICES_JSON: &str = include_str!("../../../web/devices.json");

/// The Arctis 7+ family — the tested native driver.
const NATIVE_PIDS: [u16; 4] = [0x220e, 0x2212, 0x2216, 0x2236];

/// Descriptions whose graphic equalizer the window shows: being checked on a real headset (the others
/// stay hidden until then).
const SHOWN_EQ: [&str; 1] = ["arctis_7.yaml"];

#[derive(Clone, Debug, Default)]
pub struct HidIface {
    pub path: String,
    pub mi: i32,
    pub pid: u16,
    pub usage_page: u16,
    pub in_len: u16,
    pub out_len: u16,
    pub feat_len: u16,
    /// the container (the physical receiver): its audio devices in Windows share it
    pub container: Option<[u8; 16]>,
}

/// A handle that closes itself.
struct H(HANDLE);

impl Drop for H {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

unsafe impl Send for H {}

fn open(path: &str, access: u32) -> Option<H> {
    unsafe { CreateFileW(&HSTRING::from(path), access, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }.ok().map(H)
}

fn open_rw(path: &str) -> Option<H> {
    open(path, GENERIC_READ.0 | GENERIC_WRITE.0)
}

/// All HID interfaces of SteelSeries (VID 0x1038) with the interface number and capabilities.
pub fn enum_hid() -> Vec<HidIface> {
    let mut res = Vec::new();
    unsafe {
        let guid: GUID = HidD_GetHidGuid();
        let Ok(set) = SetupDiGetClassDevsW(Some(&guid), None, None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) else { return res };
        for i in 0.. {
            let mut d = SP_DEVICE_INTERFACE_DATA { cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInterfaces(set, None, &guid, i, &mut d).is_err() {
                break;
            }
            let mut need = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(set, &d, None, 0, Some(&mut need), None);
            if need == 0 {
                continue;
            }
            let mut buf = vec![0u8; need as usize + 8];
            *(buf.as_mut_ptr() as *mut u32) = 8; // cbSize of SP_DEVICE_INTERFACE_DETAIL_DATA_W on x64
            let mut di = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiGetDeviceInterfaceDetailW(set, &d, Some(buf.as_mut_ptr() as *mut _), need, None, Some(&mut di)).is_err() {
                continue;
            }
            let w = std::slice::from_raw_parts(buf.as_ptr().add(4) as *const u16, (need as usize - 4) / 2);
            let path = util::from_wide(w);
            let lp = path.to_lowercase();
            if !lp.contains("vid_1038") {
                continue;
            }
            let pid = lp.find("pid_").and_then(|k| lp.get(k + 4..k + 8)).and_then(|h| u16::from_str_radix(h, 16).ok()).unwrap_or(0);
            // a device without several interfaces — interface 0
            let mi = lp.find("&mi_").and_then(|k| lp.get(k + 4..k + 6)).and_then(|h| i32::from_str_radix(h, 16).ok()).unwrap_or(0);
            // access 0 — capabilities only (system collections open this way too)
            let Some(h) = open(&path, 0) else { continue };
            let mut hi = HidIface { path, mi, pid, container: crate::audio::btbatt::container(di.DevInst), ..Default::default() };
            let mut ppd = PHIDP_PREPARSED_DATA::default();
            if HidD_GetPreparsedData(h.0, &mut ppd) {
                let mut caps = HIDP_CAPS::default();
                let _ = HidP_GetCaps(ppd, &mut caps);
                let _ = HidD_FreePreparsedData(ppd);
                (hi.usage_page, hi.in_len, hi.out_len, hi.feat_len) = (caps.UsagePage, caps.InputReportByteLength, caps.OutputReportByteLength, caps.FeatureReportByteLength);
            }
            res.push(hi);
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    res
}

/// The mi_03 and mi_05 interfaces of the Arctis 7+ family: mi_05 needs the collection with
/// 0xff00 input reports, mi_03 — 0xffc0 with output reports.
fn find_arctis(all: &[HidIface]) -> (Option<HidIface>, Option<HidIface>) {
    let (mut m3, mut m5): (Option<HidIface>, Option<HidIface>) = (None, None);
    for hi in all.iter().filter(|h| NATIVE_PIDS.contains(&h.pid)) {
        let slot = match hi.mi {
            3 => &mut m3,
            5 => &mut m5,
            _ => continue,
        };
        if slot.as_ref().is_none_or(|o| o.in_len < hi.in_len) {
            *slot = Some(hi.clone());
        }
    }
    (m3, m5)
}

/// Sends a report (data[0] — the report number, 0) through a handle of its own; a stuck write is
/// given up after 1.5 s.
pub fn write_report(hi: &HidIface, data: &[u8]) -> bool {
    let n = hi.out_len as usize;
    if n < data.len() {
        return false;
    }
    let Some(h) = open_rw(&hi.path) else { return false };
    let mut buf = vec![0u8; n];
    buf[..data.len()].copy_from_slice(data);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut w = 0u32;
        let ok = unsafe { WriteFile(h.0, Some(&buf), Some(&mut w), None) }.is_ok();
        drop(h); // closed here: even if the write hung and nobody waits any more
        let _ = tx.send(ok);
    });
    rx.recv_timeout(Duration::from_millis(1500)).unwrap_or(false)
}

/// The reading handles of a session, so that it can stop them when it gives way to another receiver (a
/// read waits for a report that may never come).
#[derive(Clone, Default)]
struct Readers {
    handles: Arc<Mutex<Vec<isize>>>,
    stop: Arc<AtomicBool>,
}

impl Readers {
    /// Ends the reads of this session; each reader exits.
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        // a reader may be just between two reads: cancelled again until all are gone
        for _ in 0..20 {
            {
                let l = self.handles.lock().unwrap();
                if l.is_empty() {
                    return;
                }
                for h in l.iter() {
                    let _ = unsafe { CancelIoEx(HANDLE(*h as *mut c_void), None) };
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

/// Reads input reports until an error or Readers::stop; false — the collection could not be opened.
fn read_reports(hi: &HidIface, readers: &Readers, mut f: impl FnMut(&[u8])) -> bool {
    let Some(h) = open_rw(&hi.path).or_else(|| open(&hi.path, GENERIC_READ.0)) else {
        logf!("failed to open {}", hi.path);
        return false;
    };
    let raw = h.0.0 as isize;
    readers.handles.lock().unwrap().push(raw);
    let n = if hi.in_len < 8 { 65 } else { hi.in_len as usize };
    let mut buf = vec![0u8; n];
    let r = loop {
        if readers.stop.load(Ordering::Acquire) {
            break true;
        }
        let mut got = 0u32;
        if let Err(e) = unsafe { ReadFile(h.0, Some(&mut buf), Some(&mut got), None) } {
            if !readers.stop.load(Ordering::Acquire) {
                logf!("reading {} stopped: {e}", hi.path);
            }
            break true;
        }
        f(&buf[..got as usize]);
    };
    // out of the list before the handle closes: Readers::stop never cancels a closed one
    readers.handles.lock().unwrap().retain(|x| *x != raw);
    drop(h);
    r
}

// ---------- several receivers: each has a session of its own ----------

/// The container of the audio device that plays now (audio::refresh_out): its receiver is the active one
/// (the window, the tray and the balance follow it).
static PREFER: Mutex<Option<[u8; 16]>> = Mutex::new(None);
/// A session runs with the native Arctis 7+ receiver / with a described one.
static RUN_NATIVE: AtomicBool = AtomicBool::new(false);
static RUN_GENERIC: AtomicBool = AtomicBool::new(false);
/// Wakes the search for receivers (a device came or a session ended).
static RESCAN: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// (audio thread) The output that plays now, by its container.
pub fn prefer(c: Option<[u8; 16]>) {
    *PREFER.lock().unwrap() = c;
    if headset::choose(c) {
        util::post_ui(crate::WM_HID, 0);
    }
}

/// Looks for receivers again now (audio devices changed: one may have been plugged in).
pub fn rescan() {
    let (m, c) = &RESCAN;
    *m.lock().unwrap() = true;
    c.notify_one();
}

fn clamp_pct(v: i32) -> i32 {
    v.clamp(0, 100)
}

fn notify() {
    headset::choose(*PREFER.lock().unwrap());
    util::post_ui(crate::WM_HID, 0);
}

// ---------- the current headset (for commands from the window) ----------

/// The native driver's command interface (mi_03), when an Arctis 7+ is connected.
static NATIVE: Mutex<Option<HidIface>> = Mutex::new(None);
/// The generic session, when a described headset is connected.
static GENERIC: Mutex<Option<Arc<GenDev>>> = Mutex::new(None);

/// The Arctis 7+ receiver (its own settings and equalizer go there even while another headset is active).
fn native() -> Option<HidIface> {
    NATIVE.lock().unwrap().clone()
}

/// The described headset, when it is the active one (commands from the window go to it).
pub fn generic() -> Option<Arc<GenDev>> {
    if headset::active() != Some(headset::SLOT_GENERIC) {
        return None;
    }
    GENERIC.lock().unwrap().clone()
}

/// How often the headset is asked for its state: more often for output auto-switching, so that
/// switching the headset on or off is noticed in a few seconds.
fn status_every() -> Duration {
    if config::get().auto_switch { Duration::from_secs(4) } else { Duration::from_secs(45) }
}

static SAVE_GEN: AtomicU64 = AtomicU64::new(0);

/// Writing to the headset flash (0x09) — once after a pause, so a series of changes does not wear it.
pub fn schedule_save(st: &HidIface) {
    let g = SAVE_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let st = st.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(900));
        if SAVE_GEN.load(Ordering::Relaxed) == g {
            write_report(&st, &[0, 0x09]);
            logf!("settings saved to headset");
        }
    });
}

/// A one-argument command + saving (0x09), as in HeadsetControl.
pub fn send_device_cmd(cmd: u8, arg: u8) -> bool {
    let Some(st) = native() else { return false };
    let ok = write_report(&st, &[0, cmd, arg]);
    schedule_save(&st);
    logf!("command 0x{cmd:02x}({arg}): {ok}");
    ok
}

/// Applies sidetone, auto-off, mic gain and the mute light, if the user changed them.
fn apply_device_settings() {
    let c = config::get();
    if (0..=3).contains(&c.sidetone) {
        send_device_cmd(0x39, c.sidetone as u8);
    }
    if (0..=90).contains(&c.auto_off) {
        send_device_cmd(0xa3, c.auto_off as u8);
    }
    if (1..=7).contains(&c.mic_gain) {
        send_device_cmd(0x37, c.mic_gain as u8);
    }
    if (0..=3).contains(&c.mute_led) {
        send_device_cmd(0xae, crate::devices::mute_led_value(c.mute_led));
    }
}

static EQ_PEND: Mutex<(Option<Vec<f64>>, bool)> = Mutex::new((None, false));
/// The curve changed since it was last written to the headset flash.
static EQ_UNSAVED: AtomicBool = AtomicBool::new(false);
/// The receiver was (re)found: the curve goes to the headset once it reports being on.
static EQ_NEED: AtomicBool = AtomicBool::new(false);

/// The curve last sent: every curve command makes the headset click faintly, so the same one is not
/// sent twice.
static EQ_LAST: Mutex<Vec<f64>> = Mutex::new(Vec::new());

/// The curve from the settings again (the headset keeps only its flash copy over a power cycle).
fn reapply_eq() {
    let c = config::get();
    if c.eq.len() == 10 && !c.eq_preset.is_empty() {
        EQ_LAST.lock().unwrap().clear();
        send_eq_latest(&c.eq);
    }
}

/// A curve from the window or a profile (written to the flash on exit).
pub fn queue_eq(bands: &[f64]) {
    EQ_UNSAVED.store(true, Ordering::Relaxed);
    send_eq_latest(bands);
}

/// Curves are written one after another and only the last: otherwise parallel writes while
/// dragging could reach the headset out of order.
fn send_eq_latest(bands: &[f64]) {
    {
        let mut g = EQ_PEND.lock().unwrap();
        g.0 = Some(bands.to_vec());
        if g.1 {
            return;
        }
        g.1 = true;
    }
    std::thread::spawn(|| {
        loop {
            let b = {
                let mut g = EQ_PEND.lock().unwrap();
                match g.0.take() {
                    Some(b) => b,
                    None => {
                        g.1 = false;
                        return;
                    }
                }
            };
            send_eq(&b);
        }
    });
}

/// The built-in 10-band headset equalizer (−12…+12 dB, 0.5 steps). Not written to the flash here:
/// a flash write right after a change makes the sound crackle for a second or two; the curve is
/// written once on exit (as Linux-Arctis-Manager does) and sent again on every (re)connection.
fn send_eq(bands: &[f64]) -> bool {
    let Some(st) = native() else { return false };
    if bands.len() != 10 {
        return false;
    }
    {
        let mut last = EQ_LAST.lock().unwrap();
        if *last == bands {
            return true;
        }
        *last = bands.to_vec();
    }
    let mut buf = vec![0u8, 0x33];
    for v in bands {
        buf.push((0x18 + (v.clamp(-12.0, 12.0) * 2.0).round() as i32) as u8);
    }
    buf.push(0);
    let ok = write_report(&st, &buf);
    logf!("equalizer: {bands:?}");
    ok
}

fn native_name(pid: u16) -> &'static str {
    match pid {
        0x2212 => "Arctis 7P+",
        0x2216 => "Arctis 7X+",
        0x2236 => "Arctis 7+ Destiny",
        _ => "Arctis 7+",
    }
}

/// Looks for receivers and starts a session with each (an Arctis 7+ and a described headset can be
/// connected at once); runs forever on a thread of its own.
pub fn hid_loop() {
    let pids = device_pids(DEVICES_JSON).unwrap_or_else(|e| {
        logf!("device definitions are corrupted: {e}");
        Default::default()
    });
    let mut devs: Option<Vec<DevConfig>> = None;
    loop {
        let (nr, gr) = (RUN_NATIVE.load(Ordering::Acquire), RUN_GENERIC.load(Ordering::Acquire));
        if !nr || !gr {
            let all = enum_hid();
            if !nr && let (Some(st), Some(mix)) = find_arctis(&all) {
                RUN_NATIVE.store(true, Ordering::Release);
                std::thread::spawn(move || {
                    run_native(mix, st);
                    RUN_NATIVE.store(false, Ordering::Release);
                    rescan();
                });
            }
            // a described headset on another receiver than the native one; the full descriptions are kept
            // only while one is found by its code and not matched yet
            let others: Vec<HidIface> = all.into_iter().filter(|h| !NATIVE_PIDS.contains(&h.pid)).collect();
            let known = !gr && others.iter().any(|h| pids.contains(&(h.pid as i32)));
            if known && devs.is_none() {
                devs = parse_devices(DEVICES_JSON).ok();
            }
            if !known {
                devs = None;
            }
            if let Some(d) = devs.as_deref() {
                if let Some(g) = match_generic(d, &others) {
                    devs = None;
                    RUN_GENERIC.store(true, Ordering::Release);
                    std::thread::spawn(move || {
                        run_generic(Arc::new(g));
                        RUN_GENERIC.store(false, Ordering::Release);
                        rescan();
                    });
                }
            }
        }
        // a new audio device or an ended session wakes the search; while a receiver is talked to,
        // another one is looked for rarely otherwise
        let wait = if RUN_NATIVE.load(Ordering::Acquire) || RUN_GENERIC.load(Ordering::Acquire) { Duration::from_secs(30) } else { Duration::from_secs(3) };
        let (m, c) = &RESCAN;
        let g = m.lock().unwrap();
        let (mut g, _) = c.wait_timeout_while(g, wait, |r| !*r).unwrap();
        *g = false;
    }
}

/// The tested Arctis 7+ driver (returns when the connection is lost).
fn run_native(mix: HidIface, st: HidIface) {
    headset::open_slot(headset::SLOT_NATIVE);
    {
        let mut h = headset::lock_slot(headset::SLOT_NATIVE);
        h.connected = true;
        h.dev_name = native_name(st.pid).into();
        h.container = st.container;
    }
    *NATIVE.lock().unwrap() = Some(st.clone());
    EQ_NEED.store(true, Ordering::Relaxed);
    notify();
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(1500));
        apply_device_settings();
    });
    logf!("headset found: {} (PID {:04x}), mi_05 in={}, mi_03 in={} out={}", native_name(st.pid), st.pid, mix.in_len, st.in_len, st.out_len);

    let stop = Arc::new(AtomicBool::new(false));
    let readers = Readers::default();
    let (wake_tx, wake_rx) = mpsc::channel::<()>();
    let last = Arc::new(Mutex::new(None::<(i32, i32)>)); // the dial position (game, chat), once known
    {
        // live ChatMix values
        let (stop, wake, last, rd) = (stop.clone(), wake_tx.clone(), last.clone(), readers.clone());
        std::thread::spawn(move || {
            read_reports(&mix, &rd, |b| {
                if b.len() >= 4 && b[1] == 0x45 {
                    let (g, c) = (clamp_pct(b[2] as i32), clamp_pct(b[3] as i32));
                    *last.lock().unwrap() = Some((g, c));
                    let mut h = headset::lock_slot(headset::SLOT_NATIVE);
                    (h.game, h.chat, h.mix_new) = (g, c, true);
                    drop(h);
                    notify();
                }
            });
            stop.store(true, Ordering::Release);
            let _ = wake.send(());
        });
    }
    {
        // status: battery, power
        let (stop, wake, last, stc, rd) = (stop.clone(), wake_tx.clone(), last.clone(), st.clone(), readers.clone());
        std::thread::spawn(move || {
            let mut last_status = String::new();
            // the previous frame had the headset on and its data known
            let mut was_on = false;
            read_reports(&stc, &rd, |b| {
                if b.len() >= 7 && b[1] == 0xb0 {
                    let s = b[..b.len().min(8)].iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ");
                    if s != last_status {
                        logf!("status: {s}");
                        last_status = s;
                    }
                    // ff ff in place of the dial: the receiver has nothing from the headset yet (right after it
                    // is found or the headset comes on, e.g. 03 00 00 ff ff); its battery byte reads 0 there,
                    // so the battery and the dial wait for a frame with data
                    let known = !(b[5] == 0xff && b[6] == 0xff);
                    let (g, c) = (clamp_pct(b[5] as i32), clamp_pct(b[6] as i32));
                    let mut l = last.lock().unwrap();
                    let mut h = headset::lock_slot(headset::SLOT_NATIVE);
                    let was = h.powered;
                    h.powered = b[2] == 3; // 3 — connected (0..2 — not connected / searching)
                    if h.powered && (EQ_NEED.swap(false, Ordering::Relaxed) || !was) {
                        // found or switched on: the headset starts with the curve from its flash
                        std::thread::spawn(|| {
                            std::thread::sleep(Duration::from_millis(1000));
                            reapply_eq();
                        });
                    }
                    if known {
                        h.batt_pct = (b[3] as i32).min(4) * 25;
                        h.charging = b[4] == 1;
                        h.have_batt = true;
                    }
                    // the status repeats the dial position, but a dial resting near the middle reads a few
                    // points off every time (100/91, 99/100…): it gives the position only until the dial
                    // reports itself, and again when the headset comes on; turning it sends its own frames
                    let on = h.powered && known;
                    let fresh = l.is_none() || !was_on;
                    was_on = on;
                    if on && fresh && *l != Some((g, c)) {
                        (h.game, h.chat, h.mix_new) = (g, c, true);
                        *l = Some((g, c));
                    }
                    drop(h);
                    drop(l);
                    notify();
                }
            });
            stop.store(true, Ordering::Release);
            let _ = wake.send(());
        });
    }
    // periodic status request
    std::thread::sleep(Duration::from_millis(300));
    while !stop.load(Ordering::Acquire) {
        if !write_report(&st, &[0, 0xb0]) {
            logf!("status request failed");
        }
        match wake_rx.recv_timeout(status_every()) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    readers.stop();
    std::thread::sleep(Duration::from_secs(1));
    logf!("lost connection to headset, searching again");
    *NATIVE.lock().unwrap() = None;
    headset::close_slot(headset::SLOT_NATIVE);
    notify();
}


// ---------- headsets from descriptions ----------

pub struct GenDev {
    pub cfg: DevConfig,
    pub pid: u16,
    pub name: String,
    /// the main collection for commands
    cmd: HidIface,
    /// all collections of the command interface that can be written (the main one first)
    outs: Vec<HidIface>,
    /// all collections of the command and event interfaces to read
    ins: Vec<HidIface>,
    pub eq: Option<DevEq>,
    pub schema: Vec<DevSetting>,
    /// the latest field values from the headset
    status: Mutex<HashMap<String, FVal>>,
    eq_pending: Mutex<Option<Vec<f64>>>,
    save_gen: AtomicU64,
    tx: Mutex<Option<Sender<Msg>>>,
}

type Req = Box<dyn FnOnce(&mut Session) + Send>;

enum Msg {
    Report(Vec<u8>),
    Req(Req),
    Lost,
}

/// A headset found from the descriptions. In Windows every HID collection is a device of its own
/// (unlike hidraw in Linux), so all collections of the needed interfaces are taken: commands can go
/// into one and events (ChatMix, buttons) come from another.
fn match_generic(devs: &[DevConfig], all: &[HidIface]) -> Option<GenDev> {
    // a companion (the GameBuds case) only when its main device is not there
    for comp in [false, true] {
        for hi in all {
            for c in devs.iter().filter(|c| c.device.companion == comp) {
                if !c.has_pid(hi.pid as i32) {
                    continue;
                }
                let want = c.device.hid.command.as_ref();
                // the described interface missing — a vendor collection of another one, only where the description's
                // interface is in doubt (engine::quirks: Arctis 9); for the rest it may just not be enumerated yet
                let other = || c.device.hid.any_iface.then(|| pick_vendor(all, hi.pid)).flatten();
                let Some(cmd) = pick_iface(all, hi.pid, want, c.device.hid.usage_page, true).or_else(|| pick_iface(all, hi.pid, want, 0, true)).or_else(other) else {
                    continue;
                };
                let sync_mi = c.device.hid.sync.as_ref().map(|s| s.interface).unwrap_or(cmd.mi);
                let mut outs = vec![cmd.clone()];
                let mut ins = Vec::new();
                for x in all.iter().filter(|x| x.pid == hi.pid) {
                    if x.mi == cmd.mi && x.path != cmd.path && (x.out_len > 0 || x.feat_len > 0) {
                        outs.push(x.clone());
                    }
                    if (x.mi == cmd.mi || x.mi == sync_mi) && x.in_len > 0 {
                        ins.push(x.clone());
                    }
                }
                let name = c.variant_name(hi.pid as i32);
                return Some(GenDev {
                    name: short_model(&name),
                    pid: hi.pid,
                    // the parametric one is not checked on a real headset yet: only for testers (untestedEq)
                    eq: c.detect_eq().filter(|e| e.kind != "parametric" || config::get().untested_eq).map(|mut e| {
                        e.shown = SHOWN_EQ.contains(&c.file.as_str());
                        e
                    }),
                    schema: c.settings_schema(),
                    cfg: c.clone(),
                    cmd,
                    outs,
                    ins,
                    status: Mutex::new(HashMap::new()),
                    eq_pending: Mutex::new(None),
                    save_gen: AtomicU64::new(0),
                    tx: Mutex::new(None),
                });
            }
        }
    }
    None
}

/// The model as the headset is called (and as Windows names its devices): without the brand and the
/// variant notes — "SteelSeries Arctis 7 (2019 refresh)" → "Arctis 7".
fn short_model(name: &str) -> String {
    let n = name.strip_prefix("SteelSeries ").unwrap_or(name);
    let n = match n.find(" (") {
        Some(i) if n.ends_with(')') => &n[..i],
        _ => n,
    };
    n.trim().to_string()
}

fn pick_iface(all: &[HidIface], pid: u16, want: Option<&super::engine::DevIface>, page: i32, need_out: bool) -> Option<HidIface> {
    let mut best: Option<&HidIface> = None;
    for hi in all.iter().filter(|h| h.pid == pid) {
        if want.is_some_and(|w| hi.mi != w.interface) {
            continue;
        }
        let up = want.filter(|w| w.usage_page != 0).map(|w| w.usage_page).unwrap_or(page);
        if up != 0 && hi.usage_page as i32 != up {
            continue;
        }
        if need_out && hi.out_len == 0 && hi.feat_len == 0 {
            continue;
        }
        if best.is_none_or(|b| hi.out_len > b.out_len) {
            best = Some(hi);
        }
    }
    best.cloned()
}

/// A vendor-defined collection (usage page 0xFFxx) that can be written; the largest output report first.
fn pick_vendor(all: &[HidIface], pid: u16) -> Option<HidIface> {
    all.iter().filter(|h| h.pid == pid && h.usage_page >= 0xff00 && (h.out_len > 0 || h.feat_len > 0)).max_by_key(|h| h.out_len).cloned()
}

impl GenDev {
    pub fn key(&self, api: &str) -> String {
        format!("{:04x}:{api}", self.pid)
    }

    pub fn status_value(&self, field: &str) -> Option<f64> {
        match self.status.lock().unwrap().get(field) {
            Some(FVal::Num(v)) => Some(*v),
            _ => None,
        }
    }

    /// Runs f on the session thread (waits at most 2 s to hand it over).
    pub fn post(&self, f: impl FnOnce(&mut Session) + Send + 'static) {
        if let Some(tx) = self.tx.lock().unwrap().as_ref() {
            let _ = tx.send(Msg::Req(Box::new(f)));
        }
    }

    /// From the window: frequent changes merge — only the last curve is written.
    pub fn set_eq(self: &Arc<Self>, bands: &[f64]) {
        let first = {
            let mut p = self.eq_pending.lock().unwrap();
            let first = p.is_none();
            *p = Some(bands.to_vec());
            first
        };
        if first {
            self.post(|s| {
                let b = s.g.eq_pending.lock().unwrap().take();
                if let Some(b) = b {
                    s.apply_eq(&b, true);
                    s.g.schedule_save();
                }
            });
        }
    }

    pub fn schedule_save(self: &Arc<Self>) {
        if !self.cfg.apis.contains_key("save_to_flash") {
            return;
        }
        let token = self.save_gen.fetch_add(1, Ordering::Relaxed) + 1;
        let me = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            if me.save_gen.load(Ordering::Relaxed) == token {
                me.post(|s| s.call("save_to_flash"));
            }
        });
    }
}

/// The session thread state of a described headset.
pub struct Session {
    pub g: Arc<GenDev>,
    wh: HashMap<String, H>,
    /// the collection the last write went through
    ok_out: usize,
    rx: Receiver<Msg>,
    /// requests that came while waiting for an answer
    pending: VecDeque<Req>,
    inited: bool,
    last_init: Instant,
    lost: bool,
    /// a part of the state not answered — logged once
    partial_logged: bool,
    /// state requests not answered in a row while others were (left out from 3 on)
    silent: HashMap<String, u8>,
    /// the equalizer curve in the headset now (None — unknown: the whole curve is sent)
    eq_sent: Option<Vec<f64>>,
}

fn btoi(b: bool) -> i32 {
    b as i32
}

fn label_state(v: &FVal) -> Option<bool> {
    let FVal::Str(s) = v else { return None };
    let s = s.to_uppercase();
    if ["NOT_CONNECTED", "DISCONNECTED", "OFFLINE", "NOT_PAIRED", "SEARCHING"].iter().any(|k| s.contains(k)) {
        return Some(false);
    }
    if s.contains("CONNECTED") || s.contains("ONLINE") {
        return Some(true);
    }
    None
}

impl Session {
    fn write_to(&self, x: &HidIface, payload: &[u8], feature: bool) -> Result<(), String> {
        let h = self.wh.get(&x.path).ok_or("collection not open")?;
        if feature {
            let n = x.feat_len as usize;
            if n == 0 {
                return Err("no feature reports".into());
            }
            let mut buf = vec![0u8; n];
            let k = payload.len().min(n);
            buf[..k].copy_from_slice(&payload[..k]);
            return if unsafe { HidD_SetFeature(h.0, buf.as_ptr() as *const c_void, n as u32) } { Ok(()) } else { Err("HidD_SetFeature failed".into()) };
        }
        let n = x.out_len as usize;
        if n == 0 {
            return Err("no output reports".into());
        }
        let mut buf = vec![0u8; n];
        let k = payload.len().min(n);
        buf[..k].copy_from_slice(&payload[..k]);
        let raw = h.0.0 as isize;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut w = 0u32;
            let r = unsafe { WriteFile(HANDLE(raw as *mut _), Some(&buf), Some(&mut w), None) };
            let _ = tx.send(r.map_err(|e| e.to_string()));
        });
        rx.recv_timeout(Duration::from_millis(1500)).unwrap_or(Err("write did not complete".into()))
    }

    /// The collection that worked last time first, then the rest.
    fn write(&mut self, payload: &[u8], feature: bool) -> Result<(), String> {
        let mut last = Err("no writable collection".to_string());
        let n = self.g.outs.len();
        for k in 0..n {
            let i = (self.ok_out + k) % n;
            match self.write_to(&self.g.outs[i], payload, feature) {
                Err(e) => last = Err(e),
                Ok(()) => {
                    if i != self.ok_out {
                        logf!("writing via collection {:04x} (mi_{:02x})", self.g.outs[i].usage_page, self.g.outs[i].mi);
                        self.ok_out = i;
                    }
                    return Ok(());
                }
            }
        }
        last
    }

    fn write_api(&mut self, api: &str, vals: &HashMap<String, FVal>) -> Result<(), String> {
        self.write_api_where(api, vals, |_| true)
    }

    /// The same, leaving out the messages `keep` turns down.
    fn write_api_where(&mut self, api: &str, vals: &HashMap<String, FVal>, keep: impl Fn(&[u8]) -> bool) -> Result<(), String> {
        let acts = self.g.cfg.prepare_write(api, vals)?;
        let mut sent = false;
        for a in acts {
            if !a.sleep.is_zero() {
                std::thread::sleep(a.sleep);
                sent = false;
                continue;
            }
            if !keep(&a.payload) {
                continue;
            }
            // frames of one write at least 30 ms apart: Nova 7 Gen 2 drops faster ones (Arctis Sound Manager)
            if sent {
                std::thread::sleep(Duration::from_millis(30));
            }
            self.write(&a.payload, a.feature)?;
            sent = true;
        }
        Ok(())
    }

    /// A request and its answer (the answer repeats the first 2 bytes of the request).
    fn read_api(&mut self, api: &str) -> Result<HashMap<String, FVal>, String> {
        let (req, op) = self.g.cfg.prepare_read(api)?;
        if op.transport == "HID_FEATURE" {
            self.write(&req, true)?;
            for x in self.g.outs.iter().filter(|x| x.feat_len > 0) {
                let mut buf = vec![0u8; x.feat_len as usize];
                buf[0] = req[0];
                let Some(h) = self.wh.get(&x.path) else { continue };
                if unsafe { HidD_GetFeature(h.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) } {
                    return self.g.cfg.deserialize(api, &buf);
                }
            }
            return Err(format!("failed to read {api}"));
        }
        // answers left from earlier requests are handled as events
        while let Ok(m) = self.rx.try_recv() {
            self.take(m);
        }
        self.write(&req, false)?;
        for _ in 0..8 {
            match self.rx.recv_timeout(Duration::from_secs(1)) {
                Ok(Msg::Report(r)) => {
                    if r.len() >= 2 && r[0] == req[0] && r[1] == req[1] {
                        return self.g.cfg.deserialize(api, &r);
                    }
                    self.handle_report(&r);
                }
                Ok(m) => self.take(m),
                Err(RecvTimeoutError::Timeout) => return Err(format!("no response to {api}")),
                Err(RecvTimeoutError::Disconnected) => return Err("connection lost".into()),
            }
            if self.lost {
                return Err("connection lost".into());
            }
        }
        Err(format!("too many unrelated reports before response to {api}"))
    }

    /// A message that came while waiting: reports are handled, requests wait their turn.
    fn take(&mut self, m: Msg) {
        match m {
            Msg::Report(r) => self.handle_report(&r),
            Msg::Req(f) => self.pending.push_back(f),
            Msg::Lost => self.lost = true,
        }
    }

    fn handle_report(&mut self, r: &[u8]) {
        let Some(d) = self.g.cfg.dispatch(r) else { return };
        if let Some(e) = d.emit {
            self.apply_emit(&e);
        }
        for s in d.side_effects {
            self.pending.push_back(Box::new(move |se: &mut Session| se.call(&s)));
        }
    }

    fn apply_emit(&mut self, e: &super::engine::DevEmit) {
        let mut changed = false;
        {
            let mut st = self.g.status.lock().unwrap();
            for (k, v) in &e.fields {
                changed |= st.insert(k.clone(), v.clone()).as_ref() != Some(v);
            }
        }
        let mut h = headset::lock_slot(headset::SLOT_GENERIC);
        let before = (h.game, h.chat, h.batt_pct, h.have_batt, h.powered, h.charging, h.mic_muted, h.extra_batt.clone());
        let (mut game, mut chat) = (-1, -1);
        for (k, v) in &e.fields {
            let role = e.roles.get(k).map(|s| s.as_str()).unwrap_or("");
            let num = if let FVal::Num(n) = v { Some(*n) } else { None };
            if role == "chatmix_game" {
                game = clamp_pct(v.num().round() as i32);
            } else if role == "chatmix_chat" {
                chat = clamp_pct(v.num().round() as i32);
            } else if role == "battery_headset" && num.is_some() {
                h.batt_pct = clamp_pct(v.num().round() as i32);
                h.have_batt = true;
            } else if e.signal.contains("connection") || k.contains("connection") || k.contains("rf_status") {
                if let Some(on) = label_state(v) {
                    h.powered = on;
                }
            } else if role.starts_with("battery_") && num.is_some() {
                h.extra_batt.insert(role.trim_start_matches("battery_").to_string(), clamp_pct(v.num().round() as i32));
            } else if k.contains("mic_mute") || k == "mic_muted" || e.signal.contains("mic_mute") {
                h.mic_muted = match v {
                    FVal::Num(n) => btoi(*n != 0.0),
                    other => {
                        let s = other.to_string().to_uppercase();
                        btoi(s == "ON" || s == "MUTED" || s == "TRUE")
                    }
                };
            } else if e.signal.contains("charging") || k.contains("charging") {
                if let FVal::Str(s) = v {
                    let s = s.to_uppercase();
                    h.charging = s.contains("CHARGING") && !s.contains("NOT_CHARGING") && !s.contains("DISCHARGING");
                }
            }
        }
        if game >= 0 || chat >= 0 {
            let g = if game < 0 { h.game } else { game };
            let c = if chat < 0 { h.chat } else { chat };
            if (g, c) != (h.game, h.chat) {
                (h.game, h.chat, h.mix_new) = (g, c, true);
            }
        }
        changed |= before != (h.game, h.chat, h.batt_pct, h.have_batt, h.powered, h.charging, h.mic_muted, h.extra_batt.clone());
        let came_on = h.powered && !before.4;
        drop(h);
        if came_on {
            self.silent.clear(); // the headset came on: every request is asked again
            // it starts without the setup and the settings the app keeps (SteelSeries GG sends both again):
            // the Arctis 7 chip forgets its equalizer
            if self.inited {
                self.eq_sent = None;
                self.pending.push_back(Box::new(|s: &mut Session| {
                    s.lifecycle("init");
                    s.reapply();
                }));
            }
        }
        // a state polled often (the Arctis 7 / 9 dial, the mic button) mostly repeats: the window and the tray hear of changes only
        if changed {
            notify();
        }
    }

    /// Reads the state: an error only when nothing answers — a model may not know one of the requests
    /// (the rest still counts).
    fn sync_read(&mut self) -> Result<(), String> {
        let mut err = None;
        let mut any = false;
        let mut failed = Vec::new();
        for e in self.g.cfg.sync_read.clone() {
            // a request the model never answers (3 times in a row while others are answered) is left out:
            // each costs a 1 s wait that holds the dial and the window's commands
            if self.silent.get(&e.strukt).is_some_and(|n| *n >= 3) {
                continue;
            }
            match self.read_api(&e.strukt) {
                Ok(f) => {
                    any = true;
                    self.silent.remove(&e.strukt);
                    for em in self.g.cfg.map_read(&e, &f) {
                        self.apply_emit(&em);
                    }
                }
                Err(x) => {
                    failed.push(e.strukt.clone());
                    err = Some(x);
                }
            }
            if self.lost {
                return Err("connection lost".into());
            }
        }
        // counted only while the headset is on: with it off the receiver answers part of the requests
        if any && headset::lock_slot(headset::SLOT_GENERIC).powered {
            for s in failed {
                *self.silent.entry(s).or_default() += 1;
            }
        }
        match err {
            Some(e) if !any => Err(e),
            Some(e) => {
                if !self.partial_logged {
                    self.partial_logged = true;
                    logf!("state read in part: {e}");
                }
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Parts of the state the headset does not report by itself (Arctis 7, Arctis 9), asked for more often
    /// than the rest: the ChatMix dial and the microphone button while the headset is on, whether it is on
    /// at all always (true — only while on).
    fn quick_reads(&self) -> Vec<(super::engine::DevReadEntry, Duration, bool)> {
        let c = &self.g.cfg;
        let wanted: [(&str, Duration, bool); 3] = [("chatmix", Duration::from_millis(250), true), ("mic_mute", Duration::from_secs(1), true), ("connection", Duration::from_secs(3), false)];
        let mut l = Vec::new();
        for (sig, every, on_only) in wanted {
            if c.events.values().any(|e| e.emit.contains(sig)) {
                continue;
            }
            if let Some(e) = c.sync_read.iter().find(|e| e.maps.iter().any(|m| m.emit.contains(sig))) {
                if !l.iter().any(|(x, _, _): &(super::engine::DevReadEntry, Duration, bool)| x.strukt == e.strukt) {
                    l.push((e.clone(), every, on_only));
                }
            }
        }
        l
    }

    pub fn call(&mut self, name: &str) {
        if let Some((api, field, val)) = simple_call(name) {
            if self.g.cfg.apis.contains_key(api) {
                if let Err(e) = self.write_api(api, &HashMap::from([(field.to_string(), FVal::Num(val))])) {
                    logf!("{name}: {e}");
                }
            }
            return;
        }
        match name {
            "save_to_flash" | "av6x02_init" => {
                if self.g.cfg.apis.contains_key(name) {
                    if let Err(e) = self.write_api(name, &HashMap::new()) {
                        logf!("{name}: {e}");
                    }
                }
            }
            "sync_all" | "send_init_wireless_connection_battery_status" => {
                if let Err(e) = self.sync_read() {
                    logf!("{name}: {e}");
                }
            }
            "discord_certified_set_attributes" => {}
            _ => logf!("unknown call {name}, skipping"),
        }
    }

    fn lifecycle(&mut self, hook: &str) {
        for c in self.g.cfg.lifecycle.get(hook).cloned().unwrap_or_default() {
            self.call(&c.call);
        }
    }

    fn init(&mut self) -> bool {
        self.lifecycle("init");
        if let Err(e) = self.sync_read() {
            logf!("headset not responding yet: {e}");
            return false;
        }
        self.lifecycle("post_init");
        self.reapply();
        true
    }

    /// Settings the user changed in the app (the headset may not remember them).
    fn reapply(&mut self) {
        let c = config::get();
        if let Some(eq) = &self.g.eq {
            if c.eq.len() == eq.bands && !c.eq_preset.is_empty() && c.eq_preset != "flat" {
                self.apply_eq(&c.eq, false);
            }
        }
        let prefix = format!("{:04x}:", self.g.pid);
        for (k, fields) in &c.dev_settings {
            let Some(api) = k.strip_prefix(&prefix) else { continue };
            if !self.g.cfg.apis.contains_key(api) {
                continue;
            }
            for (f, v) in fields {
                self.set_setting(api, f, *v, false);
            }
        }
    }

    /// Writes one field of an API; the other fields come from saved / current / default values.
    pub fn set_setting(&mut self, api: &str, field: &str, value: f64, persist: bool) {
        let stored = config::get().dev_settings.get(&self.g.key(api)).cloned().unwrap_or_default();
        let mut vals = HashMap::new();
        for f in self.g.cfg.out_fields(api) {
            if f.constant.is_some() {
                continue;
            }
            let v = if f.name == field {
                value
            } else if let Some(v) = stored.get(&f.name) {
                *v
            } else if let Some(v) = self.g.status_value(&f.name) {
                v
            } else if f.range.len() == 2 {
                super::engine::any_num(self.g.cfg.resolve(&f.range[0])).unwrap_or(0.0)
            } else {
                0.0
            };
            vals.insert(f.name.clone(), FVal::Num(v));
        }
        if let Err(e) = self.write_api(api, &vals) {
            logf!("setting {api}.{field}={value}: {e}");
            return;
        }
        logf!("setting {api}.{field} = {value}");
        self.g.status.lock().unwrap().insert(field.to_string(), FVal::Num(value));
        if persist {
            let k = self.g.key(api);
            config::set(|c| {
                c.dev_settings.entry(k).or_default().insert(field.to_string(), value);
            });
            self.g.schedule_save();
        }
    }

    pub fn apply_eq(&mut self, bands: &[f64], persist: bool) {
        let Some(eq) = self.g.eq.clone() else { return };
        // a chip written band by band (AV6X02): only the bands that changed since the last curve go out, as
        // SteelSeries GG does (the whole curve is 84 messages, some 2.5 s)
        let av6 = self.g.cfg.apis.get(&eq.api).and_then(|a| a.write.as_ref()).is_some_and(|w| w.steps.iter().any(|s| s.transform == "builtin:av6x02_eq_gains"));
        let prev = self.eq_sent.take().filter(|_| av6);
        let same = |b: usize| prev.as_ref().is_some_and(|p| p.len() == bands.len() && p.get(b) == bands.get(b));
        let vals = self.g.cfg.eq_values(&eq, bands);
        if let Err(e) = self.write_api_where(&eq.api, &vals, |m| super::builtins::av6_band_of(m).is_none_or(|b| !same(b))) {
            logf!("equalizer: {e}");
            return;
        }
        self.eq_sent = Some(bands.to_vec());
        logf!("equalizer: {bands:?}");
        // Nova Pro / GameDAC: the curve goes into the "custom" slot, but only the selected preset
        // sounds — after writing, exactly that slot is selected (otherwise "Flat" from start stays)
        if !eq.preset_api.is_empty() && eq.has_custom {
            self.set_setting(&eq.preset_api, &eq.preset_field, eq.custom_slot, persist);
        }
    }

    /// A built-in headset preset (game ones too).
    pub fn set_hw_preset(&mut self, v: f64) {
        let Some(eq) = self.g.eq.clone() else { return };
        if !eq.preset_api.is_empty() {
            self.set_setting(&eq.preset_api, &eq.preset_field, v, true);
        }
    }
}

fn run_generic(g: Arc<GenDev>) {
    let (tx, rx) = mpsc::channel::<Msg>();
    *g.tx.lock().unwrap() = Some(tx.clone());
    let mut s = Session { g: g.clone(), wh: HashMap::new(), ok_out: 0, rx, pending: VecDeque::new(), inited: false, last_init: Instant::now(), lost: false, partial_logged: false, silent: HashMap::new(), eq_sent: None };
    for x in &g.outs {
        if let Some(h) = open_rw(&x.path) {
            s.wh.insert(x.path.clone(), h);
        }
    }
    if s.wh.is_empty() {
        logf!("{}: failed to open command interface", g.name);
        *g.tx.lock().unwrap() = None;
        std::thread::sleep(Duration::from_secs(3));
        return;
    }
    let cols: Vec<String> = g.ins.iter().map(|x| format!("mi_{:02x}/{:04x} in={}", x.mi, x.usage_page, x.in_len)).collect();
    logf!(
        "headset found: {} (PID {:04x}, {}), commands mi_{:02x}/{:04x} out={} feat={}, reading: {}",
        g.name,
        g.pid,
        g.cfg.file,
        g.cmd.mi,
        g.cmd.usage_page,
        g.cmd.out_len,
        g.cmd.feat_len,
        cols.join(", ")
    );
    headset::open_slot(headset::SLOT_GENERIC);
    {
        let mut h = headset::lock_slot(headset::SLOT_GENERIC);
        (h.connected, h.powered) = (true, true);
        h.generic = true;
        h.generic_soft_chatmix = g.cfg.apis.contains_key("software_chatmix_status");
        h.generic_wireless = g.cfg.device.capabilities.iter().any(|c| c == "wireless" || c == "battery");
        h.dev_name = g.name.clone();
        h.container = g.cmd.container;
    }
    *GENERIC.lock().unwrap() = Some(g.clone());
    notify();

    let readers = Readers::default();
    for x in g.ins.clone() {
        let (tx, rd) = (tx.clone(), readers.clone());
        std::thread::spawn(move || {
            if read_reports(&x, &rd, |b| {
                let _ = tx.send(Msg::Report(b.to_vec()));
            }) && !rd.stop.load(Ordering::Acquire)
            {
                let _ = tx.send(Msg::Lost); // the collection opened and then reading broke — unplugged
            }
        });
    }

    std::thread::sleep(Duration::from_millis(300));
    s.last_init = Instant::now();
    s.inited = s.init();
    let mut next_tick = Instant::now() + Duration::from_secs(10);
    let quick = s.quick_reads();
    let mut next_quick: Vec<Instant> = quick.iter().map(|(_, every, _)| Instant::now() + *every).collect();
    while !s.lost {
        while let Some(f) = s.pending.pop_front() {
            f(&mut s);
        }
        if s.lost {
            break;
        }
        let now = Instant::now();
        if let Some(i) = next_quick.iter().position(|t| now >= *t) {
            let (e, every, on_only) = &quick[i];
            next_quick[i] = now + *every;
            let on = headset::lock_slot(headset::SLOT_GENERIC).powered;
            if s.inited && (on || !on_only) && s.silent.get(&e.strukt).is_none_or(|n| *n < 3) {
                match s.read_api(&e.strukt) {
                    Ok(f) => {
                        s.silent.remove(&e.strukt);
                        for em in s.g.cfg.map_read(e, &f) {
                            s.apply_emit(&em);
                        }
                    }
                    // a request the model never answers stops being asked (as in sync_read)
                    Err(_) if on => *s.silent.entry(e.strukt.clone()).or_default() += 1,
                    Err(_) => {}
                }
            }
            continue;
        }
        let due = next_quick.iter().copied().fold(next_tick, Instant::min);
        match s.rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
            Ok(Msg::Report(r)) => {
                s.handle_report(&r);
                if !s.inited && s.last_init.elapsed() > Duration::from_secs(3) {
                    s.last_init = Instant::now();
                    s.inited = s.init();
                }
            }
            Ok(Msg::Req(f)) => f(&mut s),
            Ok(Msg::Lost) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) if Instant::now() < next_tick => {} // a quick read is due
            Err(RecvTimeoutError::Timeout) => {
                next_tick = Instant::now() + Duration::from_secs(10);
                if !s.inited {
                    s.inited = s.init();
                } else if let Err(e) = s.sync_read() {
                    logf!("poll: {e}");
                    headset::lock_slot(headset::SLOT_GENERIC).powered = false;
                    notify();
                }
            }
        }
    }
    readers.stop();
    logf!("lost connection to {}", g.name);
    *g.tx.lock().unwrap() = None;
    *GENERIC.lock().unwrap() = None;
    headset::close_slot(headset::SLOT_GENERIC);
    notify();
    std::thread::sleep(Duration::from_secs(1));
}


/// Shutdown calls (saving to flash, switching software ChatMix off).
pub fn shutdown() {
    if EQ_UNSAVED.swap(false, Ordering::Relaxed)
        && let Some(st) = native()
        && headset::lock_slot(headset::SLOT_NATIVE).powered
    {
        // the curve stays in the headset for use without the app
        let ok = write_report(&st, &[0, 0x09]);
        logf!("equalizer saved to headset: {ok}");
    }
    let Some(g) = GENERIC.lock().unwrap().clone() else { return };
    let (tx, rx) = mpsc::channel();
    g.post(move |s| {
        s.lifecycle("shutdown");
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(Duration::from_secs(2));
}
