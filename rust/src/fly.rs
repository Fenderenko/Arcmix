//! The app window: a borderless window with an embedded WebView2 (the Edge engine in Windows
//! 10/11). Rounded corners and the shadow are drawn by Windows (DWM). Always opaque.

use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, AtomicI64, Ordering},
    time::{Duration, Instant},
};

use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_COLOR, COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC, CreateCoreWebView2EnvironmentWithOptions, GetAvailableCoreWebView2BrowserVersionString, ICoreWebView2,
        ICoreWebView2Controller, ICoreWebView2Controller2, ICoreWebView2Settings3, ICoreWebView2Settings5, ICoreWebView2Settings6,
    },
    NavigationCompletedEventHandler,
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute},
            Gdi::{CreateSolidBrush, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
        },
        System::{Com::CoTaskMemFree, LibraryLoader::GetModuleHandleW},
        UI::{
            Controls::MARGINS,
            WindowsAndMessaging::{
                CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_EXSTYLE, GetClassNameW, GetClientRect, GetCursorPos, GetForegroundWindow, GetSystemMetrics,
                GetWindowLongPtrW, HWND_TOPMOST, ICON_BIG, ICON_SMALL, IDC_ARROW, IMAGE_ICON, IsIconic, KillTimer, LR_SHARED, LoadCursorW, LoadImageW, RegisterClassExW, SM_CXICON, SM_CXSMICON,
                SW_HIDE, SW_RESTORE, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SendMessageW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos,
                ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE, WM_CLOSE, WM_DISPLAYCHANGE, WM_ERASEBKGND, WM_MOVE, WM_NCCALCSIZE, WM_NCHITTEST, WM_SETICON, WM_SIZE, WM_SYSCOMMAND,
                WNDCLASSEXW, WS_CAPTION, WS_CLIPCHILDREN, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_MINIMIZEBOX, WS_POPUP, WS_THICKFRAME,
            },
        },
    },
    core::{BOOL, HSTRING, Interface, PCWSTR, PWSTR, w},
};

use crate::{
    APP_NAME, audio,
    config::{self, Config},
    i18n::t,
    logf, server, tray, util, wintheme,
};

pub const TIMER_IDLE: usize = 5;
pub const TIMER_WATCH: usize = 9;
/// focus went to the taskbar — hide a bit later (see deactivated)
pub const TIMER_HIDE: usize = 12;

const FLY_W: f64 = 400.0; // DIP
const FLY_H: f64 = 720.0;

#[derive(PartialEq, Clone, Copy)]
enum St {
    None,
    Creating,
    Ready,
}

struct Fly {
    hwnd: HWND,
    ctl: Option<ICoreWebView2Controller>,
    wv: Option<ICoreWebView2>,
    st: St,
    visible: bool,
    hidden_at: Option<Instant>,
    shown_at: Instant,
    recreated: Option<Instant>,
    class_ok: bool,
}

thread_local! {
    static FLY: RefCell<Fly> = RefCell::new(Fly {
        hwnd: HWND::default(), ctl: None, wv: None, st: St::None, visible: false, hidden_at: None,
        shown_at: Instant::now(), recreated: None, class_ok: false,
    });
}

fn fly<R>(f: impl FnOnce(&mut Fly) -> R) -> R {
    FLY.with(|x| f(&mut x.borrow_mut()))
}

/// The window exists but is hidden: the page gets no updates then.
pub static HIDDEN: AtomicBool = AtomicBool::new(false);
/// The window is shown (audio work that is visible only there runs meanwhile).
pub static SHOWN: AtomicBool = AtomicBool::new(false);
/// When the page answered last (UnixNano) and its size in physical pixels (w<<32 | h).
pub static ALIVE: AtomicI64 = AtomicI64::new(0);
pub static ALIVE_SIZE: AtomicI64 = AtomicI64::new(0);

fn now_nanos() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as i64).unwrap_or(0)
}

