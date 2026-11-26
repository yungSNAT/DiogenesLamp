// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Serve the viewer and the data directory on localhost, as Python's
//! `ThreadingHTTPServer` with `SimpleHTTPRequestHandler` (HTTP/1.0, one
//! request per connection) and DiogenesLamp's handler overrides.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::pyerr;
use crate::pystr;
use crate::pyurl;
use crate::viewer;

/// A name the scraper never writes, under the data directory: refused
/// requests are pointed at it so they 404 by not existing.
pub const REFUSED: &str = ".refused-traversal";
const SERVER_VERSION: &str = "SimpleHTTP/0.6 DiogenesLamp/0.1";
pub const CSP: &str = "default-src 'none'; img-src 'self' data:; script-src 'self'; style-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'none'";

const ERROR_TEMPLATE: &str = "<!DOCTYPE HTML>
<html lang=\"en\">
    <head>
        <meta charset=\"utf-8\">
        <style type=\"text/css\">
            :root {
                color-scheme: light dark;
            }
        </style>
        <title>Error response</title>
    </head>
    <body>
        <h1>Error response</h1>
        <p>Error code: %(code)d</p>
        <p>Message: %(message)s.</p>
        <p>Error code explanation: %(code)s - %(explain)s.</p>
    </body>
</html>
";

/// `http.server`'s phrases for the codes this server sends.
const STATUS_TEXT: [(u16, &str, &str); 8] = [
    (200, "OK", "Request fulfilled, document follows"),
    (
        301,
        "Moved Permanently",
        "Object moved permanently -- see URI list",
    ),
    (
        304,
        "Not Modified",
        "Document has not changed since given time",
    ),
    (
        400,
        "Bad Request",
        "Bad request syntax or unsupported method",
    ),
    (
        403,
        "Forbidden",
        "Request forbidden -- authorization will not help",
    ),
    (404, "Not Found", "Nothing matches the given URI"),
    (
        431,
        "Request Header Fields Too Large",
        "The server refused this request because the request header fields are too large",
    ),
    (
        501,
        "Not Implemented",
        "Server does not support this operation",
    ),
];

const SOCKET_READ_TIMEOUT: Duration = Duration::from_secs(10);
const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// After its first `SOCKET_WRITE_TIMEOUT`, a response must keep this average
/// rate in bytes a second: a reader taking a little at a time cannot hold
/// its thread, and a large catalogue shard still has time to arrive.
const MIN_WRITE_RATE: u64 = 64 * 1024;

/// Reads or writes with each call bounded by what is left of one deadline,
/// so a client trickling bytes cannot hold its thread past it.
struct Deadline<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl Deadline<'_> {
    fn left(&self) -> std::io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| std::io::ErrorKind::TimedOut.into())
    }
}

impl Read for Deadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.set_read_timeout(Some(self.left()?))?;
        let mut stream = self.stream;
        stream.read(buf)
    }
}

impl Write for Deadline<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Still no longer than the old per-call limit, so a reader that
        // stops altogether is dropped as soon as before.
        self.stream
            .set_write_timeout(Some(self.left()?.min(SOCKET_WRITE_TIMEOUT)))?;
        let mut stream = self.stream;
        stream.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut stream = self.stream;
        stream.flush()
    }
}

/// The time a response of `len` bytes has to be written.
fn write_allowance(len: usize) -> Duration {
    SOCKET_WRITE_TIMEOUT + Duration::from_secs(len as u64 / MIN_WRITE_RATE)
}

fn write_by_deadline(stream: &TcpStream, data: &[u8]) -> std::io::Result<()> {
    let deadline = Instant::now() + write_allowance(data.len());
    let mut writer = Deadline { stream, deadline };
    writer.write_all(data)?;
    writer.flush()
}

fn configure_stream(stream: &TcpStream) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(SOCKET_READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SOCKET_WRITE_TIMEOUT));
}

fn valid_host_header(value: &str, expected_port: u16) -> bool {
    let (host, port) = if let Some(rest) = value.strip_prefix('[') {
        let Some((ipv6, tail)) = rest.split_once(']') else {
            return false;
        };
        if ipv6 != "::1" {
            return false;
        }
        if tail.is_empty() {
            ("::1", None)
        } else if let Some(p) = tail.strip_prefix(':') {
            ("::1", Some(p))
        } else {
            return false;
        }
    } else if let Some((h, p)) = value.split_once(':') {
        (h, Some(p))
    } else {
        (value, None)
    };
    let host_ok = host == "127.0.0.1" || host == "::1" || host.eq_ignore_ascii_case("localhost");
    let port_ok = port.is_none_or(|p| p == expected_port.to_string());
    host_ok && port_ok
}

