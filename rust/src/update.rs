//! Update check on request (About → "Check for updates"): the tag of the latest release in the
//! GitHub repository, through WinHTTP (TLS and the proxy settings come from Windows). Nothing is
//! checked on its own. "Update" puts the release's exe in place of the running one and restarts.

use std::{ffi::c_void, fs, path::PathBuf};

use windows::{
    Win32::Networking::WinHttp::{
        INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle,
        WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    },
    core::{HSTRING, PCWSTR, w},
};

use crate::{APP_NAME, APP_VER, logf};

/// The page of the latest release ("Download" opens it; listed in server::OPEN_URLS).
pub const RELEASES_PAGE: &str = "https://github.com/Fenderenko/Arcmix/releases/latest";
const API_PATH: PCWSTR = w!("/repos/Fenderenko/Arcmix/releases/latest");
/// A release description is a few KB; anything far larger is not that answer.
const MAX_BODY: usize = 1 << 20;
/// Where the release files lie on github.com (the exe is taken only from there).
const DOWNLOADS: &str = "/Fenderenko/Arcmix/releases/download/";

/// The latest release: its version and the path of its exe on github.com (CI puts Arcmix-<version>.exe
/// next to the zip; older releases have none).
#[derive(Clone, Debug)]
pub struct Latest {
    pub ver: String,
    pub exe: Option<String>,
}

/// A WinHTTP handle, closed when dropped.
struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// (any thread; blocks for up to ~20 s) The latest release ("1.0.1"), or None when GitHub cannot be
/// reached or there is no release yet.
pub fn latest() -> Option<Latest> {
    let body = fetch(API_PATH)?;
    let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
    parse_latest(&v)
}

fn parse_latest(v: &serde_json::Value) -> Option<Latest> {
    let tag = v.get("tag_name")?.as_str()?.trim().trim_start_matches('v');
    if tag.is_empty() {
        return None;
    }
    let name = format!("{APP_NAME}-{tag}.exe");
    let exe = v.get("assets").and_then(|a| a.as_array()).and_then(|a| {
        a.iter()
            .find(|x| x.get("name").and_then(|n| n.as_str()) == Some(name.as_str()))
            .and_then(|x| x.get("browser_download_url")?.as_str()?.strip_prefix("https://github.com"))
            .filter(|p| p.starts_with(DOWNLOADS))
            .map(str::to_string)
    });
    Some(Latest { ver: tag.to_string(), exe })
}

/// (any thread; blocks while downloading) Puts the release's exe in place of the running one (the
/// running exe can be renamed, not overwritten) and starts it; the new copy waits for this one to
/// quit (main: --after-update). The caller quits then. Err — nothing changed.
pub fn install(l: &Latest) -> Result<(), String> {
    let path = l.exe.as_deref().ok_or("the release has no exe")?;
    let cur = std::env::current_exe().map_err(|e| e.to_string())?;
    let (new, old) = (PathBuf::from(format!("{}.new", cur.display())), PathBuf::from(format!("{}.old", cur.display())));
    let body = get_bytes("github.com", path, 64 << 20)?;
    if body.len() < 1 << 20 || !body.starts_with(b"MZ") {
        return Err("the download is not the program".into());
    }
    fs::write(&new, &body).map_err(|e| format!("{}: {e}", new.display()))?;
    // the version inside the file, not just its name
    let n = new.to_string_lossy();
    if crate::appinfo::file_string_info(&n, "ProductName") != APP_NAME || crate::appinfo::file_string_info(&n, "ProductVersion") != l.ver {
        let _ = fs::remove_file(&new);
        return Err("the download is not the expected version".into());
    }
    let _ = fs::remove_file(&old);
    if let Err(e) = fs::rename(&cur, &old) {
        let _ = fs::remove_file(&new);
        return Err(format!("{}: {e}", cur.display()));
    }
    let back = |e: String| {
        let _ = fs::rename(&cur, &new);
        let _ = fs::rename(&old, &cur);
        Err(e)
    };
    if let Err(e) = fs::rename(&new, &cur) {
        let _ = fs::rename(&old, &cur);
        let _ = fs::remove_file(&new);
        return Err(format!("{}: {e}", cur.display()));
    }
    if let Err(e) = std::process::Command::new(&cur).arg("--after-update").arg(std::process::id().to_string()).spawn() {
        return back(e.to_string());
    }
    logf!("update: {APP_VER} → {} in place, restarting", l.ver);
    Ok(())
}