pub fn mark_alive(w: f64, h: f64) {
    ALIVE_SIZE.store(((w.round() as i64) << 32) | (h.round() as i64 & 0xffff_ffff), Ordering::Relaxed);
    ALIVE.store(now_nanos(), Ordering::Relaxed);
}

pub fn visible() -> bool {
    fly(|f| f.visible)
}

pub fn toggle() {
    if visible() {
        hide();
        return;
    }
    if fly(|f| f.hidden_at.is_some_and(|t| t.elapsed() < Duration::from_millis(400))) {
        return; // the icon click itself hid the window just now (focus loss)
    }
    open();
}

pub fn open() {
    if server::port() == 0 || fly(|f| f.st == St::Creating) {
        return;
    }
    if fly(|f| f.st == St::None) && !create() {
        return;
    }
    show();
}

/// Light window theme (with "as in Windows").
pub fn is_light(c: &Config) -> bool {
    if c.theme == "black" {
        return false; // OLED is dark by definition
    }
    if c.theme == "win" {
        return wintheme::light();
    }
    c.mode == "light" || (c.mode == "auto" && wintheme::system_light())
}

fn theme_bg() -> (u8, u8, u8) {
    let c = config::get();
    let oled = c.theme == "black" || (c.theme == "custom" && c.custom_oled);
    if oled && !is_light(&c) {
        return (0, 0, 0);
    }
    match (c.theme == "win", is_light(&c)) {
        (true, true) => (0xF3, 0xF3, 0xF3),
        (true, false) => (0x20, 0x20, 0x20),
        (false, true) => (0xEE, 0xEF, 0xF3),
        (false, false) => (0x14, 0x13, 0x16),
    }
}

fn register_class() -> bool {
    if fly(|f| f.class_ok) {
        return true;
    }
    let (r, g, b) = theme_bg();
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: CreateSolidBrush(windows::Win32::Foundation::COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)),
            lpszClassName: w!("ArcmixFly"),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            logf!("RegisterClassEx(fly) failed");
            return false;
        }
    }
    fly(|f| f.class_ok = true);
    true
}

pub fn dpi_scale() -> f64 {
    let d = unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() };
    if d < 96 { 1.0 } else { d as f64 / 96.0 }
}

fn set_dark_frame(hwnd: HWND) {
    let dark: BOOL = (!is_light(&config::get())).into();
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const _, 4);
    }
}

fn webview_version() -> Option<String> {
    unsafe {
        let mut p = PWSTR::null();
        GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut p).ok()?;
        if p.is_null() {
            return None;
        }
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.filter(|s| !s.is_empty())
    }
}

fn create() -> bool {
    let Some(v) = webview_version() else {
        logf!("WebView2 Runtime not found, opening in browser");
        tray::balloon(&t("Потрібен WebView2"), &t("Встановіть Microsoft Edge WebView2 Runtime. Поки що відкриваю у браузері."), false);
        util::shell_open(&server::url());
        return false;
    };
    logf!("WebView2 {v}");
    if !register_class() {
        return false;
    }
    let sc = dpi_scale();
    let (x, y, w, h) = target_rect((FLY_W * sc) as i32, (FLY_H * sc) as i32);
    // the frame stays for Windows' shadow and rounding, but is not visible (WM_NCCALCSIZE);
    // without WS_SYSMENU Windows draws no title buttons; MINIMIZEBOX — a click on the taskbar
    // button minimizes the window (then it is hidden)
    let style = WS_POPUP | WS_THICKFRAME | WS_CAPTION | WS_CLIPCHILDREN | WS_MINIMIZEBOX;
    let hwnd = unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        CreateWindowExW(ex_style(), w!("ArcmixFly"), &HSTRING::from(APP_NAME), WINDOW_STYLE(style.0), x, y, w, h, None, None, Some(inst.into()), None)
    };
    let Ok(hwnd) = hwnd else {
        logf!("CreateWindowEx(fly) failed");
        return false;
    };
    fly(|f| {
        f.hwnd = hwnd;
        f.st = St::Creating;
    });
    set_icons(hwnd);
    unsafe {
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as *const _, 4);
        set_dark_frame(hwnd);
        let m = MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &m);
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER);
        // the window is shown before WebView2 is created — as in Microsoft's samples
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(hwnd);
    }
    fly(|f| f.visible = true);
    SHOWN.store(true, Ordering::Relaxed);
    HIDDEN.store(false, Ordering::Relaxed);

    let data = HSTRING::from(util::data_dir().join("webview").as_os_str());
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |res, env| {
        let Some(env) = env.filter(|_| res.is_ok()) else {
            fail("environment", res.err());
            return Ok(());
        };
        let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |res, ctl| {
            match ctl.filter(|_| res.is_ok()) {
                Some(ctl) => util::safely("webview", || ready(hwnd, ctl)),
                None => {
                    fail("controller", res.err());
                    true
                }
            };
            Ok(())
        }));
        if let Err(e) = unsafe { env.CreateCoreWebView2Controller(hwnd, &handler) } {
            fail("controller", Some(e));
        }
        Ok(())
    }));
    if let Err(e) = unsafe { CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &data, None, &handler) } {
        fail("environment", Some(e));
        return false;
    }
    true
}

