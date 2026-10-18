//! The local HTTP API for the window page: state, server-sent
//! events, settings changes. Only 127.0.0.1, a random port and a random token in every request.

use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU16, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    time::Duration,
};

use serde::Deserialize;
use serde_json::json;

use crate::{
    audio, autostart,
    config::{self, Config, EqUserPreset},
    fly,
    http::{self, Reply, Request},
    i18n, logf, settings, state, util,
};

const INDEX_HTML: &str = include_str!("../../web/index.html");
const FONT_TTF: &[u8] = include_bytes!("../../web/onest.ttf");

/// Links the page may open in the browser.
pub const OPEN_URLS: [&str; 5] = [
    "https://vb-audio.com/Cable/",
    "https://shop.vb-audio.com/en/audio-apps/12-vb-cable-ab.html",
    "https://github.com/Swiftlyx/OpenChatMix",
    "https://github.com/Fenderenko/Arcmix",
    crate::update::RELEASES_PAGE,
];

static PORT: AtomicU16 = AtomicU16::new(0);
static TOKEN: Mutex<String> = Mutex::new(String::new());
static SUBS: Mutex<Vec<SyncSender<()>>> = Mutex::new(Vec::new());

pub fn port() -> u16 {
    PORT.load(Ordering::Relaxed)
}

pub fn url() -> String {
    format!("http://127.0.0.1:{}/?k={}", port(), TOKEN.lock().unwrap())
}

/// Tells every open page the state changed (it fetches the state anew).
pub fn notify_subs() {
    SUBS.lock().unwrap().retain(|s| !matches!(s.try_send(()), Err(TrySendError::Disconnected(_))));
}

