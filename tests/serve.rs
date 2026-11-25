// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Ported from tests/test_serve.py and tests/wheel_smoke.py.
mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use common::write;
use lamp::serve::{REFUSED, Server, Viewer, make_server};

struct Running {
    port: u16,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn start(viewer: Viewer, data: &Path) -> Running {
    let server: Server = make_server(viewer, data, 0).unwrap();
    let port = server.port();
    let stop = server.shutdown_handle();
    let thread = std::thread::spawn(move || server.serve_forever());
    Running {
        port,
        stop,
        thread: Some(thread),
    }
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn request(port: u16, method: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut raw = format!("{method} {path} HTTP/1.0\r\n");
    for (k, v) in headers {
        raw.push_str(&format!("{k}: {v}\r\n"));
    }
    raw.push_str("\r\n");
    stream.write_all(raw.as_bytes()).unwrap();
    let mut data = Vec::new();
    stream.read_to_end(&mut data).unwrap();
    let split = data
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(data.len());
    let head = String::from_utf8_lossy(&data[..split]).into_owned();
    let body = data.get(split + 4..).unwrap_or_default().to_vec();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap_or("")
        .split(' ')
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let headers = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Reply {
        status,
        headers,
        body,
    }
}

fn get(port: u16, path: &str) -> Reply {
    request(port, "GET", path, &[])
}

fn layout(dir: &Path) -> (PathBuf, PathBuf) {
    let viewer = dir.join("viewer");
    let data = dir.join("data");
    write(&viewer.join("index.html"), "<h1>lamp</h1>");
    write(&data.join("index.json"), r#"{"generated_at": "x"}"#);
    (viewer, data)
}

#[test]
fn serves_viewer_and_data() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let server = start(Viewer::Dir(viewer), &data);
    let root = get(server.port, "/");
    assert!(root.status == 200 && String::from_utf8_lossy(&root.body).contains("lamp"));
    let index = get(server.port, "/data/index.json");
    assert!(index.status == 200 && String::from_utf8_lossy(&index.body).contains("generated_at"));
}

#[test]
fn data_path_traversal_is_refused() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    write(&dir.path().join("outside.txt"), "must never be served");
    let server = start(Viewer::Dir(viewer), &data);
    for attempt in [
        "/data/../outside.txt",
        "/data/..%2foutside.txt",
        "/data/%2e%2e/outside.txt",
        "/data/sub/../../outside.txt",
        "/data/....//outside.txt",
        "/data/%252e%252e/outside.txt",
        "/%252e%252e/outside.txt",
    ] {
        let reply = get(server.port, attempt);
        assert_ne!(reply.status, 200, "{attempt}");
        assert!(
            !String::from_utf8_lossy(&reply.body).contains("must never be served"),
            "{attempt}"
        );
    }
    assert_eq!(get(server.port, "/data/index.json").status, 200);
}

#[test]
fn cache_headers_for_thumbs_only_on_success_and_on_revalidation() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    write(&data.join("thumbs/seller1/1.jpg"), b"jpg");
    let server = start(Viewer::Dir(viewer), &data);
    let thumb = get(server.port, "/data/thumbs/seller1/1.jpg?v=x");
    assert_eq!(
        (thumb.status, thumb.header("Cache-Control")),
        (200, Some("public, max-age=86400"))
    );
    let missing = get(server.port, "/data/thumbs/seller1/2.jpg");
    assert_eq!(
        (missing.status, missing.header("Cache-Control")),
        (404, Some("no-store"))
    );
    let index = get(server.port, "/data/index.json");
    assert_eq!(
        (index.status, index.header("Cache-Control")),
        (200, Some("no-store"))
    );
    let modified = thumb.header("Last-Modified").unwrap().to_string();
    let again = request(
        server.port,
        "GET",
        "/data/thumbs/seller1/1.jpg",
        &[("If-Modified-Since", &modified)],
    );
    assert_eq!(
        (again.status, again.header("Cache-Control")),
        (304, Some("public, max-age=86400"))
    );
}

