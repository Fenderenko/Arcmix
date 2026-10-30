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

/// Release builds carry the page, its stylesheet and script; debug builds read them from web/ on
/// every request, so a change needs only a reload of the window, not a rebuild.
macro_rules! web_file {
    ($name:ident, $file:literal) => {
        #[cfg(not(debug_assertions))]
        fn $name() -> std::borrow::Cow<'static, str> {
            include_str!(concat!("../../web/", $file)).into()
        }
        #[cfg(debug_assertions)]
        fn $name() -> std::borrow::Cow<'static, str> {
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/", $file)).unwrap_or_default().into()
        }
    };
}
web_file!(index_html, "index.html");
web_file!(app_css, "app.css");
web_file!(app_js, "app.js");
pub(crate) const FONT_TTF: &[u8] = include_bytes!("../../web/onest.ttf");

/// Links the page may open in the browser.
pub const OPEN_URLS: [&str; 7] = [
    "https://vb-audio.com/Cable/",
    "https://shop.vb-audio.com/en/audio-apps/12-vb-cable-ab.html",
    "https://github.com/Swiftlyx/OpenChatMix",
    "https://github.com/Fenderenko/Arcmix",
    crate::update::RELEASES_PAGE,
    // NVIDIA's redistributables of the Audio Effects SDK (one per RTX generation)
    "https://www.nvidia.com/en-us/geforce/broadcasting/broadcast-sdk/resources/",
    crate::apo::DOWNLOAD_PAGE,
];

/// The newer release found by the last update check ("Update" installs it).
static LATEST: Mutex<Option<crate::update::Latest>> = Mutex::new(None);
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
    // static files the page links without the token (no secrets in them): only this host
    if matches!(r.path.as_str(), "/font.ttf" | "/app.css" | "/app.js") {
        if r.header("Host") != Some(host.as_str()) {
            return Reply::Status(403, "Forbidden");
        }
        return match r.path.as_str() {
            "/app.css" => Reply::Body { ctype: "text/css; charset=utf-8", cache: "no-store", body: app_css().into_owned().into_bytes() },
            "/app.js" => Reply::Body { ctype: "text/javascript; charset=utf-8", cache: "no-store", body: app_js().into_owned().into_bytes() },
            _ => Reply::Body { ctype: "font/ttf", cache: "max-age=86400", body: FONT_TTF.to_vec() },
        };
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
    /// profiles: the app the profile is bound to ("" — none)
    app: String,
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
    #[serde(rename = "denoiseKind")]
    denoise_kind: Option<String>,
    #[serde(rename = "nvMode")]
    nv_mode: Option<String>,
    #[serde(rename = "nvStrength")]
    nv_strength: Option<i32>,
    comp: Option<bool>,
    #[serde(rename = "compDb")]
    comp_db: Option<f64>,
    #[serde(rename = "compRatio")]
    comp_ratio: Option<f64>,
    #[serde(rename = "compGain")]
    comp_gain: Option<f64>,
    order: Option<Vec<String>>,
    sleep: Option<bool>,
}