fn random_token() -> String {
    use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
    let mut b = [0u8; 16];
    let _ = unsafe { BCryptGenRandom(None, &mut b, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn start() {
    let ln = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            logf!("server: {e}");
            return;
        }
    };
    let port = ln.local_addr().map(|a| a.port()).unwrap_or(0);
    PORT.store(port, Ordering::Relaxed);
    *TOKEN.lock().unwrap() = random_token();
    autostart::refresh_cache();
    let handler: Arc<http::Handler> = Arc::new(handle);
    std::thread::Builder::new().name("server".into()).spawn(move || http::serve(ln, handler)).ok();
    logf!("local server on 127.0.0.1:{port}");
}

fn handle(r: &Request, s: &mut TcpStream) -> Reply {
    let host = format!("127.0.0.1:{}", port());
    if r.path == "/font.ttf" {
        if r.header("Host") != Some(host.as_str()) {
            return Reply::Status(403, "Forbidden");
        }
        return Reply::Body { ctype: "font/ttf", cache: "max-age=86400", body: FONT_TTF.to_vec() };
    }
    // the page, the API and icons: only this host, the token, and no other origin
    let token = TOKEN.lock().unwrap().clone();
    if r.header("Host") != Some(host.as_str()) || r.query("k") != token {
        return Reply::Status(403, "Forbidden");
    }
    if let Some(o) = r.header("Origin") {
        if o != format!("http://{host}") {
            return Reply::Status(403, "Forbidden");
        }
    }
    route(r, s)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AppReq {
    exe: String,
    cat: String,
    /// app volume 0..100
    vol: Option<f64>,
    /// its own output device ("" — as in Windows)
    out: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OnReq {
    on: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpName {
    op: String,
    name: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct EqUserReq {
    op: String,
    name: String,
    /// rename: the new name
    to: String,
    icon: String,
    bands: Vec<f64>,
    freqs: Vec<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OutDevReq {
    op: String,
    id: String,
    ids: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MicReq {
    level: Option<i32>,
    mute: Option<bool>,
    dev: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct EqReq {
    bands: Vec<f64>,
    preset: String,
    /// "" — the headset; "game" / "chat" — the software equalizer of a category
    target: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RouteReq {
    on: Option<bool>,
    game: Option<String>,
    chat: Option<String>,
    media: Option<String>,
    out: Option<String>,
    auto: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MicFxReq {
    on: Option<bool>,
    #[serde(rename = "in")]
    input: Option<String>,
    out: Option<String>,
    gate: Option<f64>,
    ceil: Option<f64>,
    denoise: Option<bool>,
}

fn route(r: &Request, s: &mut TcpStream) -> Reply {
    match r.path.as_str() {
        "/" => {
            logf!("page requested by {}", r.header("User-Agent").unwrap_or(""));
            let page = INDEX_HTML.replacen(r#"/*I18N*/{"lang":"uk","t":{},"langs":[]}"#, &i18n::payload(&config::get().lang), 1);
            Reply::Body { ctype: "text/html; charset=utf-8", cache: "no-store", body: page.into_bytes() }
        }
        "/api/state" => http::json(&state::build()),
        "/api/events" => {
            events(s);
            Reply::Taken
        }
        "/api/pos" => {
            #[derive(Deserialize, Default)]
            #[serde(default)]
            struct V {
                v: i32,
            }
            if let Some(v) = r.json::<V>() {
                util::post_ui(crate::WM_SET_POS, (v.v.clamp(-100, 100) + 200) as usize);
            }
            Reply::NoContent
        }
        "/api/cfg" => {
            if let Some(p) = r.json::<crate::cfgpatch::CfgPatch>() {
                crate::cfgpatch::apply(p);
            }
            Reply::NoContent
        }
        "/api/app" => {
            if let Some(v) = r.json::<AppReq>() {
                match (v.vol, v.out) {
                    (Some(vol), _) => audio::set_app_volume(&v.exe, (vol / 100.0) as f32),
                    (None, Some(out)) => {
                        let e = crate::appinfo::canonical(&v.exe.trim().to_lowercase()).to_string();
                        if !e.is_empty() {
                            config::set(|c| {
                                if out.is_empty() {
                                    c.app_outs.remove(&e);
                                } else {
                                    c.app_outs.insert(e, out);
                                }
                            });
                            audio::reapply();
                            notify_subs();
                        }
                    }
                    (None, None) => {
                        set_app_cat(&v.exe, &v.cat);
                        util::post_ui(crate::WM_CFG_CHG, 0);
                    }
                }
            }
            Reply::NoContent
        }
        "/api/app/remove" => {
            if let Some(v) = r.json::<AppReq>() {
                let e = v.exe.to_lowercase();
                config::set(|c| c.pinned.retain(|x| *x != e));
                util::post_ui(crate::WM_CFG_CHG, 0);
            }
            Reply::NoContent
        }
        "/api/quit" => {
            util::post_ui(crate::WM_QUIT_REQ, 0);
            Reply::NoContent
        }
        "/api/hide" => {
            util::post_ui(crate::WM_HIDE_FLY, 0);
            Reply::NoContent
        }
        "/api/hkcapture" => {
            if let Some(v) = r.json::<OnReq>() {
                util::post_ui(crate::WM_HK_CAPTURE, v.on as usize);
            }
            Reply::NoContent
        }
        "/api/outdev" => {
            if let Some(v) = r.json::<OutDevReq>() {
                if (!v.id.is_empty() || v.op == "order") && ["up", "down", "order", "on", "off", "forget"].contains(&v.op.as_str()) {
                    audio::out_dev_op(&v.op, &v.id, v.ids);
                }
            }
            Reply::NoContent
        }
        "/api/output" => {
            #[derive(Deserialize, Default)]
            #[serde(default)]
            struct V {
                dev: String,
            }
            if let Some(v) = r.json::<V>().filter(|v| !v.dev.is_empty()) {
                audio::set_output(&v.dev);
            }
            Reply::NoContent
        }
        "/api/mic" => {
            if let Some(v) = r.json::<MicReq>() {
                if let Some(d) = v.dev {
                    config::set(|c| c.mic_dev = d);
                    audio::refresh_mic();
                    std::thread::spawn(|| crate::router::sync(&config::get())); // processing and "listen to yourself" take this microphone
                    notify_subs();
                }
                if let Some(l) = v.level {
                    audio::set_mic_level(l.clamp(0, 100));
                }
                if let Some(m) = v.mute {
                    audio::set_mic_mute(m);
                }
            }
            Reply::NoContent
        }
        "/api/eq" => {
            if let Some(v) = r.json::<EqReq>() {
                set_eq(v);
            }
            Reply::NoContent
        }
        "/api/dev" => {
            #[derive(Deserialize, Default)]
            #[serde(default)]
            struct V {
                api: String,
                field: String,
                value: f64,
            }
            if let Some(v) = r.json::<V>() {
                crate::devices::set_setting(&v.api, &v.field, v.value);
            }
            Reply::NoContent
        }
        "/api/eqpreset" => {
            #[derive(Deserialize, Default)]
            #[serde(default)]
            struct V {
                value: f64,
            }
            if let Some(v) = r.json::<V>() {
                crate::devices::set_hw_preset(v.value);
            }
            Reply::NoContent
        }
        "/api/alive" => {
            // the page painted after the window was shown (size in physical pixels)
            #[derive(Deserialize, Default)]
            #[serde(default)]
            struct V {
                w: f64,
                h: f64,
            }
            let v = r.json::<V>().unwrap_or_default();
            fly::mark_alive(v.w, v.h);
            Reply::NoContent
        }
        "/api/ping" => {
            logf!("UI rendered: {}", r.query("v"));
            Reply::NoContent
        }
        "/api/procs" => http::json(&list_procs()),
        "/api/audiodevs" => {
            let (render, capture) = audio::list_devices();
            http::json(&json!({ "render": render, "capture": capture }))
        }
        "/api/route" => {
            if let Some(v) = r.json::<RouteReq>() {
                config::set(|c| {
                    let rc = &mut c.route;
                    if let Some(x) = v.on {
                        rc.on = x;
                    }
                    if let Some(x) = v.game {
                        rc.game = x;
                    }
                    if let Some(x) = v.chat {
                        rc.chat = x;
                    }
                    if let Some(x) = v.media {
                        rc.media = x;
                    }
                    if let Some(x) = v.out {
                        rc.out = x;
                    }
                    if let Some(x) = v.auto {
                        rc.auto = x;
                    }
                });
                util::post_ui(crate::WM_CFG_CHG, 0);
            }
            Reply::NoContent
        }
        "/api/micfx" => {
            if let Some(v) = r.json::<MicFxReq>() {
                config::set(|c| {
                    let m = &mut c.mic_fx;
                    if let Some(x) = v.on {
                        m.on = x;
                    }
                    if let Some(x) = v.input {
                        c.mic_dev = x; // older windows: now the same microphone as on the main screen
                    }
                    let m = &mut c.mic_fx;
                    if let Some(x) = v.out {
                        m.out = x;
                    }
                    if let Some(x) = v.gate {
                        m.gate = x.clamp(-100.0, 0.0);
                    }
                    if let Some(x) = v.ceil {
                        m.ceil = x.clamp(-40.0, 0.0);
                    }
                    if let Some(x) = v.denoise {
                        m.denoise = x;
                    }
                });
                std::thread::spawn(|| crate::router::sync(&config::get()));
                notify_subs();
            }
            Reply::NoContent
        }
        "/api/miclevel" => {
            let (i, o, open, ok) = crate::router::mic_meter();
            http::json(&json!({ "in": (i * 10.0).round() / 10.0, "out": (o * 10.0).round() / 10.0, "open": open, "ok": ok }))
        }
        "/api/duckmeter" => http::json(&audio::duck_meter()),
        "/api/winducking" => {
            util::run_on_ui(|| settings::open_win_sound(3));
            Reply::NoContent
        }
        "/api/winsound" => {
            util::run_on_ui(|| settings::open_win_sound(0));
            Reply::NoContent
        }
        "/api/micpeak" => {
            let (db, ok) = crate::router::mic_peak_db();
            http::json(&json!({ "db": (db * 10.0).round() / 10.0, "ok": ok }))
        }
        "/api/monitor" => {
            if r.method == "POST" {
                if let Some(v) = r.json::<OpName>() {
                    crate::router::monitor_set(&v.op);
                }
            }
            http::json(&crate::router::monitor_status())
        }
        "/api/equser" => {
            if let Some(v) = r.json::<EqUserReq>() {
                eq_user(v);
                notify_subs();
            }
            Reply::NoContent
        }
        "/api/settings" => {
            if let Some(v) = r.json::<OpName>() {
                match v.op.as_str() {
                    "export" => util::run_on_ui(settings::export),
                    "import" => util::run_on_ui(settings::import),
                    _ => {}
                }
            }
            Reply::NoContent
        }
        "/api/open" => {
            #[derive(Deserialize, Default)]
            #[serde(default)]
            struct V {
                url: String,
            }
            if let Some(v) = r.json::<V>() {
                if let Some(i) = OPEN_URLS.iter().position(|u| *u == v.url) {
                    util::post_ui(crate::WM_OPEN_URL, i);
                }
            }
            Reply::NoContent
        }
        "/api/update" => {
            // a request to GitHub only on the button press; the answer waits for it (own connection thread)
            let latest = crate::update::latest();
            let newer = latest.as_deref().is_some_and(|v| crate::version::newer(v, crate::APP_VER));
            logf!("update check: latest {}, newer {newer}", latest.as_deref().unwrap_or("-"));
            http::json(&json!({ "latest": latest, "newer": newer }))
        }
        "/api/vst/scan" => http::json(&crate::vst::scan()),
        "/api/vst/pick" => {
            util::run_on_ui(crate::vst::pick);
            Reply::NoContent
        }
        "/api/vst" => match r.json::<serde_json::Value>() {
            Some(v) => {
                crate::vst::op(&v);
                Reply::NoContent
            }
            None => Reply::Status(400, "Bad Request"),
        },
        "/api/profile" => {
            if let Some(v) = r.json::<OpName>() {
                match v.op.as_str() {
                    "save" => settings::save_profile(&v.name),
                    "apply" => settings::apply_profile(&v.name),
                    "delete" => settings::delete_profile(&v.name),
                    _ => {}
                }
                notify_subs();
            }
            Reply::NoContent
        }
        "/api/report" => match settings::make_report() {
            Ok(p) => {
                let pp = p.clone();
                util::run_on_ui(move || settings::show_in_explorer(&pp));
                http::json(&json!({ "path": p }))
            }
            Err(e) => http::json(&json!({ "err": e })),
        },
        "/api/reset" => {
            settings::reset();
            Reply::NoContent
        }
        "/icon" => match crate::appinfo::icon_png(r.query("exe")) {
            Some(b) => Reply::Body { ctype: "image/png", cache: "max-age=3600", body: b },
            None => Reply::Status(404, "Not Found"),
        },
        _ => Reply::Status(404, "Not Found"),
    }
}

/// Server-sent events: the state now, then after every change (not while the window is hidden —
/// it gets the state when shown), and a ping every 20 s.
fn events(s: &mut TcpStream) {
    let (tx, rx) = mpsc::sync_channel::<()>(1);
    SUBS.lock().unwrap().push(tx);
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\r\n";
    let send = |s: &mut TcpStream| -> bool {
        let b = serde_json::to_string(&state::build()).unwrap_or_default();
        s.write_all(format!("data: {b}\n\n").as_bytes()).and_then(|_| s.flush()).is_ok()
    };
    if s.write_all(head.as_bytes()).is_err() || !send(s) {
        return;
    }
    loop {
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(()) => {
                if fly::HIDDEN.load(Ordering::Relaxed) {
                    continue;
                }
                if !send(s) {
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if s.write_all(b": ping\n\n").and_then(|_| s.flush()).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

pub fn set_app_cat(exe: &str, cat: &str) {
    let e = crate::appinfo::canonical(&exe.trim().to_lowercase()).to_string();
    if e.is_empty() {
        return;
    }
    config::set(|c| {
        for l in [&mut c.chat, &mut c.media, &mut c.game, &mut c.pinned] {
            l.retain(|x| *x != e);
        }
        c.pinned.push(e.clone());
        match cat {
            "chat" => c.chat.push(e),
            "media" => c.media.push(e),
            _ => c.game.push(e),
        }
    });
}

fn set_eq(mut v: EqReq) {
    if v.bands.len() < 3 || v.bands.len() > 12 {
        return;
    }
    for b in v.bands.iter_mut() {
        *b = (((*b * 2.0 + 1000.5) as i64 - 1000) as f64 / 2.0).clamp(-12.0, 12.0);
    }
    if matches!(v.target.as_str(), "game" | "chat" | "media") {
        if v.bands.len() == 10 {
            config::set(|c| {
                let se = c.soft_eq.entry(v.target.clone()).or_default();
                se.bands = v.bands.clone();
                se.preset = v.preset.clone();
            });
            std::thread::spawn(|| crate::router::sync(&config::get()));
            notify_subs();
        }
        return;
    }
    // "custom" — a changed curve not saved as a preset: it is the curve itself (c.eq)
    config::set(|c| {
        c.eq = v.bands.clone();
        c.eq_preset = v.preset.clone();
    });
    crate::devices::set_eq(&v.bands);
    notify_subs();
}

/// The user's equalizer presets: save (a new one, or overwrite in place), rename, delete.
fn eq_user(v: EqUserReq) {
    let clean = |s: &str| -> String { s.trim().chars().take(24).collect() };
    let (name, to) = (clean(&v.name), clean(&v.to));
    let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
    config::set(|c: &mut Config| match v.op.as_str() {
        "save" => {
            if name.is_empty() || v.bands.len() < 3 || v.bands.len() > 12 || v.bands.len() != v.freqs.len() {
                return;
            }
            let p = EqUserPreset { name: name.clone(), bands: v.bands.iter().map(|b| b.clamp(-12.0, 12.0)).collect(), freqs: v.freqs.clone(), icon: v.icon.clone() };
            match c.eq_user.iter().position(|x| same(&x.name, &name)) {
                Some(i) => c.eq_user[i] = p,
                None if c.eq_user.len() < 24 => c.eq_user.push(p),
                None => {}
            }
        }
        "rename" => {
            if to.is_empty() || (!same(&name, &to) && c.eq_user.iter().any(|x| same(&x.name, &to))) {
                return;
            }
            let Some(x) = c.eq_user.iter_mut().find(|x| same(&x.name, &name)) else { return };
            x.name = to.clone();
            // the chosen preset follows its new name (the headset and the category equalizers)
            let follow = |p: &mut String| {
                if p.strip_prefix("u:").is_some_and(|n| same(n, &name)) {
                    *p = format!("u:{to}");
                }
            };
            follow(&mut c.eq_preset);
            c.soft_eq.values_mut().for_each(|se| follow(&mut se.preset));
        }
        "delete" => c.eq_user.retain(|x| !same(&x.name, &name)),
        _ => {}
    });
}

#[derive(serde::Serialize)]
struct ProcJson {
    exe: String,
    name: String,
    listed: bool,
}

/// Open apps with windows and running apps of the groups (for "+" in an app group).
fn list_procs() -> Vec<ProcJson> {
    let own = std::env::current_exe().ok().map(|p| util::base_name(&p.display().to_string()).to_lowercase()).unwrap_or_default();
    let listed: Vec<String> = state::build_apps().into_iter().map(|a| a.exe.to_lowercase()).collect();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for w in crate::appinfo::top_windows() {
        if w.title.is_empty() || !crate::appinfo::is_visible(w.hwnd) {
            continue;
        }
        let ex = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(w.hwnd, windows::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE) };
        if ex & 0x80 != 0 && ex & 0x40000 == 0 {
            continue; // a tool window without WS_EX_APPWINDOW
        }
        let full = util::exe_path_of(w.pid);
        if full.is_empty() {
            continue;
        }
        if full.to_lowercase().contains(r"\windows\") {
            continue;
        }
        let name = crate::appinfo::app_of(&full);
        if seen.contains(&name) || name == own {
            continue;
        }
        seen.insert(name.clone());
        let exe = if name.eq_ignore_ascii_case(util::base_name(&full)) { util::base_name(&full).to_string() } else { name.clone() };
        out.push(ProcJson { exe, name: crate::appinfo::display_name(&name), listed: listed.contains(&name) });
    }
    // apps of the groups that run without a window (in the tray, as Discord does after its start)
    let c = config::get();
    let procs = crate::appinfo::running_procs();
    for e in c.chat.iter().chain(&c.media).chain(&c.game) {
        let Some(&pid) = procs.get(e) else { continue };
        if seen.contains(e) {
            continue;
        }
        let full = util::exe_path_of(pid);
        let name = if full.is_empty() { e.clone() } else { crate::appinfo::app_of(&full) };
        if !seen.insert(name.clone()) {
            continue;
        }
        let exe = if !full.is_empty() && name.eq_ignore_ascii_case(util::base_name(&full)) { util::base_name(&full).to_string() } else { name.clone() };
        out.push(ProcJson { exe, name: crate::appinfo::display_name(&name), listed: listed.contains(&name) });
    }
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}
