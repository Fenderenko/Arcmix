//! Apps: readable names from exe metadata, icons as PNG for the window, top-level windows and
//! running processes.

use std::{
    collections::HashMap,
    ffi::c_void,
    sync::{Mutex, mpsc},
    time::Duration,
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, TRUE},
        Graphics::Gdi::{BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ, ReleaseDC},
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
            Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS},
        },
        UI::{
            Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW},
            WindowsAndMessaging::{DestroyIcon, EnumWindows, GetClassNameW, GetIconInfo, GetWindowTextW, GetWindowThreadProcessId, HICON, ICONINFO, IsWindowVisible, PrivateExtractIconsW},
        },
    },
    core::{BOOL, HSTRING, PCWSTR},
};

use crate::util::{self, from_wide};

#[derive(Default)]
struct Paths {
    /// exe (lower case) → full path
    exe: HashMap<String, String>,
    desc: HashMap<String, String>,
    icon: HashMap<String, Option<Vec<u8>>>,
}

static PATHS: Mutex<Option<Paths>> = Mutex::new(None);

/// The paths of apps seen before (exe → path), kept in the data folder: a closed app keeps its name
/// and icon after a restart. None outside the app (tests).
fn store() -> Option<std::path::PathBuf> {
    let d = util::data_dir();
    (d != std::path::Path::new(".")).then(|| d.join("apppaths.json"))
}

fn with<R>(f: impl FnOnce(&mut Paths) -> R) -> R {
    let mut g = PATHS.lock().unwrap();
    f(g.get_or_insert_with(|| {
        let mut p = Paths::default();
        let saved = store().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str::<HashMap<String, String>>(&t).ok());
        p.exe = saved.unwrap_or_default().into_iter().filter(|(_, v)| std::path::Path::new(v).is_file()).collect();
        p
    }))
}

pub fn remember_path(exe: &str, path: &str) {
    if exe.is_empty() || path.is_empty() {
        return;
    }
    let key = exe.to_lowercase();
    // written only when something new is learned
    let all = with(|p| (p.exe.insert(key, path.to_string()).as_deref() != Some(path)).then(|| p.exe.clone()));
    if let (Some(all), Some(f)) = (all, store()) {
        let _ = std::fs::write(f, serde_json::to_string(&all).unwrap_or_default());
    }
}

pub fn path_for(exe: &str) -> String {
    with(|p| p.exe.get(&exe.to_lowercase()).cloned().unwrap_or_default())
}

/// Helper processes listed and handled as their app: (helper, app), exe names in lower case.
/// steamwebhelper.exe plays Steam's voice chat and store sounds.
pub const ALIASES: [(&str, &str); 1] = [("steamwebhelper.exe", "steam.exe")];

/// The app an exe (lower case) belongs to.
pub fn canonical(exe: &str) -> &str {
    ALIASES.iter().find(|a| a.0 == exe).map_or(exe, |a| a.1)
}

/// The app (lower case) of a process by its exe path; remembers the app's path for its name and icon.
/// A helper counts as its app, whose exe is looked up in the folders above the helper.
pub fn app_of(full: &str) -> String {
    let own = util::base_name(full).to_lowercase();
    let app = canonical(&own);
    if app == own {
        remember_path(app, full);
    } else if path_for(app).is_empty() {
        let mut dir = std::path::Path::new(full).parent();
        while let Some(d) = dir {
            let p = d.join(app);
            if p.is_file() {
                remember_path(app, &p.display().to_string());
                break;
            }
            dir = d.parent();
        }
    }
    app.to_string()
}