fn status_text(code: u16) -> (&'static str, &'static str) {
    STATUS_TEXT
        .iter()
        .find(|(known, _, _)| *known == code)
        .map_or(("???", "???"), |(_, short, long)| (short, long))
}

/// Where viewer files come from: a directory, or the copies in the binary.
#[derive(Clone, Debug)]
pub enum Viewer {
    Dir(PathBuf),
    Embedded,
}

/// `email.utils.formatdate(timestamp, usegmt=True)`.
pub fn http_date(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86_400);
    let secs = timestamp.rem_euclid(86_400);
    let date = crate::isodate::Date {
        ordinal: days + 719_163,
    };
    let (y, m, d) = date.ymd();
    let weekday = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        [(date.ordinal - 1).rem_euclid(7) as usize];
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(m - 1) as usize];
    format!(
        "{weekday}, {d:02} {month} {y:04} {:02}:{:02}:{:02} GMT",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// `email.utils.parsedate_to_datetime`, kept to a UTC timestamp; None for
/// dates Python ignores or would not compare (non-UTC offsets). Where RFC 9110
/// section 5.6.7 differs, Python wins: a leap second is ignored, and a 2-digit
/// year of 69 to 99 is 19xx and 00 to 68 is 20xx, not the RFC's 50-year rule.
fn parse_http_date(text: &str) -> Option<i64> {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    if words
        .first()
        .is_some_and(|w| w.ends_with(',') || w.len() == 3 && w.chars().all(char::is_alphabetic))
    {
        words.remove(0);
    }
    // An RFC 850 date: `06-Nov-94 08:49:37 GMT`.
    if words.len() == 3 && words[0].split('-').count() == 3 {
        let date = words[0];
        words.splice(0..1, date.split('-'));
    }
    if words.len() < 4 {
        return None;
    }
    let (day, month) = if words[0].chars().all(|c| c.is_ascii_digit()) {
        (words[0], words[1])
    } else {
        (words[1], words[0])
    };
    // An asctime date puts the year after the time: `Nov  6 08:49:37 1994`.
    let (year, time) = if words[2].contains(':') {
        (words[3], words[2])
    } else {
        (words[2], words[3])
    };
    let month = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| month.to_lowercase().starts_with(m))?
        + 1;
    let day: i64 = day.parse().ok()?;
    let mut year: i64 = year.parse().ok()?;
    if year < 100 {
        year += if year > 68 { 1900 } else { 2000 };
    }
    let time: Vec<i64> = time.split(':').map(|p| p.parse().unwrap_or(-1)).collect();
    let (h, mi, s) = match time.as_slice() {
        [h, m] => (*h, *m, 0),
        [h, m, s] => (*h, *m, *s),
        _ => return None,
    };
    // Python's datetime refuses a leap second, which RFC 9110 allows.
    if !(0..24).contains(&h) || !(0..60).contains(&mi) || !(0..60).contains(&s) {
        return None;
    }
    match words.get(4).map(|z| z.to_uppercase()) {
        None => {}
        Some(z) if matches!(z.as_str(), "GMT" | "UT" | "UTC" | "Z" | "+0000" | "-0000") => {}
        Some(_) => return None,
    }
    let date = crate::isodate::Date::from_ymd(year, month as i64, day)?;
    Some((date.ordinal - 719_163) * 86_400 + h * 3600 + mi * 60 + s)
}

/// The Content-Type of a file lamp writes or embeds, by its extension. As in
/// Python's `splitext`, either slash ends a directory and leading dots are
/// part of the name, so `..json` has no extension.
fn guess_type(path: &str) -> &'static str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or_default();
    let ext = name
        .trim_start_matches('.')
        .rsplit_once('.')
        .map(|(_, e)| e);
    match ext.map(str::to_ascii_lowercase).as_deref() {
        Some("html" | "htm") => "text/html",
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("json") => "application/json",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

/// `posixpath.normpath`.
fn posix_normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let initial = if path.starts_with("//") && !path.starts_with("///") {
        2
    } else {
        usize::from(path.starts_with('/'))
    };
    let mut parts: Vec<&str> = Vec::new();
    for comp in path.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp != ".." || (initial == 0 && parts.is_empty()) || parts.last() == Some(&"..") {
            parts.push(comp);
        } else if !parts.is_empty() {
            parts.pop();
        }
    }
    let joined = format!("{}{}", "/".repeat(initial), parts.join("/"));
    if joined.is_empty() {
        ".".into()
    } else {
        joined
    }
}

