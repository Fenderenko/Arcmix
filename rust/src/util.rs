//! Small helpers: wide strings, the log, data paths, registry, process paths, crash output and
//! running closures on the UI thread.

use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicIsize, Ordering},
    },
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, WPARAM},
        System::{
            Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_DWORD, REG_MULTI_SZ, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
                RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
            },
            SystemInformation::GetLocalTime,
            Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW},
        },
        UI::WindowsAndMessaging::PostMessageW,
    },
    core::{HSTRING, PWSTR},
};

// ---------- wide strings ----------

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Text up to the first NUL.
pub fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// Copies s into a fixed buffer, always NUL-terminated (cut if too long).
pub fn copy_wide(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for c in s.encode_utf16() {
        if n + 1 >= dst.len() {
            break;
        }
        dst[n] = c;
        n += 1;
    }
    for d in dst[n..].iter_mut() {
        *d = 0;
    }
}

// ---------- paths and the log ----------

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// %APPDATA%\Arcmix — settings, log and crash report.
pub fn data_dir() -> &'static Path {
    DATA_DIR.get().map(|p| p.as_path()).unwrap_or(Path::new("."))
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn appdata_base() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

struct LogFile {
    file: File,
    size: u64,
}

static LOG: Mutex<Option<LogFile>> = Mutex::new(None);

pub fn init_paths(dir_name: &str) {
    let dir = appdata_base().join(dir_name);
    let _ = fs::create_dir_all(&dir);
    let _ = DATA_DIR.set(dir.clone());
    // a small log: over 256 KB at start — begin anew
    let lp = dir.join("log.txt");
    if fs::metadata(&lp).map(|m| m.len() > 256 * 1024).unwrap_or(false) {
        let _ = fs::remove_file(&lp);
    }
    if let Ok(f) = OpenOptions::new().create(true).append(true).open(&lp) {
        let size = f.metadata().map(|m| m.len()).unwrap_or(0);
        *LOG.lock().unwrap() = Some(LogFile { file: f, size });
    }
    let cp = dir.join("crash.txt");
    if fs::metadata(&cp).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = fs::remove_file(&cp);
    }
}

/// "2006-01-02 15:04:05" in local time.
pub fn timestamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

/// "2006-01-02-150405" in local time (for file names).
pub fn file_stamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02}-{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

/// One line in log.txt; over 1 MB within a session the log starts anew.
pub fn log(msg: &str) {
    let mut g = match LOG.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let Some(l) = g.as_mut() else { return };
    if l.size > 1 << 20 {
        let _ = l.file.set_len(0);
        l.size = 0;
    }
    let line = format!("{} {}\n", timestamp(), msg);
    if l.file.write_all(line.as_bytes()).is_ok() {
        l.size += line.len() as u64;
    }
}

#[macro_export]
macro_rules! logf {
    ($($t:tt)*) => { $crate::util::log(&format!($($t)*)) };
}

/// Panics: message and backtrace to the log and crash.txt (a windowed app would vanish silently).
pub fn install_crash_output() {
    panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let msg = format!("PANIC in thread {thread}: {info}\n{bt}");
        log(&msg);
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(data_dir().join("crash.txt")) {
            let _ = writeln!(f, "{} {}", timestamp(), msg);
        }
    }));
    unsafe {
        windows::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(Some(seh_filter));
    }
}

unsafe extern "system" fn seh_filter(info: *const windows::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
    let (code, addr) = unsafe {
        let r = &*(*info).ExceptionRecord;
        (r.ExceptionCode.0 as u32, r.ExceptionAddress as usize)
    };
    let msg = format!("fatal exception 0x{code:08x} at 0x{addr:x}");
    log(&msg);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(data_dir().join("crash.txt")) {
        let _ = writeln!(f, "{} {}", timestamp(), msg);
    }
    0 // EXCEPTION_CONTINUE_SEARCH
}

/// Runs f; a panic is logged instead of taking the whole app down.
pub fn safely<F: FnOnce()>(name: &str, f: F) -> bool {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(()) => true,
        Err(_) => {
            log(&format!("PANIC in {name} (see above)"));
            false
        }
    }
}