#[test]
fn catalogue_shards_revalidate_by_content() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let shard = data.join("catalog/seller.json");
    write(&shard, br#"{"value":1}"#);
    let mtime = std::fs::metadata(&shard).unwrap().modified().unwrap();
    let server = start(Viewer::Dir(viewer), &data);
    let first = get(server.port, "/data/catalog/seller.json");
    assert_eq!(
        (first.status, first.body.as_slice()),
        (200, &br#"{"value":1}"#[..])
    );
    assert_eq!(first.header("Cache-Control"), Some("no-cache"));
    assert_eq!(first.header("Content-Type"), Some("application/json"));
    let etag = first.header("ETag").unwrap().to_string();
    let head = request(server.port, "HEAD", "/data/catalog/seller.json", &[]);
    assert!(
        head.status == 200 && head.body.is_empty() && head.header("ETag") == Some(etag.as_str())
    );
    for validator in [etag.clone(), format!("\"unrelated\", W/{etag}"), "*".into()] {
        let reply = request(
            server.port,
            "GET",
            "/data/catalog/seller.json",
            &[("If-None-Match", &validator)],
        );
        assert!(reply.status == 304 && reply.body.is_empty());
        assert_eq!(reply.header("Cache-Control"), Some("no-cache"));
        assert_eq!(reply.header("ETag"), Some(etag.as_str()));
    }
    write(&shard, br#"{"value":2}"#);
    std::fs::File::options()
        .write(true)
        .open(&shard)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let fresh = request(
        server.port,
        "GET",
        "/data/catalog/seller.json",
        &[
            ("If-None-Match", &etag),
            ("If-Modified-Since", "Fri, 01 Jan 2100 00:00:00 GMT"),
        ],
    );
    assert_eq!(
        (fresh.status, fresh.body.as_slice()),
        (200, &br#"{"value":2}"#[..])
    );
    assert_ne!(fresh.header("ETag"), Some(etag.as_str()));
    let missing = request(
        server.port,
        "GET",
        "/data/catalog/missing.json",
        &[("If-None-Match", "*")],
    );
    assert!(
        missing.status == 404
            && missing.header("Cache-Control") == Some("no-store")
            && missing.header("ETag").is_none()
    );
    let escaped = get(server.port, "/data/catalog/../index.json");
    assert!(
        escaped.status == 200
            && escaped.header("Cache-Control") == Some("no-store")
            && escaped.header("ETag").is_none()
    );
    let post = request(server.port, "POST", "/data/catalog/seller.json", &[]);
    assert!(
        post.status == 501
            && post.header("Cache-Control") == Some("no-store")
            && post.header("ETag").is_none()
    );
}

#[test]
fn directory_listings_and_null_bytes_get_404() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    write(&data.join("catalog/sellerx.json"), "{}");
    let server = start(Viewer::Dir(viewer), &data);
    for path in [
        "/data/catalog/",
        "/data/",
        "/data/....//x",
        "/data/%00index.json",
        "/%00index.html",
    ] {
        assert_eq!(get(server.port, path).status, 404, "{path}");
    }
    assert_eq!(get(server.port, "/data/catalog/sellerx.json").status, 200);
    assert_eq!(get(server.port, "/").status, 200);
}

#[test]
fn security_headers_on_every_response() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let server = start(Viewer::Dir(viewer), &data);
    for path in ["/", "/data/index.json", "/missing"] {
        let reply = get(server.port, path);
        let csp = reply.header("Content-Security-Policy").unwrap_or("");
        assert!(
            csp.contains("default-src 'none'")
                && csp.contains("script-src 'self'")
                && csp.contains("manifest-src 'self'"),
            "{path}: {csp}"
        );
        assert_eq!(reply.header("X-Content-Type-Options"), Some("nosniff"));
    }
}

#[test]
fn the_viewer_declares_and_serves_the_pwa_manifest() {
    let html = common::read(&common::root().join("viewer/index.html"));
    assert!(html.contains("rel=\"manifest\"") && html.contains("href=\"manifest.json\""));
    let manifest = common::read_json(&common::root().join("viewer/manifest.json"));
    assert_eq!(common::obj(&manifest)["name"].py_str(), "DiogenesLamp");
    assert_eq!(common::obj(&manifest)["start_url"].py_str(), "./");
    let dir = common::tmp();
    let data = dir.path().join("data");
    write(&data.join("index.json"), "{}");
    let server = start(Viewer::Embedded, &data);
    let reply = get(server.port, "/manifest.json");
    assert_eq!(reply.status, 200);
    assert_eq!(
        common::obj(&common::json(&String::from_utf8_lossy(&reply.body)))["name"].py_str(),
        "DiogenesLamp"
    );
    assert!(
        reply
            .header("Content-Security-Policy")
            .unwrap()
            .contains("manifest-src 'self'")
    );
}

/// Every kind of file lamp writes under its data directory or embeds as
/// the viewer is served with the Content-Type a browser needs for it.
#[test]
fn every_file_lamp_writes_is_served_with_its_content_type() {
    let dir = common::tmp();
    let data = dir.path().join("data");
    let files = [
        ("index.json", "application/json"),
        ("aliases.json", "application/json"),
        ("notify-pending.json", "application/json"),
        ("catalog/s.json", "application/json"),
        ("runs/s.json", "application/json"),
        ("thumbs/s/1.jpg", "image/jpeg"),
        ("thumbs/s/2.png", "image/png"),
        ("thumbs/s/3.webp", "image/webp"),
        ("debug/s-page1.html", "text/html"),
        ("snapshots/20261122T000000-0123abcd.zip", "application/zip"),
        (".lamp.lock", "application/octet-stream"),
    ];
    for (name, _) in files {
        write(&data.join(name), "x");
    }
    let server = start(Viewer::Embedded, &data);
    for (name, expected) in files {
        let reply = get(server.port, &format!("/data/{name}"));
        assert_eq!(reply.status, 200, "{name}");
        assert_eq!(reply.header("Content-Type"), Some(expected), "{name}");
    }
    for (name, _) in lamp::viewer::FILES {
        let expected = match name.rsplit_once('.').map(|(_, ext)| ext) {
            Some("html") => "text/html",
            Some("css") => "text/css",
            Some("js") => "text/javascript",
            Some("json") => "application/json",
            other => panic!("{name}: no expected type for {other:?}"),
        };
        let reply = get(server.port, &format!("/{name}"));
        assert_eq!(reply.status, 200, "{name}");
        assert_eq!(reply.header("Content-Type"), Some(expected), "{name}");
    }
}

#[test]
fn the_refusal_sentinel_is_a_name_nothing_ever_writes() {
    for module in ["scrape.rs", "storage.rs"] {
        let source = common::read(&common::root().join("src").join(module));
        assert!(!source.contains(REFUSED), "{module} writes the sentinel");
    }
}

/// The wheel smoke check, for the binary: every asset index.html loads is
/// embedded and served from outside any source checkout.
#[test]
fn every_viewer_asset_is_embedded_and_served() {
    let html = common::read(&common::root().join("viewer/index.html"));
    let mut paths = vec!["index.html".to_string()];
    let re = regex::Regex::new(r#"<(script|link)\b([^>]*)>"#).unwrap();
    let attr = |tag: &str, name: &str| -> Option<String> {
        regex::Regex::new(&format!(r#"\b{name}="([^"]*)""#))
            .unwrap()
            .captures(tag)
            .map(|c| c[1].to_string())
    };
    for caps in re.captures_iter(&html) {
        let tag = &caps[2];
        let path = if &caps[1] == "script" {
            attr(tag, "src")
        } else if attr(tag, "rel").is_some_and(|r| {
            r.to_lowercase()
                .split_whitespace()
                .any(|x| x == "stylesheet" || x == "manifest")
        }) {
            attr(tag, "href")
        } else {
            None
        };
        if let Some(path) = path
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    }
    assert!(paths.len() > 5);
    let dir = common::tmp();
    let server = start(Viewer::Embedded, &dir.path().join("empty-data"));
    for path in &paths {
        let reply = get(server.port, &format!("/{path}"));
        assert!(reply.status == 200 && !reply.body.is_empty(), "{path}");
    }
}

#[test]
fn the_embedded_viewer_matches_the_viewer_directory() {
    let mut on_disk: Vec<String> = std::fs::read_dir(common::root().join("viewer"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| {
            [".html", ".css", ".js", ".json"]
                .iter()
                .any(|s| n.ends_with(s))
        })
        .collect();
    on_disk.sort();
    let embedded: Vec<String> = lamp::viewer::FILES
        .iter()
        .map(|(n, _)| n.to_string())
        .collect();
    assert_eq!(embedded, on_disk);
    for (name, data) in lamp::viewer::FILES {
        assert_eq!(
            std::fs::read(common::root().join("viewer").join(name)).unwrap(),
            *data,
            "{name}"
        );
    }
}

/// Sends `raw` unchanged, closes the writing half and returns every byte the
/// server sent back.
fn exchange(port: u16, raw: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.write_all(raw).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut data = Vec::new();
    stream.read_to_end(&mut data).unwrap();
    data
}

#[test]
fn a_directory_without_its_slash_is_redirected_with_its_query() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    write(&viewer.join("sub/index.html"), "<p>sub</p>");
    let server = start(Viewer::Dir(viewer), &data);
    let moved = get(server.port, "/sub?x=1");
    assert_eq!(
        (
            moved.status,
            moved.header("Location"),
            moved.header("Content-Length")
        ),
        (301, Some("/sub/?x=1"), Some("0"))
    );
    assert!(moved.body.is_empty());
    for path in ["/sub/", "/sub%2F", "/sub%2f"] {
        let reply = get(server.port, path);
        assert_eq!(
            (reply.status, reply.body.as_slice()),
            (200, &b"<p>sub</p>"[..]),
            "{path}"
        );
    }
}

/// `//host` would make the redirect's Location a link to another site.
#[test]
fn a_leading_double_slash_cannot_redirect_to_another_host() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    write(&viewer.join("evil.example/index.html"), "x");
    let server = start(Viewer::Dir(viewer), &data);
    for path in ["//evil.example", "///evil.example"] {
        let reply = get(server.port, path);
        assert_eq!(
            (reply.status, reply.header("Location")),
            (301, Some("/evil.example/")),
            "{path}"
        );
    }
    assert_eq!(get(server.port, "//data/index.json").status, 200);
}

#[test]
fn a_file_asked_for_as_a_directory_is_not_found() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let on_disk = start(Viewer::Dir(viewer), &data);
    assert_eq!(get(on_disk.port, "/index.html").status, 200);
    assert_eq!(get(on_disk.port, "/index.html/").status, 404);
    let embedded = start(Viewer::Embedded, &data);
    assert_eq!(get(embedded.port, "/app.js").status, 200);
    for path in ["/app.js/", "/missing.js", "/nested/app.js"] {
        let reply = get(embedded.port, path);
        assert_eq!(
            (reply.status, reply.header("Cache-Control")),
            (404, Some("no-store")),
            "{path}"
        );
    }
}

#[test]
fn the_embedded_viewer_serves_its_index_at_the_root() {
    let dir = common::tmp();
    let server = start(Viewer::Embedded, &dir.path().join("empty-data"));
    let root = get(server.port, "/");
    assert_eq!(root.status, 200);
    assert_eq!(root.header("Content-type"), Some("text/html"));
    assert_eq!(root.body, lamp::viewer::get("index.html").unwrap());
}

#[test]
fn a_missing_shard_is_not_found_and_not_cached() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    write(&data.join("catalog/sellerx.json"), "{}");
    let server = start(Viewer::Dir(viewer), &data);
    let reply = get(server.port, "/data/catalog/sellery.json");
    assert_eq!(
        (
            reply.status,
            reply.header("Cache-Control"),
            reply.header("ETag")
        ),
        (404, Some("no-store"), None)
    );
}

/// An HTTP/0.9 request gets the bare body: no status line or headers.
#[test]
fn a_simple_request_gets_only_the_body() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let server = start(Viewer::Dir(viewer), &data);
    assert_eq!(
        exchange(server.port, b"GET /data/index.json\r\n"),
        br#"{"generated_at": "x"}"#
    );
}

