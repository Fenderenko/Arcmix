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
const API_HOST: PCWSTR = w!("api.github.com");
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

fn fetch(path: PCWSTR) -> Option<Vec<u8>> {
    unsafe {
        let agent = HSTRING::from(format!("Arcmix/{APP_VER}"));
        let ses = Handle(WinHttpOpen(&agent, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if ses.0.is_null() {
            return None;
        }
        let _ = WinHttpSetTimeouts(ses.0, 5000, 5000, 5000, 8000);
        let con = Handle(WinHttpConnect(ses.0, API_HOST, INTERNET_DEFAULT_HTTPS_PORT, 0));
        if con.0.is_null() {
            return None;
        }
        let req = Handle(WinHttpOpenRequest(con.0, w!("GET"), path, PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
        if req.0.is_null() {
            return None;
        }
        let headers: Vec<u16> = "Accept: application/vnd.github+json\r\n".encode_utf16().collect();
        if let Err(e) = WinHttpSendRequest(req.0, Some(&headers), None, 0, 0, 0).and_then(|_| WinHttpReceiveResponse(req.0, std::ptr::null_mut())) {
            logf!("update check: {e}");
            return None;
        }
        let (mut code, mut len) = (0u32, 4u32);
        WinHttpQueryHeaders(req.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut code as *mut u32 as *mut c_void), &mut len, std::ptr::null_mut()).ok()?;
        if code != 200 {
            // 404: no release published yet
            logf!("update check: HTTP {code}");
            return None;
        }
        let mut body = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let mut n = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n).ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&buf[..n as usize]);
            if body.len() > MAX_BODY {
                return None;
            }
        }
        Some(body)
    }
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