/// WebView2 could not start: the window is closed and the page opens in the browser.
fn fail(what: &str, e: Option<windows::core::Error>) {
    logf!("failed to create WebView2 ({what}): {e:?}");
    let hwnd = fly(|f| std::mem::take(&mut f.hwnd));
    if !hwnd.is_invalid() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
    fly(|f| {
        f.st = St::None;
        f.visible = false;
        f.ctl = None;
        f.wv = None;
    });
    SHOWN.store(false, Ordering::Relaxed);
    util::shell_open(&server::url());
}

fn ready(hwnd: HWND, ctl: ICoreWebView2Controller) {
    let Ok(wv) = (unsafe { ctl.CoreWebView2() }) else {
        fail("webview", None);
        return;
    };
    unsafe {
        if let Ok(st) = wv.Settings() {
            let _ = st.SetAreDefaultContextMenusEnabled(false);
            let _ = st.SetAreDevToolsEnabled(false);
            let _ = st.SetIsZoomControlEnabled(false);
            let _ = st.SetIsStatusBarEnabled(false);
            if let Ok(s3) = st.cast::<ICoreWebView2Settings3>() {
                let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false);
            }
            if let Ok(s5) = st.cast::<ICoreWebView2Settings5>() {
                let _ = s5.SetIsPinchZoomEnabled(false);
            }
            if let Ok(s6) = st.cast::<ICoreWebView2Settings6>() {
                let _ = s6.SetIsSwipeNavigationEnabled(false);
            }
        }
        let mut token = Default::default();
        let _ = wv.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(|_, args| {
                if let Some(a) = args {
                    let mut ok = BOOL::default();
                    let _ = a.IsSuccess(&mut ok);
                    let mut code = Default::default();
                    let _ = a.WebErrorStatus(&mut code);
                    logf!("page loaded: ok={}, code={}", ok.as_bool(), code.0);
                }
                Ok(())
            })),
            &mut token,
        );
    }
    fly(|f| {
        f.ctl = Some(ctl.clone());
        f.wv = Some(wv.clone());
    });
    background();
    resize();
    unsafe {
        let _ = ctl.SetIsVisible(fly(|f| f.visible));
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let url = server::url();
        logf!("window created, client {}x{}, opening {}", rc.right - rc.left, rc.bottom - rc.top, url.split('?').next().unwrap_or(""));
        let _ = wv.Navigate(&HSTRING::from(url));
    }
    fly(|f| {
        f.st = St::Ready;
        f.shown_at = Instant::now();
    });
    unsafe {
        SetTimer(Some(util::main_hwnd()), TIMER_WATCH, 4000, None); // the first load takes longer
    }
    if fly(|f| f.visible) {
        audio::set_activity(true);
        server::notify_subs();
    }
}