fn route(r: &Request, s: &mut TcpStream) -> Reply {
    match r.path.as_str() {
        "/" => {
            logf!("page requested by {}", r.header("User-Agent").unwrap_or(""));
            let page = index_html().replacen(r#"/*I18N*/{"lang":"uk","t":{},"langs":[]}"#, &i18n::payload(&config::get().lang), 1);
            Reply::Body { ctype: "text/html; charset=utf-8", cache: "no-store", body: page.into_bytes() }
        }
        "/api/state" => http::json(&state::build()),
        "/api/events" => {
            events(s);
            Reply::Taken
        }
        "/api/meters" => {
            meters(s, r.query("m"));
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
                remove_app(&v.exe);
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
        "/api/apo" => {
            #[derive(Deserialize, Default)]
            #[serde(default)]
            struct V {
                op: String,
            }
            // both wait for a UAC prompt and an elevated program: off the server thread
            match r.json::<V>().map(|v| v.op).as_deref() {
                Some("setup") => {
                    std::thread::spawn(|| {
                        crate::apo::setup();
                        notify_subs();
                    });
                }
                Some("modInstallAll") => {
                    std::thread::spawn(crate::apomod::install_all);
                }
                Some(op @ ("modInstall" | "modRemove")) => {
                    let id = crate::audio::out_state().id;
                    let op = if op == "modInstall" { "install" } else { "remove" };
                    std::thread::spawn(move || crate::apomod::run(op, id));
                }
                Some("devices") => {
                    std::thread::spawn(|| {
                        crate::apo::choose_devices();
                        std::thread::spawn(|| crate::router::sync(&config::get()));
                        notify_subs();
                    });
                }
                _ => {}
            }
            Reply::NoContent
        }
        "/api/autoeq" => {
            #[derive(Deserialize, Default)]
            #[serde(default)]
            struct V {
                /// search
                q: Option<String>,
                /// the output device to set
                dev: String,
                path: String,
                name: String,
                src: String,
                on: Option<bool>,
                remove: bool,
            }
            let Some(v) = r.json::<V>() else { return Reply::NoContent };
            if let Some(q) = v.q {
                return match crate::apo::search(&q) {
                    Ok(l) => http::json(&json!({ "list": l })),
                    Err(e) => http::json(&json!({ "err": e })),
                };
            }
            if v.dev.is_empty() {
                return Reply::NoContent;
            }
            if v.remove {
                config::set(|c| {
                    c.auto_eq.remove(&v.dev);
                });
            } else if let Some(on) = v.on {
                config::set(|c| {
                    if let Some(a) = c.auto_eq.get_mut(&v.dev) {
                        a.on = on;
                    }
                });
            } else if !v.path.is_empty() {
                match crate::apo::fetch_curve(&v.path) {
                    Ok(pts) => config::set(|c| {
                        c.auto_eq.insert(v.dev.clone(), config::AutoEq { name: v.name.chars().take(80).collect(), src: v.src.chars().take(80).collect(), pts, on: true });
                    }),
                    Err(e) => {
                        logf!("autoeq: {e}");
                        return http::json(&json!({ "err": e }));
                    }
                }
            }
            std::thread::spawn(|| crate::router::sync(&config::get()));
            notify_subs();
            http::json(&json!({ "ok": true }))
        }
        "/api/outvol" => {
            #[derive(Deserialize, Default)]
            #[serde(default)]
            struct V {
                vol: Option<i32>,
                mute: Option<bool>,
            }
            if let Some(v) = r.json::<V>() {
                audio::set_out_volume(v.vol, v.mute);
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
                    if d.is_empty() || !config::get().win_ctl {
                        // without "control Windows sound" the choice is the app's own
                        config::set(|c| c.mic_dev = d);
                        audio::refresh_mic();
                        std::thread::spawn(|| crate::router::sync(&config::get())); // processing and "listen to yourself" take this microphone
                        notify_subs();
                    } else {
                        audio::set_input(&d); // Windows' default microphone too
                    }
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
            let (render, capture) = audio::list_devices_with_off();
            http::json(&json!({ "render": render, "capture": capture }))
        }
        "/api/route" => {
            if let Some(v) = r.json::<RouteReq>() {
                for x in [&v.game, &v.chat, &v.media].into_iter().flatten() {
                    audio::enable_cable(x, false);
                }
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
                    if let Some(x) = v.out.clone() {
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
                    if let Some(k) = v.denoise_kind.filter(|k| k == "rnn" || k == "nv") {
                        m.denoise_kind = k;
                    }
                    if let Some(k) = v.nv_mode.filter(|k| crate::nvafx::mode_index(k).is_some()) {
                        m.nv_mode = k;
                    }
                    if let Some(x) = v.nv_strength {
                        m.nv_strength = x.clamp(0, 100);
                    }
                    if let Some(x) = v.comp {
                        m.comp = x;
                    }
                    if let Some(x) = v.comp_db {
                        m.comp_db = x.clamp(-60.0, 0.0);
                    }
                    if let Some(x) = v.comp_ratio {
                        m.comp_ratio = x.clamp(1.5, 20.0);
                    }
                    if let Some(x) = v.comp_gain {
                        m.comp_gain = x.clamp(0.0, 24.0);
                    }
                    if let Some(x) = v.order {
                        m.order = x.into_iter().filter(|n| crate::dsp::MIC_STAGES.contains(&n.as_str())).collect();
                    }
                    if let Some(x) = v.sleep {
                        m.sleep = x;
                    }
                });
                if let Some(x) = &v.out {
                    audio::enable_cable(x, true);
                }
                std::thread::spawn(|| crate::router::sync(&config::get()));
                notify_subs();
            }
            Reply::NoContent
        }
        "/api/winducking" => {
            util::run_on_ui(|| settings::open_win_sound(3));
            Reply::NoContent
        }
        "/api/winsound" => {
            util::run_on_ui(|| settings::open_win_sound(0));
            Reply::NoContent
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
                    // the settings folder: next to Arcmix.exe or in %APPDATA%
                    "portable" | "appdata" => {
                        let r = config::set_portable(v.op == "portable");
                        notify_subs();
                        return match r {
                            Ok(()) => http::json(&json!({ "ok": true })),
                            Err(e) => {
                                logf!("settings folder not changed: {e}");
                                http::json(&json!({ "err": e }))
                            }
                        };
                    }
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
            let ver = latest.as_ref().map(|l| l.ver.clone());
            let newer = ver.as_deref().is_some_and(|v| crate::version::newer(v, crate::APP_VER));
            let exe = newer && latest.as_ref().is_some_and(|l| l.exe.is_some());
            logf!("update check: latest {}, newer {newer}, exe {exe}", ver.as_deref().unwrap_or("-"));
            *LATEST.lock().unwrap() = latest.filter(|_| newer);
            http::json(&json!({ "latest": ver, "newer": newer, "exe": exe }))
        }
        "/api/update/install" => {
            // the release found by the last check: downloaded, put in place, the app restarts
            let Some(l) = LATEST.lock().unwrap().clone() else { return http::json(&json!({ "err": "no update found" })) };
            match crate::update::install(&l) {
                Ok(()) => {
                    // after the answer is sent: the new copy waits for this one to quit
                    std::thread::spawn(|| {
                        std::thread::sleep(std::time::Duration::from_millis(400));
                        util::post_ui(crate::WM_QUIT_REQ, 0);
                    });
                    http::json(&json!({ "ok": true }))
                }
                Err(e) => {
                    logf!("update failed: {e}");
                    http::json(&json!({ "err": e }))
                }
            }
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
                    "save" => settings::save_profile(&v.name, &v.app),
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

/// A level meter as a stream (SSE) while the page shows it, ~30 values a second: one connection
/// instead of a request per value, and the page draws it smoothly. "mic" — the microphone peak,
/// "fx" — the levels of the microphone processing (the peak while it does not run), "duck" — the
/// chat sound that ducks the game. Ends when the window hides or the page closes it.
fn meters(s: &mut TcpStream, kind: &str) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\r\n";
    if !matches!(kind, "mic" | "fx" | "duck") || s.write_all(head.as_bytes()).is_err() {
        return;
    }
    let r1 = |v: f64| (v * 10.0).round() / 10.0;
    // the processing levels are shown: it does not sleep meanwhile
    let _watch = (kind == "fx").then(crate::router::FxWatch::new);
    loop {
        if fly::HIDDEN.load(Ordering::Relaxed) {
            return;
        }
        let v = if kind == "duck" {
            serde_json::to_value(audio::duck_meter()).unwrap_or_default()
        } else if kind == "fx" && crate::router::mic_running() {
            let (i, o, open, ok) = crate::router::mic_meter();
            json!({ "in": r1(i), "out": r1(o), "open": open, "ok": ok, "run": true })
        } else {
            let (db, ok) = crate::router::mic_peak_db();
            json!({ "db": r1(db), "ok": ok })
        };
        if s.write_all(format!("data: {v}\n\n").as_bytes()).and_then(|_| s.flush()).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(33));
    }
}

pub fn set_app_cat(exe: &str, cat: &str) {
    let e = crate::appinfo::canonical(&exe.trim().to_lowercase()).to_string();
    if e.is_empty() {
        return;
    }
    config::set(|c| {
        for l in [&mut c.chat, &mut c.media, &mut c.game, &mut c.pinned, &mut c.hidden] {
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

/// Removes an app from the groups altogether: its group, its own output device and its remembered
/// volume are forgotten, and it stays out of the lists (even while it plays) until added again with "+".
/// Its sound meanwhile counts as a game, as any app outside the lists.
fn remove_app(exe: &str) {
    let e = crate::appinfo::canonical(&exe.trim().to_lowercase()).to_string();
    if e.is_empty() {
        return;
    }
    let had_out = config::get().app_outs.contains_key(&e);
    config::set(|c| {
        for l in [&mut c.chat, &mut c.media, &mut c.game, &mut c.pinned] {
            l.retain(|x| *x != e);
        }
        c.app_outs.remove(&e);
        c.app_vols.remove(&e);
        if !c.hidden.contains(&e) {
            c.hidden.push(e.clone());
        }
    });
    if had_out {
        audio::reapply(); // back to "As in Windows", as choosing it in the menu does
    }
}

fn set_eq(mut v: EqReq) {
    if v.bands.len() < 3 || v.bands.len() > 12 {
        return;
    }
    for b in v.bands.iter_mut() {
        *b = (((*b * 2.0 + 1000.5) as i64 - 1000) as f64 / 2.0).clamp(-12.0, 12.0);
    }
    if matches!(v.target.as_str(), "all" | "game" | "chat" | "media") {
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
    // apps of the groups that run without a window (in the tray, as Discord does after its start), and
    // removed ones, so that "+" can bring them back
    let c = config::get();
    let procs = crate::appinfo::running_procs();
    for e in c.chat.iter().chain(&c.media).chain(&c.game).chain(&c.hidden) {
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