/// A string from exe metadata (FileDescription, ProductName, ProductVersion…).
pub fn file_string_info(path: &str, name: &str) -> String {
    unsafe {
        let p = HSTRING::from(path);
        let size = GetFileVersionInfoSizeW(&p, None);
        if size == 0 {
            return String::new();
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(&p, None, size, buf.as_mut_ptr() as *mut c_void).is_err() {
            return String::new();
        }
        let mut tp: *mut c_void = std::ptr::null_mut();
        let mut tl = 0u32;
        if !VerQueryValueW(buf.as_ptr() as *const c_void, &HSTRING::from(r"\VarFileInfo\Translation"), &mut tp, &mut tl).as_bool() || tl < 4 {
            return String::new();
        }
        let tr = std::slice::from_raw_parts(tp as *const u16, 2);
        let sub = HSTRING::from(format!(r"\StringFileInfo\{:04x}{:04x}\{name}", tr[0], tr[1]));
        let mut sp: *mut c_void = std::ptr::null_mut();
        let mut sl = 0u32;
        if !VerQueryValueW(buf.as_ptr() as *const c_void, &sub, &mut sp, &mut sl).as_bool() || sl == 0 {
            return String::new();
        }
        let s = std::slice::from_raw_parts(sp as *const u16, sl as usize);
        from_wide(s).trim().to_string()
    }
}

/// A readable name from exe metadata, otherwise the file name without .exe.
pub fn display_name(exe: &str) -> String {
    let key = exe.to_lowercase();
    let (cached, path) = with(|p| (p.desc.get(&key).cloned(), p.exe.get(&key).cloned()));
    if let Some(d) = cached {
        return d;
    }
    let mut name = path.as_deref().map(|p| file_string_info(p, "FileDescription")).unwrap_or_default();
    if name.is_empty() {
        let n = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(exe);
        let mut c = n.chars();
        name = match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        };
    }
    if path.is_some() {
        with(|p| p.desc.insert(key, name.clone()));
    }
    name
}

// ---------- icons ----------

type IconJob = (String, mpsc::Sender<Option<Vec<u8>>>);
static ICON_REQ: Mutex<Option<mpsc::Sender<IconJob>>> = Mutex::new(None);

/// Icons are taken on a thread with COM (STA): without it the shell sometimes gives no icon on the
/// first try for some apps (Steam games among them).
fn icon_worker() -> mpsc::Sender<IconJob> {
    let (tx, rx) = mpsc::channel::<IconJob>();
    std::thread::Builder::new()
        .name("icons".into())
        .spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            for (exe, out) in rx {
                let r = util::safely_value("icon", || icon_png_raw(&exe)).flatten();
                let _ = out.send(r);
            }
        })
        .ok();
    tx
}

/// PNG of the app icon (cached in memory).
pub fn icon_png(exe: &str) -> Option<Vec<u8>> {
    let key = exe.to_lowercase();
    if let Some(b) = with(|p| p.icon.get(&key).cloned()) {
        return b;
    }
    let tx = {
        let mut g = ICON_REQ.lock().unwrap();
        g.get_or_insert_with(icon_worker).clone()
    };
    let (otx, orx) = mpsc::channel();
    tx.send((exe.to_string(), otx)).ok()?;
    orx.recv_timeout(Duration::from_secs(5)).ok().flatten()
}

fn dib_bits(hdc: windows::Win32::Graphics::Gdi::HDC, hbm: HBITMAP, w: i32, h: i32) -> Option<Vec<u8>> {
    let mut bih = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let r = unsafe { GetDIBits(hdc, hbm, 0, h as u32, Some(buf.as_mut_ptr() as *mut c_void), &mut bih, DIB_RGB_COLORS) };
    (r != 0).then_some(buf)
}

fn icon_png_raw(exe: &str) -> Option<Vec<u8>> {
    let key = exe.to_lowercase();
    if let Some(b) = with(|p| p.icon.get(&key).cloned()) {
        return b;
    }
    let path = path_for(exe);
    if path.is_empty() {
        return None;
    }
    // the icon straight from the exe (the one in Explorer and on the taskbar) at 64 px — sharp and
    // free of the stale Windows icon cache; otherwise the shell icon (32 px)
    let mut wpath = util::wide(&path);
    wpath.resize(260, 0);
    let mut icons = [HICON::default()];
    let mut ids = [0u32];
    let n = unsafe { PrivateExtractIconsW(&wpath[..260].try_into().unwrap(), 0, 64, 64, Some(&mut icons), Some(ids.as_mut_ptr()), 0) };
    let mut hicon = icons[0];
    if n == 0 || n == u32::MAX || hicon.is_invalid() {
        let mut sfi = SHFILEINFOW::default();
        let p = HSTRING::from(path.as_str());
        let r = unsafe { SHGetFileInfoW(PCWSTR(p.as_ptr()), Default::default(), Some(&mut sfi), std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_ICON | SHGFI_LARGEICON) };
        if r == 0 || sfi.hIcon.is_invalid() {
            return None;
        }
        hicon = sfi.hIcon;
    }
    let png = icon_to_png(hicon);
    unsafe {
        let _ = DestroyIcon(hicon);
    }
    let png = png?;
    with(|p| p.icon.insert(key, Some(png.clone())));
    Some(png)
}