#[test]
fn header_lines_over_64_kib_are_refused() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let server = start(Viewer::Dir(viewer), &data);
    // A header line of `len` bytes, counting its CRLF.
    let line = |len: usize| format!("X-Long: {}\r\n", "a".repeat(len - 10));
    let fits = exchange(
        server.port,
        format!("GET /data/index.json HTTP/1.0\r\n{}\r\n", line(65_536)).as_bytes(),
    );
    assert!(fits.starts_with(b"HTTP/1.0 200 OK\r\n"));
    // Nothing follows the long line, so the server reads every byte sent.
    let over = exchange(
        server.port,
        format!("GET /data/index.json HTTP/1.0\r\n{}", line(65_537)).as_bytes(),
    );
    let over = String::from_utf8_lossy(&over);
    assert!(over.starts_with("HTTP/1.0 431 Line too long\r\n"), "{over}");
    assert!(over.contains("got more than 65536 bytes when reading header line"));
    assert!(!over.contains("generated_at"));
}

#[test]
fn more_than_100_headers_are_refused() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let server = start(Viewer::Dir(viewer), &data);
    let headers = |n: usize| (0..n).map(|i| format!("X-{i}: v\r\n")).collect::<String>();
    let fits = exchange(
        server.port,
        format!("GET /data/index.json HTTP/1.0\r\n{}\r\n", headers(100)).as_bytes(),
    );
    assert!(fits.starts_with(b"HTTP/1.0 200 OK\r\n"));
    let over = exchange(
        server.port,
        format!("GET /data/index.json HTTP/1.0\r\n{}", headers(101)).as_bytes(),
    );
    let over = String::from_utf8_lossy(&over);
    assert!(
        over.starts_with("HTTP/1.0 431 Too many headers\r\n"),
        "{over}"
    );
    assert!(over.contains("got more than 100 headers"));
}