/// Like `safely`, with the closure's result (None after a panic).
pub fn safely_value<R>(name: &str, f: impl FnOnce() -> R) -> Option<R> {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(v) => Some(v),
        Err(_) => {
            log(&format!("PANIC in {name} (see above)"));
            None
        }
    }
}

// ---------- UI thread ----------

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

pub fn set_main_hwnd(h: HWND) {
    MAIN_HWND.store(h.0 as isize, Ordering::Relaxed);
}

pub fn main_hwnd() -> HWND {
    HWND(MAIN_HWND.load(Ordering::Relaxed) as *mut _)
}

pub fn post_ui(msg: u32, wp: usize) {
    let h = main_hwnd();
    if !h.is_invalid() {
        unsafe {
            let _ = PostMessageW(Some(h), msg, WPARAM(wp), LPARAM(0));
        }
    }
}

type UiTask = Box<dyn FnOnce() + Send>;
static UI_TASKS: Mutex<Vec<UiTask>> = Mutex::new(Vec::new());

/// Runs f on the UI thread (window and tray work must happen there).
pub fn run_on_ui(f: impl FnOnce() + Send + 'static) {
    UI_TASKS.lock().unwrap().push(Box::new(f));
    post_ui(crate::WM_RUN_UI, 0);
}

pub fn drain_ui() {
    let tasks: Vec<UiTask> = std::mem::take(&mut *UI_TASKS.lock().unwrap());
    for t in tasks {
        safely("ui task", t);
    }
}

// ---------- registry ----------

pub struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

pub fn reg_open(path: &str) -> Option<RegKey> {
    let mut k = HKEY::default();
    let p = HSTRING::from(path);
    unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &p, None, KEY_READ, &mut k) }.ok().ok()?;
    Some(RegKey(k))
}

/// A HKEY_LOCAL_MACHINE key for reading, the 64-bit registry view.
pub fn reg_open_hklm64(path: &str) -> Option<RegKey> {
    let mut k = HKEY::default();
    let p = HSTRING::from(path);
    unsafe { RegOpenKeyExW(windows::Win32::System::Registry::HKEY_LOCAL_MACHINE, &p, None, KEY_READ | windows::Win32::System::Registry::KEY_WOW64_64KEY, &mut k) }.ok().ok()?;
    Some(RegKey(k))
}

/// A HKEY_LOCAL_MACHINE key for writing, the 64-bit registry view (elevated only).
pub fn reg_create_hklm64(path: &str) -> Option<RegKey> {
    let mut k = HKEY::default();
    let p = HSTRING::from(path);
    unsafe {
        RegCreateKeyExW(windows::Win32::System::Registry::HKEY_LOCAL_MACHINE, &p, None, None, REG_OPTION_NON_VOLATILE, KEY_SET_VALUE | KEY_READ | windows::Win32::System::Registry::KEY_WOW64_64KEY, None, &mut k, None)
    }
    .ok()
    .ok()?;
    Some(RegKey(k))
}

pub fn reg_create(path: &str) -> Option<RegKey> {
    let mut k = HKEY::default();
    let p = HSTRING::from(path);
    unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, &p, None, None, REG_OPTION_NON_VOLATILE, KEY_SET_VALUE | KEY_READ, None, &mut k, None) }
        .ok()
        .ok()?;
    Some(RegKey(k))
}

impl RegKey {
    pub fn dword(&self, name: &str) -> Option<u32> {
        let n = HSTRING::from(name);
        let mut v = 0u32;
        let mut sz = 4u32;
        let mut t = REG_VALUE_TYPE::default();
        unsafe { RegQueryValueExW(self.0, &n, None, Some(&mut t), Some(&mut v as *mut u32 as *mut u8), Some(&mut sz)) }
            .ok()
            .ok()?;
        (t == REG_DWORD).then_some(v)
    }

    pub fn binary(&self, name: &str, max: usize) -> Option<Vec<u8>> {
        let n = HSTRING::from(name);
        let mut buf = vec![0u8; max];
        let mut sz = max as u32;
        unsafe { RegQueryValueExW(self.0, &n, None, None, Some(buf.as_mut_ptr()), Some(&mut sz)) }.ok().ok()?;
        buf.truncate(sz as usize);
        Some(buf)
    }