/// The words `translate_path` keeps: no drive, separator, `.` or `..`.
fn kept_words(path: &str) -> (Vec<String>, bool) {
    let path = path.split('#').next().unwrap_or("");
    let path = path.split('?').next().unwrap_or("");
    let path = pyurl::unquote(path);
    let trailing = path.ends_with('/');
    let normal = posix_normpath(&path);
    let words = normal
        .split('/')
        .filter(|w| !w.is_empty())
        .filter(|w| {
            let has_dir = if cfg!(windows) {
                w.contains('\\') || w.contains(':')
            } else {
                false
            };
            !has_dir && *w != "." && *w != ".."
        })
        .map(str::to_string)
        .collect();
    (words, trailing)
}

/// A resolved request target.
#[derive(Clone, Debug)]
enum Resolved {
    Disk(PathBuf, bool),
    Embedded(Vec<String>, bool),
}

impl Resolved {
    fn display(&self) -> String {
        match self {
            Resolved::Disk(path, trailing) => {
                let mut s = pyerr::path_str(path);
                if *trailing {
                    s.push('/');
                }
                s
            }
            Resolved::Embedded(words, trailing) => {
                let mut s = words.join("/");
                if *trailing {
                    s.push('/');
                }
                s
            }
        }
    }
}

struct Handler<'a> {
    viewer: &'a Viewer,
    data_dir: &'a Path,
    stream: &'a TcpStream,
    path: String,
    command: String,
    request_version: String,
    headers: Vec<(String, String)>,
    status: u16,
    shard_etag: Option<String>,
    buffer: Vec<u8>,
    /// A failed header write leaves the socket unusable, so no body follows.
    write_failed: bool,
}

