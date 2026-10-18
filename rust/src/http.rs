//! A small HTTP/1.1 server for the window page on 127.0.0.1: keep-alive connections, a thread per
//! connection (the page opens a handful), and long-lived responses for server-sent events.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    time::Duration,
};

pub struct Request {
    pub method: String,
    pub path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn query(&self, name: &str) -> &str {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).unwrap_or("")
    }

    /// The JSON body as T (lenient: a missing or bad body gives defaults).
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_slice(&self.body).ok()
    }
}

pub enum Reply {
    NoContent,
    Json(String),
    Body { ctype: &'static str, cache: &'static str, body: Vec<u8> },
    Status(u16, &'static str),
    /// The handler took the connection over (server-sent events) — nothing more to write.
    Taken,
}

pub fn json<T: serde::Serialize>(v: &T) -> Reply {
    Reply::Json(serde_json::to_string(v).unwrap_or_else(|_| "null".into()))
}

pub type Handler = dyn Fn(&Request, &mut TcpStream) -> Reply + Send + Sync;

/// Serves forever on the listener (call from a thread of its own).
pub fn serve(ln: TcpListener, h: Arc<Handler>) {
    for s in ln.incoming() {
        let Ok(s) = s else { continue };
        let h = h.clone();
        let _ = std::thread::Builder::new().name("http".into()).spawn(move || conn(s, h));
    }
}

fn conn(mut s: TcpStream, h: Arc<Handler>) {
    let _ = s.set_nodelay(true);
    let _ = s.set_read_timeout(Some(Duration::from_secs(120)));
    let Ok(rs) = s.try_clone() else { return };
    let mut r = BufReader::new(rs);
    loop {
        let Some(req) = read_request(&mut r) else { return };
        let reply = crate::util::safely_value("http handler", || h(&req, &mut s)).unwrap_or(Reply::Status(500, "Internal Server Error"));
        let ok = match reply {
            Reply::Taken => return,
            Reply::NoContent => write(&mut s, 204, "No Content", "", "no-store", &[]),
            Reply::Json(b) => write(&mut s, 200, "OK", "application/json; charset=utf-8", "no-store", b.as_bytes()),
            Reply::Body { ctype, cache, body } => write(&mut s, 200, "OK", ctype, cache, &body),
            Reply::Status(code, text) => write(&mut s, code, text, "text/plain; charset=utf-8", "no-store", text.as_bytes()),
        };
        if !ok {
            return;
        }
    }
}

fn write(s: &mut TcpStream, code: u16, text: &str, ctype: &str, cache: &str, body: &[u8]) -> bool {
    let mut head = format!("HTTP/1.1 {code} {text}\r\nContent-Length: {}\r\nCache-Control: {cache}\r\n", body.len());
    if !ctype.is_empty() {
        head += &format!("Content-Type: {ctype}\r\n");
    }
    head += "\r\n";
    s.write_all(head.as_bytes()).and_then(|_| s.write_all(body)).and_then(|_| s.flush()).is_ok()
}

fn read_request(r: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut headers = Vec::new();
    loop {
        let mut l = String::new();
        if r.read_line(&mut l).ok()? == 0 {
            return None;
        }
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
        if headers.len() > 100 {
            return None;
        }
    }
    let len: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    if len > 4 << 20 {
        return None;
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).ok()?;
    let (path, qs) = target.split_once('?').unwrap_or((&target, ""));
    let query = qs
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect();
    Some(Request { method, path: decode(path), query, headers, body })
}

/// Percent-decoding (and '+' as a space) of a query part.
pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn decode() {
        assert_eq!(super::decode("a%20b+c"), "a b c");
        assert_eq!(super::decode("Discord.exe"), "Discord.exe");
        assert_eq!(super::decode("%D0%86"), "І");
        assert_eq!(super::decode("50%"), "50%");
    }
}