fn icon_to_png(hicon: HICON) -> Option<Vec<u8>> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(hicon, &mut ii).ok()?;
        let free = |ii: &ICONINFO| {
            if !ii.hbmColor.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
            }
            if !ii.hbmMask.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
            }
        };
        if ii.hbmColor.is_invalid() {
            free(&ii);
            return None;
        }
        let mut bm = BITMAP::default();
        GetObjectW(HGDIOBJ(ii.hbmColor.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void));
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        if w <= 0 || h <= 0 || w > 256 || h > 256 {
            free(&ii);
            return None;
        }
        let hdc = GetDC(None);
        let col = dib_bits(hdc, ii.hbmColor, w, h);
        let mask = dib_bits(hdc, ii.hbmMask, w, h);
        ReleaseDC(None, hdc);
        free(&ii);
        let mut col = col?;
        let has_alpha = col.chunks(4).any(|p| p[3] != 0);
        if !has_alpha {
            for i in 0..(w * h) as usize {
                col[i * 4 + 3] = if mask.as_ref().is_some_and(|m| m[i * 4] != 0) { 0 } else { 255 };
            }
        }
        let mut rgba = vec![0u8; col.len()];
        for i in 0..(w * h) as usize {
            rgba[i * 4] = col[i * 4 + 2];
            rgba[i * 4 + 1] = col[i * 4 + 1];
            rgba[i * 4 + 2] = col[i * 4];
            rgba[i * 4 + 3] = col[i * 4 + 3];
        }
        Some(crate::png::encode(w as u32, h as u32, &rgba))
    }
}

// ---------- windows and processes ----------

pub struct WinInfo {
    pub hwnd: HWND,
    pub pid: u32,
    pub title: String,
    #[allow(dead_code)]
    pub class: String,
}

unsafe extern "system" fn enum_cb(h: HWND, lp: LPARAM) -> BOOL {
    let out = unsafe { &mut *(lp.0 as *mut Vec<WinInfo>) };
    let mut pid = 0u32;
    let mut b = [0u16; 256];
    unsafe {
        GetWindowThreadProcessId(h, Some(&mut pid));
        let n = GetWindowTextW(h, &mut b);
        let title = String::from_utf16_lossy(&b[..n.max(0) as usize]);
        let n = GetClassNameW(h, &mut b);
        let class = String::from_utf16_lossy(&b[..n.max(0) as usize]);
        out.push(WinInfo { hwnd: h, pid, title, class });
    }
    TRUE
}

pub fn top_windows() -> Vec<WinInfo> {
    let mut out: Vec<WinInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

pub fn is_visible(h: HWND) -> bool {
    unsafe { IsWindowVisible(h).as_bool() }
}

/// Running processes (lower-case exe name → PID): an app that is silent right now (no audio
/// session, like Discord outside a voice channel) is not "not running", and gets its icon.
pub fn running_procs() -> HashMap<String, u32> {
    let mut out = HashMap::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut pe = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut pe).is_ok();
        while ok {
            out.insert(from_wide(&pe.szExeFile).to_lowercase(), pe.th32ProcessID);
            ok = Process32NextW(snap, &mut pe).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// App icons for the mock (tools/mock/icons, not in git), extracted the way the app does:
    /// ARCMIX_ICON_DUMP="<out dir>|<exe path>;<exe path>…" cargo test -- --ignored icon_dump
    #[test]
    #[ignore]
    fn icon_dump() {
        let Ok(spec) = std::env::var("ARCMIX_ICON_DUMP") else { return };
        let (dir, paths) = spec.split_once('|').unwrap();
        for path in paths.split(';').filter(|p| !p.is_empty()) {
            let exe = path.rsplit(['\\', '/']).next().unwrap();
            remember_path(exe, path);
            let png = icon_png_raw(exe).unwrap_or_else(|| panic!("no icon: {path}"));
            std::fs::write(format!("{dir}/{exe}.png"), png).unwrap();
        }
    }
}