impl Handler<'_> {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn send_response(&mut self, code: u16, message: Option<&str>) {
        self.status = code;
        if self.request_version != "HTTP/0.9" {
            let message = message.unwrap_or(status_text(code).0);
            self.buffer
                .extend(format!("HTTP/1.0 {code} {message}\r\n").bytes().map(latin1));
        }
        self.send_header("Server", SERVER_VERSION);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        self.send_header("Date", &http_date(now));
    }

    fn send_header(&mut self, key: &str, value: &str) {
        if self.request_version != "HTTP/0.9" {
            self.buffer.extend(
                format!("{key}: {value}\r\n")
                    .chars()
                    .map(|c| c as u32 as u8),
            );
        }
    }

    fn end_headers(&mut self) {
        let cache_ok = matches!(self.status, 200 | 304);
        if self.path.starts_with("/data/thumbs/") && cache_ok {
            self.send_header("Cache-Control", "public, max-age=86400");
        } else if let Some(etag) = self.shard_etag.clone().filter(|_| cache_ok) {
            self.send_header("Cache-Control", "no-cache");
            self.send_header("ETag", &etag);
        } else {
            self.send_header("Cache-Control", "no-store");
        }
        self.send_header("Content-Security-Policy", CSP);
        self.send_header("X-Content-Type-Options", "nosniff");
        if self.request_version != "HTTP/0.9" {
            self.buffer.extend_from_slice(b"\r\n");
            let data = std::mem::take(&mut self.buffer);
            self.write_failed = write_by_deadline(self.stream, &data).is_err();
        }
    }

    fn send_error(&mut self, code: u16, message: Option<&str>, explain: Option<&str>) {
        let (short, long) = status_text(code);
        let message = message.unwrap_or(short).to_string();
        let explain = explain.unwrap_or(long).to_string();
        self.send_response(code, Some(&message));
        self.send_header("Connection", "close");
        // Every code sent here is 400 or above, so each error has a body.
        let escape = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        };
        let content = ERROR_TEMPLATE
            .replace("%(code)d", &code.to_string())
            .replace("%(code)s", &code.to_string())
            .replace("%(message)s", &escape(&message))
            .replace("%(explain)s", &escape(&explain));
        self.send_header("Content-Type", "text/html;charset=utf-8");
        self.send_header("Content-Length", &content.len().to_string());
        self.end_headers();
        self.write_body(content.as_bytes());
    }

    fn translate_path(&self, path: &str) -> Resolved {
        let clean = pyurl::unquote(
            path.split('?')
                .next()
                .unwrap_or("")
                .split('#')
                .next()
                .unwrap_or(""),
        );
        let refused = || Resolved::Disk(self.data_dir.join(REFUSED), false);
        if clean.contains('\0') {
            return refused();
        }
        if let Some(relative) = clean.strip_prefix("/data/") {
            let candidate = pyerr::resolve(&self.data_dir.join(relative));
            if candidate.starts_with(pyerr::resolve(self.data_dir)) {
                return Resolved::Disk(candidate, false);
            }
            return refused();
        }
        let (words, trailing) = kept_words(path);
        match self.viewer {
            Viewer::Dir(dir) => {
                let mut target = dir.clone();
                for word in &words {
                    target.push(word);
                }
                Resolved::Disk(target, trailing)
            }
            Viewer::Embedded => Resolved::Embedded(words, trailing),
        }
    }

    fn write_body(&mut self, body: &[u8]) {
        if self.command != "HEAD" && !self.write_failed {
            let _ = write_by_deadline(self.stream, body);
        }
    }

    /// DiogenesLamp's override: shards are served with a content ETag.
    fn send_head(&mut self) {
        self.shard_etag = None;
        let request_path = pyurl::unquote(self.path.split('?').next().unwrap_or(""));
        if request_path.starts_with("/data/catalog/")
            && request_path.ends_with(".json")
            && let Resolved::Disk(target, _) = self.translate_path(&self.path)
        {
            let shard = pyerr::resolve(&target);
            if shard.starts_with(pyerr::resolve(&self.data_dir.join("catalog")))
                && let Ok(body) = std::fs::read(&shard)
            {
                let digest = Sha256::digest(&body);
                let etag = format!(
                    "\"{}\"",
                    digest
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                );
                self.shard_etag = Some(etag.clone());
                let validators = self.header("If-None-Match").unwrap_or("").to_string();
                let matched = validators.split(',').any(|v| {
                    let v = pystr::strip(v);
                    let v = v.strip_prefix("W/").unwrap_or(v);
                    v == "*" || v == etag
                });
                if matched {
                    self.send_response(304, None);
                    self.end_headers();
                    return;
                }
                self.send_response(200, None);
                self.send_header("Content-Type", "application/json");
                self.send_header("Content-Length", &body.len().to_string());
                self.end_headers();
                self.write_body(&body);
                return;
            }
        }
        self.simple_send_head();
    }

    /// `SimpleHTTPRequestHandler.send_head`.
    fn simple_send_head(&mut self) {
        let path = self.path.clone();
        let resolved = self.translate_path(&path);
        let is_dir = match &resolved {
            Resolved::Disk(p, _) => p.is_dir(),
            Resolved::Embedded(words, _) => words.is_empty(),
        };
        let mut resolved = resolved;
        if is_dir {
            let Ok(parts) = pyurl::urlsplit(&self.path) else {
                return;
            };
            if !(parts.path.ends_with('/')
                || parts.path.ends_with("%2f")
                || parts.path.ends_with("%2F"))
            {
                self.send_response(301, None);
                let location = pyurl::urlunsplit(
                    &parts.scheme,
                    parts.netloc_str(),
                    &format!("{}/", parts.path),
                    parts.query_str(),
                    parts.fragment.as_deref().unwrap_or(""),
                );
                self.send_header("Location", &location);
                self.send_header("Content-Length", "0");
                self.end_headers();
                return;
            }
            let index = match &resolved {
                Resolved::Disk(p, _) => ["index.html", "index.htm"]
                    .iter()
                    .map(|n| p.join(n))
                    .find(|p| p.is_file())
                    .map(|p| Resolved::Disk(p, false)),
                Resolved::Embedded(..) => viewer::get("index.html")
                    .map(|_| Resolved::Embedded(vec!["index.html".into()], false)),
            };
            match index {
                Some(index) => resolved = index,
                None => {
                    self.send_error(404, Some("No permission to list directory"), None);
                    return;
                }
            }
        }
        let display = resolved.display();
        let ctype = guess_type(&display);
        if display.ends_with('/') {
            self.send_error(404, Some("File not found"), None);
            return;
        }
        match resolved {
            Resolved::Embedded(words, _) => {
                let data = (words.len() == 1).then(|| viewer::get(&words[0])).flatten();
                let Some(data) = data else {
                    self.send_error(404, Some("File not found"), None);
                    return;
                };
                self.send_response(200, None);
                self.send_header("Content-type", ctype);
                self.send_header("Content-Length", &data.len().to_string());
                self.end_headers();
                self.write_body(data);
            }
            Resolved::Disk(path, _) => {
                // Python cannot open a directory; Rust can on Unix.
                let opened = (!path.is_dir())
                    .then(|| std::fs::File::open(&path).ok())
                    .flatten();
                let Some(mut file) = opened else {
                    self.send_error(404, Some("File not found"), None);
                    return;
                };
                let meta = file.metadata().ok();
                let mtime = meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs() as i64);
                if self.header("If-Modified-Since").is_some()
                    && self.header("If-None-Match").is_none()
                    && let Some(ims) = self.header("If-Modified-Since").and_then(parse_http_date)
                    && mtime <= ims
                {
                    self.send_response(304, None);
                    self.end_headers();
                    return;
                }
                let mut data = Vec::new();
                if file.read_to_end(&mut data).is_err() {
                    return;
                }
                self.send_response(200, None);
                self.send_header("Content-type", ctype);
                self.send_header(
                    "Content-Length",
                    &meta.map_or(data.len() as u64, |m| m.len()).to_string(),
                );
                self.send_header("Last-Modified", &http_date(mtime));
                self.end_headers();
                self.write_body(&data);
            }
        }
    }
}