    pub fn has(&self, name: &str) -> bool {
        let n = HSTRING::from(name);
        unsafe { RegQueryValueExW(self.0, &n, None, None, None, None) }.is_ok()
    }

    pub fn string(&self, name: &str) -> Option<String> {
        let n = HSTRING::from(name);
        let mut sz = 0u32;
        unsafe { RegQueryValueExW(self.0, &n, None, None, None, Some(&mut sz)) }.ok().ok()?;
        if sz < 2 {
            return None;
        }
        let mut buf = vec![0u16; sz as usize / 2 + 1];
        unsafe { RegQueryValueExW(self.0, &n, None, None, Some(buf.as_mut_ptr() as *mut u8), Some(&mut sz)) }.ok().ok()?;
        Some(from_wide(&buf))
    }

    pub fn set_string(&self, name: &str, value: &str) -> bool {
        let n = HSTRING::from(name);
        let v = wide(value);
        let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) };
        unsafe { RegSetValueExW(self.0, &n, None, REG_SZ, Some(bytes)) }.is_ok()
    }

    /// REG_MULTI_SZ: the strings, each ended by NUL, and a final NUL.
    pub fn set_multi_string(&self, name: &str, values: &[&str]) -> bool {
        let n = HSTRING::from(name);
        let mut v: Vec<u16> = values.iter().flat_map(|s| s.encode_utf16().chain(std::iter::once(0))).collect();
        v.push(0);
        let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) };
        unsafe { RegSetValueExW(self.0, &n, None, REG_MULTI_SZ, Some(bytes)) }.is_ok()
    }

    pub fn set_dword(&self, name: &str, value: u32) -> bool {
        let n = HSTRING::from(name);
        unsafe { RegSetValueExW(self.0, &n, None, REG_DWORD, Some(&value.to_le_bytes())) }.is_ok()
    }

    pub fn delete(&self, name: &str) {
        let n = HSTRING::from(name);
        unsafe {
            let _ = RegDeleteValueW(self.0, &n);
        }
    }

    /// The names of the subkeys.
    pub fn subkeys(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut buf = [0u16; 256];
        for i in 0.. {
            let mut len = buf.len() as u32;
            if unsafe { windows::Win32::System::Registry::RegEnumKeyExW(self.0, i, Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) }.is_err() {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        out
    }
}

// ---------- processes ----------

struct PidPath {
    path: String,
    at: Instant,
}

/// PID → exe path cache: audio sessions are walked every 3 s, opening each process every time is
/// wasteful. A PID can be reused by another process, so entries live a minute.
static PID_PATHS: Mutex<Option<HashMap<u32, PidPath>>> = Mutex::new(None);

pub fn prune_path_cache() {
    if let Some(m) = PID_PATHS.lock().unwrap().as_mut() {
        m.retain(|_, v| v.at.elapsed() < Duration::from_secs(60));
    }
}

pub fn exe_path_of(pid: u32) -> String {
    {
        let g = PID_PATHS.lock().unwrap();
        if let Some(v) = g.as_ref().and_then(|m| m.get(&pid)) {
            if v.at.elapsed() < Duration::from_secs(60) {
                return v.path.clone();
            }
        }
    }
    let p = exe_path_raw(pid);
    if !p.is_empty() {
        PID_PATHS.lock().unwrap().get_or_insert_with(HashMap::new).insert(pid, PidPath { path: p.clone(), at: Instant::now() });
    }
    p
}

pub fn exe_path_raw(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = vec![0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(h);
        if ok { String::from_utf16_lossy(&buf[..n as usize]) } else { String::new() }
    }
}

pub fn base_name(full: &str) -> &str {
    match full.rfind(['\\', '/']) {
        Some(i) => &full[i + 1..],
        None => full,
    }
}

/// Shell "open" of a file, folder or URL.
pub fn shell_open(target: &str) {
    let t = HSTRING::from(target);
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            &t,
            None,
            None,
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}