/// http.server's replies to these die before a byte is sent; the connection
/// closes and the server keeps serving.
#[test]
fn malformed_request_lines_close_without_a_reply() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let server = start(Viewer::Dir(viewer), &data);
    let long = vec![b'G'; 65_537];
    for raw in [
        &b""[..],
        b"   \r\n",
        &long,
        b"GET / HTTP/1\r\n",
        b"GET / FOO/1.0\r\n",
        b"GET / HTTP/2.0\r\n",
        b"GET / x HTTP/1.0\r\n",
        b"HEAD /\r\n",
        b"/\r\n",
    ] {
        let label = String::from_utf8_lossy(&raw[..raw.len().min(20)]).into_owned();
        assert_eq!(exchange(server.port, raw), b"", "{label}");
    }
    assert_eq!(get(server.port, "/data/index.json").status, 200);
}

/// A directory path that Python's urlsplit refuses (an unclosed `[` in what
/// reads as a host) ends the request without a reply, as its ValueError did.
#[test]
fn a_directory_path_urlsplit_refuses_closes_without_a_reply() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    // Windows drops the `x:` word, which names a drive there.
    let folder = if cfg!(windows) {
        viewer.join("[")
    } else {
        viewer.join("x:").join("[")
    };
    std::fs::create_dir_all(&folder).unwrap();
    let server = start(Viewer::Dir(viewer), &data);
    assert_eq!(exchange(server.port, b"GET x://[/ HTTP/1.0\r\n\r\n"), b"");
    assert_eq!(get(server.port, "/").status, 200);
}