fn latin1(b: u8) -> u8 {
    b
}

fn handle(stream: &TcpStream, viewer: &Viewer, data_dir: &Path, port: u16) {
    // The request line and headers share one deadline: the per-read timeout
    // alone restarts on every byte.
    let mut reader = BufReader::new(Deadline {
        stream,
        deadline: Instant::now() + SOCKET_READ_TIMEOUT,
    });
    let mut raw = Vec::new();
    if reader
        .by_ref()
        .take(65537)
        .read_until(b'\n', &mut raw)
        .is_err()
    {
        return;
    }
    let mut handler = Handler {
        viewer,
        data_dir,
        stream,
        path: String::new(),
        command: String::new(),
        request_version: "HTTP/0.9".into(),
        headers: Vec::new(),
        status: 0,
        shard_etag: None,
        buffer: Vec::new(),
        write_failed: false,
    };
    // Python's end_headers override reads self.path, which is unset until
    // the request line parses: those error replies die before a byte is
    // sent, and the connection just closes.
    if raw.len() > 65536 {
        return;
    }
    if raw.is_empty() {
        return;
    }
    let line: String = raw.iter().map(|&b| b as char).collect();
    let line = line.trim_end_matches(['\r', '\n']);
    let words: Vec<&str> = line
        .split(pystr::is_space)
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return;
    }
    if words.len() >= 3 {
        let version = words[words.len() - 1];
        let parsed = version.strip_prefix("HTTP/").and_then(|v| {
            let parts: Vec<&str> = v.split('.').collect();
            if parts.len() != 2
                || parts
                    .iter()
                    .any(|p| !pystr::all_digits(p) || p.chars().count() > 10)
            {
                return None;
            }
            Some((
                pystr::parse_int(parts[0])?,
                pystr::parse_int(parts[1])?,
                v.to_string(),
            ))
        });
        let Some((major, _minor, _base)) = parsed else {
            return;
        };
        handler.request_version = version.to_string();
        if major >= 2 {
            return;
        }
    }
    if !(2..=3).contains(&words.len()) {
        return;
    }
    let (command, path) = (words[0].to_string(), words[1].to_string());
    if words.len() == 2 && command != "GET" {
        return;
    }
    handler.command = command;
    handler.path = if path.starts_with("//") {
        format!("/{}", path.trim_start_matches('/'))
    } else {
        path
    };
    if words.len() == 3 {
        let mut count = 0;
        loop {
            let mut header_line = Vec::new();
            if reader
                .by_ref()
                .take(65537)
                .read_until(b'\n', &mut header_line)
                .is_err()
            {
                return;
            }
            if header_line.len() > 65536 {
                handler.send_error(
                    431,
                    Some("Line too long"),
                    Some("got more than 65536 bytes when reading header line"),
                );
                return;
            }
            if header_line.is_empty() || header_line == b"\r\n" || header_line == b"\n" {
                break;
            }
            count += 1;
            if count > 100 {
                handler.send_error(
                    431,
                    Some("Too many headers"),
                    Some("got more than 100 headers"),
                );
                return;
            }
            let text: String = header_line.iter().map(|&b| b as char).collect();
            if let Some((name, value)) = text.split_once(':') {
                handler
                    .headers
                    .push((name.trim().to_string(), value.trim().to_string()));
            }
        }
        let hosts: Vec<&str> = handler
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
            .map(|(_, v)| v.as_str())
            .collect();
        if hosts.len() > 1 || (hosts.is_empty() && handler.request_version == "HTTP/1.1") {
            handler.send_error(400, Some("Bad Request"), Some("Invalid Host header"));
            let _ = handler.stream.flush();
            return;
        }
        if let Some(&host) = hosts.first()
            && !valid_host_header(host, port)
        {
            handler.send_error(403, Some("Forbidden"), Some("Host header is not allowed"));
            let _ = handler.stream.flush();
            return;
        }
    }
    match handler.command.as_str() {
        "GET" | "HEAD" => handler.send_head(),
        other => {
            let message = format!("Unsupported method ({})", pystr::repr(other));
            handler.send_error(501, Some(&message), None);
        }
    }
    let _ = handler.stream.flush();
}