/// Without a taskbar button (like tray flyouts) or with one — so the app can be pinned there.
fn ex_style() -> WINDOW_EX_STYLE {
    if config::get().taskbar { WS_EX_APPWINDOW | WS_EX_TOPMOST } else { WS_EX_TOOLWINDOW | WS_EX_TOPMOST }
}

/// The app icon for the taskbar button and Alt+Tab (from the exe resources).
fn set_icons(hwnd: HWND) {
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        for (which, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let cx = GetSystemMetrics(metric);
            if let Ok(ic) = LoadImageW(Some(inst.into()), PCWSTR(1 as *const u16), IMAGE_ICON, cx, cx, LR_SHARED) {
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(which as usize)), Some(LPARAM(ic.0 as isize)));
            }
        }
    }
}

/// (UI thread) The taskbar button was switched in the settings. The taskbar notices the change
/// only of a hidden window, so an open one is hidden for a moment.
pub fn apply_style() {
    let hwnd = fly(|f| f.hwnd);
    if hwnd.is_invalid() {
        return;
    }
    let mask = (WS_EX_TOOLWINDOW | WS_EX_APPWINDOW).0 as isize;
    let cur = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let want = ex_style().0 as isize;
    if cur & mask == want & mask {
        return;
    }
    let vis = visible();
    if vis {
        hide();
    }
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (cur & !mask) | (want & mask));
    }
    if vis {
        show();
    }
}

/// (UI thread) The app was started again (e.g. the pinned taskbar button) — show the window.
pub fn on_show_req() {
    if visible() {
        show();
    } else {
        open();
    }
}

/// The window lost focus. If the taskbar took it (a click on the tray icon or on the window
/// button), hide later: the click decides — the tray icon hides the window, the taskbar button
/// minimizes it (SC_MINIMIZE). Otherwise a click on the button right after an instant hide started
/// the exe again and opened the window back.
fn deactivated(other: HWND) {
    let other = if other.is_invalid() { unsafe { GetForegroundWindow() } } else { other };
    let mut b = [0u16; 64];
    let n = unsafe { GetClassNameW(other, &mut b) };
    let class = String::from_utf16_lossy(&b[..n.max(0) as usize]);
    if class == "Shell_TrayWnd" || class == "Shell_SecondaryTrayWnd" {
        unsafe {
            SetTimer(Some(util::main_hwnd()), TIMER_HIDE, 350, None);
        }
        return;
    }
    hide();
}

/// The taskbar did not give the focus back — hide.
pub fn hide_tick() {
    unsafe {
        let _ = KillTimer(Some(util::main_hwnd()), TIMER_HIDE);
    }
    let (vis, hwnd) = fly(|f| (f.visible, f.hwnd));
    if vis && unsafe { GetForegroundWindow() } != hwnd && !config::get().keep_open {
        hide();
    }
}

pub fn background() {
    let (ctl, hwnd) = fly(|f| (f.ctl.clone(), f.hwnd));
    if let Some(c2) = ctl.and_then(|c| c.cast::<ICoreWebView2Controller2>().ok()) {
        let (r, g, b) = theme_bg();
        unsafe {
            let _ = c2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { A: 255, R: r, G: g, B: b });
        }
    }
    if !hwnd.is_invalid() {
        set_dark_frame(hwnd);
    }
}

fn resize() {
    let (ctl, hwnd) = fly(|f| (f.ctl.clone(), f.hwnd));
    if let Some(ctl) = ctl {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
            let _ = ctl.SetBounds(rc);
        }
    }
}