/// (the new copy, before anything else) The previous copy is still quitting: waited for (it holds the
/// single-copy mutex), its renamed exe removed after.
pub fn after_update(pid: u32) {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(h, 20_000);
            let _ = CloseHandle(h);
        }
    }
    if let Ok(cur) = std::env::current_exe() {
        let _ = fs::remove_file(format!("{}.old", cur.display()));
    }
}

/// An open HTTPS GET with the answer received (status 200); the session and the connection live as
/// long as the request.
struct Get {
    _ses: Handle,
    _con: Handle,
    req: Handle,
}

fn get(host: &str, path: &str, accept: &str) -> Result<Get, String> {
    unsafe {
        let agent = HSTRING::from(format!("Arcmix/{APP_VER}"));
        let ses = Handle(WinHttpOpen(&agent, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if ses.0.is_null() {
            return Err("WinHttpOpen".into());
        }
        let _ = WinHttpSetTimeouts(ses.0, 5000, 5000, 5000, 8000);
        let con = Handle(WinHttpConnect(ses.0, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if con.0.is_null() {
            return Err("WinHttpConnect".into());
        }
        // redirects (to a download host) are followed by WinHTTP itself
        let req = Handle(WinHttpOpenRequest(con.0, w!("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
        if req.0.is_null() {
            return Err("WinHttpOpenRequest".into());
        }
        let headers: Vec<u16> = format!("Accept: {accept}
").encode_utf16().collect();
        WinHttpSendRequest(req.0, Some(&headers), None, 0, 0, 0).and_then(|_| WinHttpReceiveResponse(req.0, std::ptr::null_mut())).map_err(|e| e.to_string())?;
        let (mut code, mut len) = (0u32, 4u32);
        WinHttpQueryHeaders(req.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut code as *mut u32 as *mut c_void), &mut len, std::ptr::null_mut())
            .map_err(|e| e.to_string())?;
        if code != 200 {
            return Err(format!("HTTP {code}"));
        }
        Ok(Get { _ses: ses, _con: con, req })
    }
}

impl Get {
    /// The next piece of the body into buf; 0 at the end.
    fn read(&self, buf: &mut [u8]) -> Result<usize, String> {
        let mut n = 0u32;
        unsafe { WinHttpReadData(self.req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n) }.map_err(|e| e.to_string())?;
        Ok(n as usize)
    }
}

/// (any thread; blocks) A whole file over HTTPS, at most `max` bytes.
pub fn get_bytes(host: &str, path: &str, max: usize) -> Result<Vec<u8>, String> {
    let g = get(host, path, "*/*")?;
    let mut body = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = g.read(&mut buf)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
        if body.len() > max {
            return Err("larger than expected".into());
        }
    }
    Ok(body)
}

fn fetch(path: PCWSTR) -> Option<Vec<u8>> {
    let path = unsafe { path.to_string() }.ok()?;
    let g = match get("api.github.com", &path, "application/vnd.github+json") {
        Ok(g) => g,
        Err(e) => {
            // HTTP 404: no release published yet
            logf!("update check: {e}");
            return None;
        }
    };
    let mut body = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = g.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
        if body.len() > MAX_BODY {
            return None;
        }
    }
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exe of a release only from this repository's downloads.
    #[test]
    fn latest_exe() {
        let v = serde_json::json!({ "tag_name": "v1.2.3", "assets": [
            { "name": "Arcmix-1.2.3.zip", "browser_download_url": "https://github.com/Fenderenko/Arcmix/releases/download/v1.2.3/Arcmix-1.2.3.zip" },
            { "name": "Arcmix-1.2.3.exe", "browser_download_url": "https://github.com/Fenderenko/Arcmix/releases/download/v1.2.3/Arcmix-1.2.3.exe" } ] });
        let l = parse_latest(&v).unwrap();
        assert_eq!(l.ver, "1.2.3");
        assert_eq!(l.exe.as_deref(), Some("/Fenderenko/Arcmix/releases/download/v1.2.3/Arcmix-1.2.3.exe"));
        let elsewhere = serde_json::json!({ "tag_name": "v1.2.3", "assets": [{ "name": "Arcmix-1.2.3.exe", "browser_download_url": "https://example.com/Arcmix-1.2.3.exe" }] });
        assert_eq!(parse_latest(&elsewhere).unwrap().exe, None);
        assert_eq!(parse_latest(&serde_json::json!({ "tag_name": "v1.2.0" })).unwrap().exe, None, "an older release without the exe");
    }

    /// A real request over the network: cargo test --release -- --ignored live_github
    #[test]
    #[ignore]
    fn live_github() {
        let body = fetch(w!("/repos/Swiftlyx/OpenChatMix")).expect("no answer from GitHub");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["full_name"], "Swiftlyx/OpenChatMix");
    }
}