pub struct Server {
    listener: TcpListener,
    viewer: Viewer,
    data_dir: PathBuf,
    shutdown: Arc<AtomicBool>,
}

/// Bind 127.0.0.1:port (0 picks a free port).
pub fn make_server(viewer: Viewer, data_dir: &Path, port: u16) -> std::io::Result<Server> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    listener.set_nonblocking(true)?;
    Ok(Server {
        listener,
        viewer,
        data_dir: data_dir.to_path_buf(),
        shutdown: Arc::new(AtomicBool::new(false)),
    })
}

impl Server {
    pub fn port(&self) -> u16 {
        self.listener.local_addr().map_or(0, |a| a.port())
    }

    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        self.shutdown.clone()
    }

    /// Serve until the shutdown flag is set; each request on its own thread.
    pub fn serve_forever(&self) {
        while !self.shutdown.load(Ordering::SeqCst) {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    configure_stream(&stream);
                    let port = self.port();
                    let viewer = self.viewer.clone();
                    let data_dir = self.data_dir.clone();
                    std::thread::spawn(move || {
                        handle(&stream, &viewer, &data_dir, port);
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                    });
                }
                // Nothing waiting (WouldBlock), or a failed accept: try again.
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}

/// The program and leading arguments that open a URL in the default browser.
#[cfg(windows)]
const OPENER: (&str, &[&str]) = ("rundll32", &["url.dll,FileProtocolHandler"]);
#[cfg(target_os = "macos")]
const OPENER: (&str, &[&str]) = ("open", &[]);
#[cfg(all(unix, not(target_os = "macos")))]
const OPENER: (&str, &[&str]) = ("xdg-open", &[]);

/// Open the default browser at `url`, as `webbrowser.open` does.
fn open_browser(url: &str) {
    let _ = std::process::Command::new(OPENER.0)
        .args(OPENER.1)
        .arg(url)
        .spawn();
}