/// The lower right corner of the work area (above the taskbar) of the monitor with the icon.
fn target_rect(w: i32, h: i32) -> (i32, i32, i32, i32) {
    let sc = dpi_scale();
    let (cx, cy) = match tray::icon_rect().filter(|r| r.right > r.left) {
        Some(r) => ((r.left + r.right) / 2, (r.top + r.bottom) / 2),
        None => {
            let mut pt = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }
            (pt.x, pt.y)
        }
    };
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let mon = MonitorFromPoint(POINT { x: cx, y: cy }, MONITOR_DEFAULTTONEAREST);
        let _ = GetMonitorInfoW(mon, &mut mi);
    }
    let wk = mi.rcWork;
    let margin = (12.0 * sc) as i32;
    let h = h.min(wk.bottom - wk.top - 2 * margin);
    let x = wk.right - w - margin;
    let mut y = wk.bottom - h - margin;
    if cy < (wk.top + wk.bottom) / 2 && wk.top > mi.rcMonitor.top {
        y = wk.top + margin; // taskbar at the top
    }
    (x, y, w, h)
}

pub fn show() {
    let hwnd = fly(|f| f.hwnd);
    if hwnd.is_invalid() {
        return;
    }
    if wintheme::refresh() {
        // the Windows look could have changed while the window was hidden
        tray::update(true);
        background();
    }
    let sc = dpi_scale();
    unsafe {
        let _ = KillTimer(Some(util::main_hwnd()), TIMER_HIDE);
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE); // was minimized — back to normal
        }
        let (x, y, w, h) = target_rect((FLY_W * sc) as i32, (FLY_H * sc) as i32);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(hwnd);
    }
    if let Some(ctl) = fly(|f| f.ctl.clone()) {
        resize();
        unsafe {
            let _ = ctl.SetIsVisible(true);
            let _ = ctl.NotifyParentWindowPositionChanged();
            let _ = ctl.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
    }
    fly(|f| {
        f.visible = true;
        f.shown_at = Instant::now();
    });
    HIDDEN.store(false, Ordering::Relaxed);
    SHOWN.store(true, Ordering::Relaxed);
    audio::set_activity(true);
    unsafe {
        let _ = KillTimer(Some(util::main_hwnd()), TIMER_IDLE);
        SetTimer(Some(util::main_hwnd()), TIMER_WATCH, 2500, None);
    }
    server::notify_subs();
}

pub fn hide() {
    let (hwnd, vis, ctl) = fly(|f| (f.hwnd, f.visible, f.ctl.clone()));
    if hwnd.is_invalid() || !vis {
        return;
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
        if let Some(ctl) = ctl {
            let _ = ctl.SetIsVisible(false); // the page does not paint while hidden
        }
    }
    fly(|f| {
        f.visible = false;
        f.hidden_at = Some(Instant::now());
    });
    HIDDEN.store(true, Ordering::Relaxed);
    SHOWN.store(false, Ordering::Relaxed);
    crate::router::mic_peak_stop(); // the window mic meter is not visible — release the mic right away
    audio::set_activity(false);
    unsafe {
        let mh = util::main_hwnd();
        let _ = KillTimer(Some(mh), TIMER_HIDE);
        let _ = KillTimer(Some(mh), TIMER_WATCH);
        SetTimer(Some(mh), TIMER_IDLE, 10 * 60 * 1000, None);
    }
}

/// A hidden window is kept 10 minutes (opens instantly), then WebView2 is released completely.
pub fn idle_tick() {
    unsafe {
        let _ = KillTimer(Some(util::main_hwnd()), TIMER_IDLE);
    }
    if !visible() {
        close();
    }
}