/// A file that opens but cannot be read ends the request without a reply.
#[cfg(target_os = "linux")]
#[test]
fn an_unreadable_file_closes_without_a_reply() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    // Reading this process's memory from address 0 fails with EIO.
    std::os::unix::fs::symlink("/proc/self/mem", viewer.join("mem")).unwrap();
    let server = start(Viewer::Dir(viewer), &data);
    assert_eq!(exchange(server.port, b"GET /mem HTTP/1.0\r\n\r\n"), b"");
    assert_eq!(get(server.port, "/").status, 200);
}

/// A connection reset before or during the headers is dropped quietly, and
/// the server goes on answering.
#[cfg(unix)]
#[test]
fn a_reset_connection_is_dropped() {
    use std::time::Duration;
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let server = start(Viewer::Dir(viewer), &data);
    for sent in [&b""[..], b"GET / HTTP/1.0\r\n"] {
        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        stream.write_all(sent).unwrap();
        // Long enough for the server to accept and block reading.
        std::thread::sleep(Duration::from_millis(200));
        // A zero linger makes the close a reset.
        rustix::net::sockopt::set_socket_linger(&stream, Some(Duration::ZERO)).unwrap();
        drop(stream);
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(get(server.port, "/").status, 200);
}

#[test]
fn host_header_rejects_rebinding_and_allows_localhost_variants() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let server = start(Viewer::Dir(viewer), &data);
    let port = server.port;
    let other_port = if port == u16::MAX { port - 1 } else { port + 1 };

    for allowed in [
        "127.0.0.1".to_string(),
        format!("127.0.0.1:{port}"),
        "localhost".to_string(),
        format!("LOCALHOST:{port}"),
        "[::1]".to_string(),
        format!("[::1]:{port}"),
    ] {
        let reply = request(port, "GET", "/data/index.json", &[("Host", &allowed)]);
        assert_eq!(reply.status, 200, "{allowed}");
    }

    for forbidden in [
        "attacker.example".to_string(),
        format!("attacker.example:{port}"),
        format!("localhost.attacker.example:{port}"),
        format!("127.0.0.1:{other_port}"),
        format!("[::1]:{other_port}"),
        "[::1]junk".to_string(),
        "".to_string(),
    ] {
        let reply = request(port, "GET", "/data/index.json", &[("Host", &forbidden)]);
        assert_eq!(reply.status, 403, "{forbidden}");
        assert!(!String::from_utf8_lossy(&reply.body).contains("generated_at"));
    }

    let dup_host = request(
        port,
        "GET",
        "/data/index.json",
        &[("Host", "localhost"), ("Host", "127.0.0.1")],
    );
    assert_eq!(dup_host.status, 400);

    let missing_http11 = exchange(port, b"GET /data/index.json HTTP/1.1\r\n\r\n");
    assert!(
        String::from_utf8_lossy(&missing_http11).starts_with("HTTP/1.0 400 Bad Request\r\n"),
        "{}",
        String::from_utf8_lossy(&missing_http11)
    );

    let valid_http11 = exchange(
        port,
        format!("GET /data/index.json HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n").as_bytes(),
    );
    assert!(
        String::from_utf8_lossy(&valid_http11).starts_with("HTTP/1.0 200 OK\r\n"),
        "{}",
        String::from_utf8_lossy(&valid_http11)
    );
}