/// `lamp serve`: print the URL, optionally open a browser, serve until Ctrl+C.
pub fn serve(server: &Server, open: bool, stop: Arc<AtomicBool>) {
    let url = format!("http://127.0.0.1:{}/", server.port());
    crate::out::out_flush(&format!("serving on {url} (Ctrl+C to stop)"))
        .unwrap_or_else(|e| crate::out::uncaught(&e));
    if open {
        open_browser(&url);
    }
    let flag = server.shutdown_handle();
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(50));
        }
        flag.store(true, Ordering::SeqCst);
    });
    server.serve_forever();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_types() {
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(
            parse_http_date("Fri, 01 Jan 2100 00:00:00 GMT"),
            Some(4_102_444_800)
        );
        assert_eq!(guess_type("x/app.js"), "text/javascript");
        assert_eq!(guess_type("x/a.json"), "application/json");
        assert_eq!(guess_type("x/1.jpg"), "image/jpeg");
        assert_eq!(guess_type("x/noext"), "application/octet-stream");
        assert_eq!(posix_normpath("/data/../x"), "/x");
        assert_eq!(posix_normpath("/../x"), "/x");
    }

    // Files lamp never writes are served as bytes, compressed ones included.
    #[test]
    fn other_names_are_octet_streams() {
        for path in [
            "x/a.tgz",
            "x/a.js.br",
            "x/a.tar.gz",
            "x/.json",
            "x/a.",
            "x.d/noext",
            "x/..json",
            "x/y\\.html",
        ] {
            assert_eq!(guess_type(path), "application/octet-stream", "{path}");
        }
        assert_eq!(guess_type("x/A.JPEG"), "image/jpeg");
    }

    // What CPython 3.15's posixpath.normpath returns.
    #[test]
    fn normpath_matches_posixpath() {
        for (path, expected) in [
            ("", "."),
            ("a/..", "."),
            ("//a//b", "//a/b"),
            ("///a", "/a"),
            ("/a/./b/", "/a/b"),
        ] {
            assert_eq!(posix_normpath(path), expected, "{path}");
        }
    }

    const NOV_6_1994: i64 = 784_111_777;

    // RFC 9110 section 5.6.7: a recipient must accept all 3 HTTP-date formats.
    #[test]
    fn rfc850_dates_are_read() {
        assert_eq!(
            parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT"),
            Some(NOV_6_1994)
        );
    }

    #[test]
    fn asctime_dates_are_read() {
        assert_eq!(
            parse_http_date("Sun Nov  6 08:49:37 1994"),
            Some(NOV_6_1994)
        );
    }

    // Section 13.1.3: a value that is not a valid HTTP-date is ignored.
    #[test]
    fn times_out_of_range_are_ignored() {
        for text in [
            "Sun, 06 Nov 1994 25:49:37 GMT",
            "Sun, 06 Nov 1994 24:00:00 GMT",
            "Sun, 06 Nov 1994 08:60:37 GMT",
            "Sun, 06 Nov 1994 08:49:60 GMT",
        ] {
            assert_eq!(parse_http_date(text), None, "{text}");
        }
    }

    // What CPython 3.15's http.server compares a file's mtime against.
    #[test]
    fn other_forms_match_http_server() {
        for (text, expected) in [
            ("Sun, 06 Nov 1994 08:49:37 GMT", Some(NOV_6_1994)),
            ("Sunday, 06 Nov 1994 08:49:37 GMT", Some(NOV_6_1994)),
            ("06 Nov 1994 08:49:37 GMT", Some(NOV_6_1994)),
            ("06 Nov 1994 08:49:37", Some(NOV_6_1994)),
            ("06 Nov 1994 08:49", Some(784_111_740)),
            ("Sun, 06 Nov 94 08:49:37 GMT", Some(NOV_6_1994)),
            ("Sun, 06 Nov 69 08:49:37 GMT", Some(-4_806_623)),
            ("Sun, 06 Nov 68 08:49:37 GMT", Some(3_119_417_377)),
            ("Sun, Nov 06 1994 08:49:37 GMT", Some(NOV_6_1994)),
            ("Sun, 06 november 1994 08:49:37 UT", Some(NOV_6_1994)),
            ("Sun, 06 Nov 1994 08:49:37 UTC", Some(NOV_6_1994)),
            ("Sun, 06 Nov 1994 08:49:37 Z", Some(NOV_6_1994)),
            ("Sun, 06 Nov 1994 08:49:37 +0000", Some(NOV_6_1994)),
            ("Sun, 06 Nov 1994 08:49:37 -0000", Some(NOV_6_1994)),
            ("Sun, 06 Nov 1994 08:49:37 gmt", Some(NOV_6_1994)),
            ("Sun, 06 Nov 1994 08:49:37 +0100", None),
            ("Sun, 06 Nov 1994 08:49:37 EST", None),
            ("Sun, 06 Nov 1994", None),
            ("Sun, 06 Nov 1994 08:49:x GMT", None),
            ("Sun, 06 Nov 1994 08 GMT", None),
            ("Sun, 06 Nov 1994 08:-1:37 GMT", None),
            ("Sun, 06 Foo 1994 08:49:37 GMT", None),
            ("Sun, xx Nov 1994 08:49:37 GMT", None),
            ("Sun, 31 Feb 1994 08:49:37 GMT", None),
            ("garbage", None),
            ("", None),
        ] {
            assert_eq!(parse_http_date(text), expected, "{text}");
        }
    }

    #[test]
    fn host_header_allowlist_and_stream_timeouts() {
        for valid in [
            "127.0.0.1",
            "127.0.0.1:8080",
            "localhost",
            "LOCALHOST",
            "LocalHost:8080",
            "[::1]",
            "[::1]:8080",
        ] {
            assert!(valid_host_header(valid, 8080), "{valid}");
        }
        for invalid in [
            "",
            "evil.example",
            "evil.example:8080",
            "localhost.evil.example:8080",
            "127.0.0.2:8080",
            "127.0.0.1:9090",
            "127.0.0.1:",
            "localhost:08080",
            "[::1]:9090",
            "[::1]extra",
            "[::1",
            "[::2]:8080",
            "user@localhost:8080",
        ] {
            assert!(!valid_host_header(invalid, 8080), "{invalid}");
        }
        let (server_stream, client) = connected_pair();
        configure_stream(&server_stream);
        assert_eq!(
            server_stream.read_timeout().unwrap(),
            Some(SOCKET_READ_TIMEOUT)
        );
        assert_eq!(
            server_stream.write_timeout().unwrap(),
            Some(SOCKET_WRITE_TIMEOUT)
        );
        drop(client);
    }

    fn connected_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (server, client)
    }

    #[test]
    fn a_response_gets_time_in_proportion_to_its_size() {
        let rate = MIN_WRITE_RATE as usize;
        assert_eq!(write_allowance(0), SOCKET_WRITE_TIMEOUT);
        assert_eq!(write_allowance(rate - 1), SOCKET_WRITE_TIMEOUT);
        assert_eq!(
            write_allowance(rate * 10 + 1),
            SOCKET_WRITE_TIMEOUT + Duration::from_secs(10)
        );
        // A shard at the 256 MiB snapshot limit has over an hour.
        assert_eq!(
            write_allowance(256 << 20),
            SOCKET_WRITE_TIMEOUT + Duration::from_secs(4096)
        );
    }

    #[test]
    fn a_reader_trickling_a_response_is_cut_off_at_the_deadline() {
        let (server, mut client) = connected_pair();
        let done = Arc::new(AtomicBool::new(false));
        let reading = Arc::clone(&done);
        // Slow but steady: every read makes room for more, so each write call
        // progresses and only a deadline on the whole response stops it.
        let reader = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let started = Instant::now();
            while !reading.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(8) {
                if matches!(client.read(&mut buf), Ok(0) | Err(_)) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let started = Instant::now();
        let written = Deadline {
            stream: &server,
            deadline: started + Duration::from_secs(1),
        }
        .write_all(&vec![0u8; 64 << 20]);
        let held = started.elapsed();
        done.store(true, Ordering::SeqCst);
        drop(server);
        reader.join().unwrap();
        let kind = written.unwrap_err().kind();
        assert!(
            matches!(
                kind,
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ),
            "{kind:?}"
        );
        assert!(
            held >= Duration::from_millis(900) && held < Duration::from_secs(4),
            "the write was stopped after {held:?}, not at the 1 s deadline"
        );
    }

    #[test]
    fn a_reader_keeping_up_gets_the_whole_response() {
        let (server, mut client) = connected_pair();
        let data: Vec<u8> = (0..8u32 << 20).map(|i| i as u8).collect();
        let reader = std::thread::spawn(move || {
            let mut got = Vec::new();
            client.read_to_end(&mut got).unwrap();
            got
        });
        write_by_deadline(&server, &data).unwrap();
        drop(server);
        assert!(
            reader.join().unwrap() == data,
            "the response arrived changed"
        );
    }

    #[test]
    fn a_passed_deadline_writes_nothing() {
        let (server, mut client) = connected_pair();
        let written = Deadline {
            stream: &server,
            deadline: Instant::now(),
        }
        .write(b"late");
        assert_eq!(written.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        drop(server);
        let mut got = Vec::new();
        client.read_to_end(&mut got).unwrap();
        assert!(got.is_empty(), "{got:?}");
    }

    // A client that stops reading is still dropped after the old 30 s of no
    // progress, however long the whole response may take.
    #[test]
    fn each_write_call_still_stalls_out_at_the_socket_timeout() {
        let (server, _client) = connected_pair();
        Deadline {
            stream: &server,
            deadline: Instant::now() + Duration::from_secs(3600),
        }
        .write_all(b"ok")
        .unwrap();
        let timeout = server.write_timeout().unwrap().unwrap();
        assert!(timeout <= SOCKET_WRITE_TIMEOUT, "{timeout:?}");
    }

    // Responses go through the deadline: it replaces whatever write timeout
    // the socket had with at most the per-call 30 s.
    #[test]
    fn responses_are_written_by_the_deadline() {
        let (server, mut client) = connected_pair();
        server
            .set_write_timeout(Some(Duration::from_secs(3600)))
            .unwrap();
        let port = server.local_addr().unwrap().port();
        let request = format!("GET /nope HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        client.write_all(request.as_bytes()).unwrap();
        handle(&server, &Viewer::Embedded, Path::new("."), port);
        let timeout = server.write_timeout().unwrap().unwrap();
        assert!(timeout <= SOCKET_WRITE_TIMEOUT, "{timeout:?}");
        drop(server);
        let mut reply = String::new();
        client.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.0 404"), "{reply}");
    }
}
