//! Update check on request (About → "Check for updates"): the tag of the latest release in the
//! GitHub repository, through WinHTTP (TLS and the proxy settings come from Windows). Nothing is
//! checked on its own.

use std::ffi::c_void;

use windows::{
    Win32::Networking::WinHttp::{
        INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle,
        WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    },
    core::{HSTRING, PCWSTR, w},
};

use crate::{APP_VER, logf};

/// The page of the latest release ("Download" opens it; listed in server::OPEN_URLS).
pub const RELEASES_PAGE: &str = "https://github.com/Fenderenko/Arcmix/releases/latest";
const API_PATH: PCWSTR = w!("/repos/Fenderenko/Arcmix/releases/latest");
/// A release description is a few KB; anything far larger is not that answer.
const MAX_BODY: usize = 1 << 20;

/// A WinHTTP handle, closed when dropped.
struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// (any thread; blocks for up to ~20 s) The version of the latest release ("1.0.1"), or None when
/// GitHub cannot be reached or there is no release yet.
pub fn latest() -> Option<String> {
    let body = fetch(API_PATH)?;
    let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let tag = v.get("tag_name")?.as_str()?.trim().trim_start_matches('v');
    (!tag.is_empty()).then(|| tag.to_string())
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

    /// A real request over the network: cargo test --release -- --ignored live_github
    #[test]
    #[ignore]
    fn live_github() {
        let body = fetch(w!("/repos/Swiftlyx/OpenChatMix")).expect("no answer from GitHub");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["full_name"], "Swiftlyx/OpenChatMix");
    }
}