/// The watchdog: after monitor changes, sleep or a video driver failure the embedded Edge can
/// stop painting the page (an empty window) or the page can crash. After every show the page
/// reports it painted (/api/alive). No answer in 2.5 s — WebView2 is created anew.
pub fn watch() {
    unsafe {
        let _ = KillTimer(Some(util::main_hwnd()), TIMER_WATCH);
    }
    let (vis, has, shown_at, hwnd, recreated) = fly(|f| (f.visible, f.ctl.is_some(), f.shown_at, f.hwnd, f.recreated));
    if !vis || !has {
        return;
    }
    let alive = ALIVE.load(Ordering::Relaxed);
    let shown_nanos = now_nanos() - shown_at.elapsed().as_nanos() as i64;
    if alive < shown_nanos {
        if recreated.is_some_and(|r| r.elapsed() < Duration::from_secs(30)) {
            logf!("webview: page still not responding");
            return;
        }
        fly(|f| f.recreated = Some(Instant::now()));
        logf!("webview: page did not respond after show — recreating the window");
        close();
        open();
        return;
    }
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let sz = ALIVE_SIZE.load(Ordering::Relaxed);
    let (w, h) = ((sz >> 32) as i32, (sz & 0xffff_ffff) as i32);
    let (dw, dh) = (w - (rc.right - rc.left), h - (rc.bottom - rc.top));
    if dw.abs() > 4 || dh.abs() > 4 {
        logf!("webview: page size {w}x{h}, window {}x{} — fixing", rc.right - rc.left, rc.bottom - rc.top);
        resize();
        if let Some(ctl) = fly(|f| f.ctl.clone()) {
            unsafe {
                let _ = ctl.NotifyParentWindowPositionChanged();
            }
        }
    }
}

pub fn close() {
    let (ctl, hwnd) = fly(|f| {
        f.wv = None;
        (f.ctl.take(), std::mem::take(&mut f.hwnd))
    });
    if let Some(ctl) = ctl {
        unsafe {
            let _ = ctl.Close();
        }
    }
    if !hwnd.is_invalid() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
    let was = fly(|f| {
        let was = f.visible;
        f.st = St::None;
        f.visible = false;
        was
    });
    HIDDEN.store(false, Ordering::Relaxed);
    SHOWN.store(false, Ordering::Relaxed);
    if was {
        audio::set_activity(false);
    }
}

/// (UI thread) After a display change a hidden window is closed — it is created anew for the new
/// monitors next time; a shown one is placed again.
pub fn reset(hidden: bool) {
    if fly(|f| f.hwnd.is_invalid()) {
        return;
    }
    if hidden {
        if !visible() {
            logf!("webview: displays changed — window will be recreated on next open");
            close();
        }
        return;
    }
    show();
}

/// The Edge profile of a version that used msedge --app.
pub fn cleanup_old() {
    let _ = std::fs::remove_dir_all(util::data_dir().join("edge"));
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let r = util::safely_value("fly window", || {
        let me = fly(|f| f.hwnd);
        match m {
            WM_NCCALCSIZE if wp.0 != 0 => return Some(LRESULT(0)), // all of it is client area
            WM_NCHITTEST => return Some(LRESULT(1)),               // HTCLIENT: not movable or resizable
            WM_SIZE => {
                if h == me {
                    if wp.0 == 1 {
                        // SIZE_MINIMIZED (taskbar, Win+Down) — hide instead
                        hide();
                        return Some(LRESULT(0));
                    }
                    resize();
                }
                return Some(LRESULT(0));
            }
            WM_MOVE => {
                if let Some(ctl) = fly(|f| f.ctl.clone()) {
                    unsafe {
                        let _ = ctl.NotifyParentWindowPositionChanged();
                    }
                }
            }
            WM_ACTIVATE => {
                // lost focus (and the window is not pinned)
                if wp.0 & 0xffff == 0 && h == me && !config::get().keep_open {
                    deactivated(HWND(lp.0 as *mut _));
                }
            }
            WM_DISPLAYCHANGE => {
                if h == me {
                    util::post_ui(crate::WM_FLY_RESET, if visible() { 0 } else { 1 });
                }
            }
            WM_CLOSE => {
                hide();
                return Some(LRESULT(0));
            }
            WM_SYSCOMMAND => {
                // a click on the taskbar button minimizes the window — hide it
                if wp.0 & 0xFFF0 == 0xF020 {
                    hide();
                    return Some(LRESULT(0));
                }
            }
            WM_ERASEBKGND => {} // the class brush paints the background
            _ => {}
        }
        None
    })
    .flatten();
    match r {
        Some(v) => v,
        None => unsafe { DefWindowProcW(h, m, wp, lp) },
    }
}
