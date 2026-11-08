// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Opt-in browser qualification: drives an installed Chromium-compatible
//! browser over CDP against a fabricated 100,000-record catalogue served by
//! the release `lamp` binary. Ordinary `cargo test` never runs it.
//!
//! Usage: cargo run --release --example browser_catalogue -- [--benchmark]
//! [--runs N] [--lamp PATH] [--generator PATH] [--browser PATH] [--output PATH]
//!
//! See tests/browser/README.md. The page-side JavaScript is sent verbatim
//! through `Runtime.evaluate`.

use std::process::ExitCode;

#[cfg(unix)]
fn main() -> ExitCode {
    driver::main()
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    eprintln!(
        "Error: This driver requires POSIX process groups for owned-tree cleanup; Windows needs an isolated job runner"
    );
    ExitCode::FAILURE
}

#[cfg(unix)]
mod driver {
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::io::{self, Read};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitCode, ExitStatus, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use regex::Regex;
    use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};
    use serde_json::{Map, Value, json};
    use tungstenite::{Message, WebSocket};

    const ALLOWED: [&str; 6] = [
        "--benchmark",
        "--lamp",
        "--generator",
        "--browser",
        "--output",
        "--runs",
    ];
    const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
    const WAIT_TIMEOUT: Duration = Duration::from_secs(30);
    const LOAD_TIMEOUT: Duration = Duration::from_secs(45);
    const SOCKET_POLL: Duration = Duration::from_millis(10);
    const UNRESOLVED: &str = "999999999999999";
    const NONE_IMPORTED: &str = "Imported 0 saved fields. Existing values were kept.";

    enum Failure {
        Deadline,
        Error(String),
    }
    type Outcome<T> = Result<T, Failure>;

    fn fail(message: impl Into<String>) -> Failure {
        Failure::Error(message.into())
    }
    impl From<io::Error> for Failure {
        fn from(error: io::Error) -> Self {
            fail(error.to_string())
        }
    }
    impl From<serde_json::Error> for Failure {
        fn from(error: serde_json::Error) -> Self {
            fail(error.to_string())
        }
    }
    impl From<tungstenite::Error> for Failure {
        fn from(error: tungstenite::Error) -> Self {
            fail(error.to_string())
        }
    }

    struct Options {
        benchmark: bool,
        repeats: u32,
        lamp: PathBuf,
        generator: PathBuf,
        browser: OsString,
        output: Option<PathBuf>,
    }

    fn option<'a>(args: &'a [String], name: &str) -> Result<Option<&'a str>, String> {
        let Some(i) = args.iter().position(|arg| arg == name) else {
            return Ok(None);
        };
        match args.get(i + 1) {
            Some(value) if !value.is_empty() && !value.starts_with("--") => Ok(Some(value)),
            _ => Err(format!("Missing value: {name}")),
        }
    }

    fn parse(args: &[String]) -> Result<Options, String> {
        let benchmark = args.iter().any(|arg| arg == "--benchmark");
        let mut i = 0;
        while i < args.len() {
            if !ALLOWED.contains(&args[i].as_str()) {
                return Err(format!("Unknown option: {}", args[i]));
            }
            i += if args[i] == "--benchmark" { 1 } else { 2 };
        }
        let repeats = match option(args, "--runs")? {
            None if benchmark => 5,
            None => 1,
            Some(value) => value
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|n| n.fract() == 0.0 && (1.0..=10.0).contains(n))
                .ok_or("--runs must be an integer from 1 to 10")? as u32,
        };
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let target_dir = std::env::var_os("CARGO_TARGET_DIR").filter(|dir| !dir.is_empty());
        let target = repo
            .join(target_dir.unwrap_or_else(|| "target".into()))
            .join("release");
        let absolute = |path: &str| std::path::absolute(path).map_err(|error| error.to_string());
        let lamp = match option(args, "--lamp")? {
            Some(path) => absolute(path)?,
            None => target.join("lamp"),
        };
        let generator = match option(args, "--generator")? {
            Some(path) => absolute(path)?,
            None => target.join("examples").join("make_demo_data"),
        };
        let browser = match option(args, "--browser")? {
            Some(path) => path.into(),
            None => std::env::var_os("BROWSER_BIN")
                .filter(|path| !path.is_empty())
                .ok_or("Set BROWSER_BIN or pass --browser with an installed Chromium-compatible executable")?,
        };
        let output = option(args, "--output")?.map(absolute).transpose()?;
        Ok(Options {
            benchmark,
            repeats,
            lamp,
            generator,
            browser,
            output,
        })
    }

    #[derive(Default)]
    struct Memory {
        samples: u64,
        peak_bytes: Option<u64>,
        error: Option<String>,
    }

    /// Aggregate RSS of every process whose group is one of `groups`.
    fn resident_bytes(groups: &[i32]) -> io::Result<u64> {
        let mut total = 0;
        for entry in fs::read_dir("/proc")? {
            let name = entry?.file_name();
            let Some(pid) = name
                .to_str()
                .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            else {
                continue;
            };
            // A child may exit between the metadata reads.
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            let group = stat
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().nth(2))
                .and_then(|field| field.parse::<i32>().ok());
            if !group.is_some_and(|group| groups.contains(&group)) {
                continue;
            }
            let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) else {
                continue;
            };
            let kilobytes = status
                .lines()
                .find_map(|line| line.strip_prefix("VmRSS:"))
                .and_then(|value| value.trim().strip_suffix(" kB"))
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(0);
            total += kilobytes * 1024;
        }
        Ok(total)
    }

    fn sample_memory(
        groups: Arc<Mutex<Vec<i32>>>,
        memory: Arc<Mutex<Memory>>,
        stop: Arc<AtomicBool>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(250));
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let owned = groups.lock().expect("group list").clone();
                let sampled = resident_bytes(&owned);
                let mut memory = memory.lock().expect("memory record");
                match sampled {
                    Ok(total) => {
                        memory.samples += 1;
                        memory.peak_bytes = Some(memory.peak_bytes.unwrap_or(0).max(total));
                    }
                    Err(error) => memory.error = Some(error.to_string()),
                }
            }
        })
    }

    /// The last `n` characters of `text`.
    fn tail(text: &str, n: usize) -> &str {
        let count = text.chars().count();
        match text.char_indices().nth(count.saturating_sub(n)) {
            Some((start, _)) if count > n => &text[start..],
            _ => text,
        }
    }

    fn exit_code(status: ExitStatus) -> String {
        status
            .code()
            .map_or_else(|| "null".to_string(), |code| code.to_string())
    }

    fn forward(mut stream: impl Read + Send + 'static, sink: mpsc::Sender<Vec<u8>>) {
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => return,
                    // Keep draining after readiness so the child never blocks on a full pipe.
                    Ok(n) => {
                        let _ = sink.send(buffer[..n].to_vec());
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => return,
                }
            }
        });
    }

    struct Driver {
        repo: &'static Path,
        run: PathBuf,
        downloads: PathBuf,
        env: Vec<(OsString, OsString)>,
        deadline: Instant,
        owned: Vec<Child>,
        groups: Arc<Mutex<Vec<i32>>>,
        cdp: Option<Cdp>,
    }

    impl Driver {
        fn check_deadline(&self) -> Outcome<()> {
            if Instant::now() >= self.deadline {
                Err(Failure::Deadline)
            } else {
                Ok(())
            }
        }

        fn generate(&mut self, generator: &Path, data: &Path) -> Outcome<()> {
            let mut child = Command::new(generator)
                .arg(data)
                .args(["10", "10000", "100"])
                .current_dir(self.repo)
                .env_clear()
                .envs(self.env.iter().cloned())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| fail(format!("{}: {error}", generator.display())))?;
            let mut stderr = child.stderr.take().expect("piped stderr");
            let diagnostic = thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stderr.read_to_end(&mut bytes);
                bytes
            });
            loop {
                if let Some(status) = child.try_wait()? {
                    if status.success() {
                        return Ok(());
                    }
                    let bytes = diagnostic.join().unwrap_or_default();
                    let text = String::from_utf8_lossy(&bytes);
                    return Err(fail(format!(
                        "Fixture generator exit {}: {}",
                        exit_code(status),
                        tail(&text, 4000)
                    )));
                }
                if let Err(failure) = self.check_deadline() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(failure);
                }
                thread::sleep(Duration::from_millis(50));
            }
        }

        /// Start an owned process group and return the first capture of `pattern` in its output.
        fn launch<S: AsRef<OsStr>>(
            &mut self,
            executable: &OsStr,
            args: &[S],
            pattern: &Regex,
        ) -> Outcome<String> {
            let label = Path::new(executable).display().to_string();
            let mut child = Command::new(executable)
                .args(args)
                .current_dir(self.repo)
                .env_clear()
                .envs(self.env.iter().cloned())
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()
                .map_err(|error| fail(format!("{label}: {error}")))?;
            let (sink, chunks) = mpsc::channel();
            forward(child.stdout.take().expect("piped stdout"), sink.clone());
            forward(child.stderr.take().expect("piped stderr"), sink);
            self.groups
                .lock()
                .expect("group list")
                .push(child.id() as i32);
            self.owned.push(child);
            let child = self.owned.len() - 1;
            let started = Instant::now();
            let mut output = String::new();
            loop {
                self.check_deadline()?;
                match chunks.recv_timeout(Duration::from_millis(50)) {
                    Ok(bytes) => {
                        output.push_str(&String::from_utf8_lossy(&bytes));
                        output = tail(&output, 100_000).to_string();
                        if let Some(found) = pattern.captures(&output) {
                            return Ok(found[1].to_string());
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if let Some(status) = self.owned[child].try_wait()? {
                    for bytes in chunks.try_iter() {
                        output.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    return Err(fail(format!(
                        "Child exited {}: {}",
                        exit_code(status),
                        tail(&output, 1000)
                    )));
                }
                if started.elapsed() >= COMMAND_TIMEOUT {
                    return Err(fail(format!(
                        "Child readiness timed out: {label}: {}",
                        tail(&output, 2000)
                    )));
                }
            }
        }

        fn cleanup(&mut self) {
            if let Some(mut cdp) = self.cdp.take() {
                let _ = cdp.call("Browser.close", json!({}), None);
                let _ = cdp.socket.close(None);
                let _ = cdp.socket.flush();
            }
            for child in &mut self.owned {
                let Some(group) = Pid::from_raw(child.id() as i32) else {
                    continue;
                };
                // The leader may have exited while descendants still own the group.
                let _ = kill_process_group(group, Signal::TERM);
                let until = Instant::now() + Duration::from_secs(3);
                while Instant::now() < until {
                    // Reap the leader, or its zombie keeps the group alive.
                    let _ = child.try_wait();
                    if test_kill_process_group(group).is_err() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                let _ = kill_process_group(group, Signal::KILL);
                let _ = child.wait();
            }
        }
    }

    /// One CDP connection. Events are handled whilst any command or wait reads the socket.
    struct Cdp {
        socket: WebSocket<TcpStream>,
        sequence: u64,
        origin: String,
        completed_downloads: Vec<String>,
        deadline: Instant,
    }

    impl Cdp {
        fn connect(endpoint: &str, origin: String, deadline: Instant) -> Outcome<Self> {
            let address = endpoint
                .strip_prefix("ws://")
                .and_then(|rest| rest.split('/').next())
                .ok_or_else(|| fail(format!("Unexpected DevTools endpoint: {endpoint}")))?;
            let stream = TcpStream::connect(address)?;
            stream.set_read_timeout(Some(COMMAND_TIMEOUT))?;
            stream.set_write_timeout(Some(COMMAND_TIMEOUT))?;
            let (socket, _) = tungstenite::client(endpoint, stream)
                .map_err(|error| fail(format!("WebSocket handshake: {error}")))?;
            socket.get_ref().set_read_timeout(Some(SOCKET_POLL))?;
            Ok(Self {
                socket,
                sequence: 0,
                origin,
                completed_downloads: Vec::new(),
                deadline,
            })
        }

        fn post(&mut self, method: &str, params: Value, session: Option<&str>) -> Outcome<u64> {
            self.sequence += 1;
            let mut message = json!({"id": self.sequence, "method": method, "params": params});
            if let Some(session) = session {
                message["sessionId"] = json!(session);
            }
            self.socket.send(Message::text(message.to_string()))?;
            Ok(self.sequence)
        }

        /// Read one message, answer or record events, and return a command reply.
        fn receive(&mut self) -> Outcome<Option<Value>> {
            if Instant::now() >= self.deadline {
                return Err(Failure::Deadline);
            }
            let text = match self.socket.read() {
                Ok(Message::Text(text)) => text,
                Ok(_) => return Ok(None),
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error.into()),
            };
            let message: Value = serde_json::from_str(&text)?;
            if message.get("id").is_some() {
                return Ok(Some(message));
            }
            let params = &message["params"];
            match message["method"].as_str() {
                Some("Browser.downloadProgress") if params["state"] == "completed" => {
                    if let Some(guid) = params["guid"].as_str() {
                        self.completed_downloads.push(guid.to_string());
                    }
                }
                Some("Fetch.requestPaused") => {
                    let allowed = params["request"]["url"]
                        .as_str()
                        .is_some_and(|url| url.starts_with(&self.origin));
                    let (method, reply) = if allowed {
                        (
                            "Fetch.continueRequest",
                            json!({"requestId": params["requestId"]}),
                        )
                    } else {
                        (
                            "Fetch.failRequest",
                            json!({"requestId": params["requestId"], "errorReason": "BlockedByClient"}),
                        )
                    };
                    let session = message["sessionId"].as_str();
                    self.post(method, reply, session)?;
                }
                _ => {}
            }
            Ok(None)
        }

        fn call(&mut self, method: &str, params: Value, session: Option<&str>) -> Outcome<Value> {
            let id = self.post(method, params, session)?;
            let until = Instant::now() + COMMAND_TIMEOUT;
            loop {
                if let Some(mut reply) = self.receive()?
                    && reply["id"] == id
                {
                    if let Some(error) = reply.get("error") {
                        return Err(fail(error.to_string()));
                    }
                    return Ok(reply["result"].take());
                }
                if Instant::now() >= until {
                    return Err(fail(format!("CDP timeout: {method}")));
                }
            }
        }

        fn pump(&mut self, duration: Duration) -> Outcome<()> {
            let until = Instant::now() + duration;
            while Instant::now() < until {
                self.receive()?;
            }
            Ok(())
        }

        fn evaluate(&mut self, session: &str, expression: &str) -> Outcome<Value> {
            let mut response = self.call(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true, "awaitPromise": true}),
                Some(session),
            )?;
            if let Some(details) = response.get("exceptionDetails") {
                return Err(fail(details.to_string()));
            }
            Ok(response
                .pointer_mut("/result/value")
                .map(Value::take)
                .unwrap_or(Value::Null))
        }

        fn wait_for<T>(
            &mut self,
            label: &str,
            timeout: Duration,
            mut check: impl FnMut(&mut Self) -> Outcome<Option<T>>,
        ) -> Outcome<T> {
            let end = Instant::now() + timeout;
            while Instant::now() < end {
                if let Some(value) = check(self)? {
                    return Ok(value);
                }
                self.pump(Duration::from_millis(40))?;
            }
            Err(fail(format!("Timed out: {label}")))
        }

        fn wait_true(
            &mut self,
            session: &str,
            expression: &str,
            label: &str,
            timeout: Duration,
        ) -> Outcome<()> {
            self.wait_for(label, timeout, |cdp| {
                Ok((cdp.evaluate(session, expression)? == true).then_some(()))
            })
        }

        fn frames(&mut self, session: &str) -> Outcome<()> {
            self.evaluate(
                session,
                "new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))",
            )?;
            Ok(())
        }

        fn key(&mut self, session: &str, value: &str) -> Outcome<()> {
            let vk = match value {
                "Tab" => 9,
                "Enter" => 13,
                "ArrowRight" => 39,
                "End" => 35,
                "Home" => 36,
                "p" => 80,
                _ => unreachable!("unmapped key {value}"),
            };
            let code = if value == "p" { "KeyP" } else { value };
            for kind in ["keyDown", "keyUp"] {
                self.call(
                    "Input.dispatchKeyEvent",
                    json!({"type": kind, "key": value, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk}),
                    Some(session),
                )?;
            }
            self.frames(session)
        }

        fn focus_state(&mut self, session: &str) -> Outcome<Value> {
            self.evaluate(
                session,
                r##"(() => {const cards=[...document.querySelectorAll('#grid > .card[tabindex="0"]')]; const a=document.activeElement; return {stops:cards.length,connected:a?.isConnected,activeCard:a===cards[0],pos:Number(a?.getAttribute('aria-posinset')),size:Number(a?.getAttribute('aria-setsize')),count:state.filtered.length};})()"##,
            )
        }

        fn upload(&mut self, session: &str, path: &Path) -> Outcome<Value> {
            self.evaluate(
                session,
                r##"document.querySelector("#catalogue-tools").open=true;document.querySelector("#action-status").textContent="""##,
            )?;
            let document = self.call("DOM.getDocument", json!({}), Some(session))?;
            let found = self.call(
                "DOM.querySelector",
                json!({"nodeId": document["root"]["nodeId"], "selector": "#import-backup"}),
                Some(session),
            )?;
            self.call(
                "DOM.setFileInputFiles",
                json!({"nodeId": found["nodeId"], "files": [text(path)?]}),
                Some(session),
            )?;
            self.wait_true(
                session,
                r##"document.querySelector("#action-status").textContent.startsWith("Imported ")"##,
                "import",
                WAIT_TIMEOUT,
            )?;
            self.evaluate(
                session,
                r##"document.querySelector("#action-status").textContent"##,
            )
        }

        fn download(&mut self, session: &str, downloads: &Path) -> Outcome<(PathBuf, Value)> {
            let previous = self.completed_downloads.len();
            self.evaluate(
                session,
                r##"document.querySelector("#export-favs").click()"##,
            )?;
            let guid = self.wait_for("download", WAIT_TIMEOUT, |cdp| {
                Ok(cdp.completed_downloads.get(previous).cloned())
            })?;
            let path = downloads.join(guid);
            let backup = serde_json::from_str(&fs::read_to_string(&path)?)?;
            Ok((path, backup))
        }
    }

    fn text(path: &Path) -> Outcome<&str> {
        path.to_str()
            .ok_or_else(|| fail(format!("Path is not UTF-8: {}", path.display())))
    }

    fn ensure(ok: bool, what: &str) -> Outcome<()> {
        if ok {
            Ok(())
        } else {
            Err(fail(format!("Assertion failed: {what}")))
        }
    }

    /// Strict equality, with numbers compared by value as JavaScript does.
    fn ensure_eq(actual: &Value, expected: &Value, what: &str) -> Outcome<()> {
        let same = match (actual.as_f64(), expected.as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => actual == expected,
        };
        if same {
            Ok(())
        } else {
            Err(fail(format!(
                "Assertion failed: {what}: expected {expected}, got {actual}"
            )))
        }
    }

    fn finite(value: &Value) -> bool {
        value.as_f64().is_some_and(f64::is_finite)
    }

    /// Backup entries in a stable order, for comparisons that ignore export order.
    fn ordered(entries: &Value) -> Vec<Value> {
        let mut entries = entries.as_array().cloned().unwrap_or_default();
        let key = |entry: &Value| {
            format!(
                "{}{}",
                entry["kind"].as_str().unwrap_or_default(),
                entry["id"].as_str().unwrap_or_default()
            )
        };
        entries.sort_by_key(key);
        entries
    }

    /// Backup entries equal in any order; a mismatch names the first entry that differs.
    fn ensure_entries(actual: &Value, expected: &Value, what: &str) -> Outcome<()> {
        let (actual, expected) = (ordered(actual), ordered(expected));
        if actual == expected {
            return Ok(());
        }
        let at = actual
            .iter()
            .zip(&expected)
            .take_while(|(a, b)| a == b)
            .count();
        Err(fail(format!(
            "Assertion failed: {what}: {} entries, expected {}; first difference at {at}: expected {}, got {}",
            actual.len(),
            expected.len(),
            expected.get(at).unwrap_or(&Value::Null),
            actual.get(at).unwrap_or(&Value::Null),
        )))
    }

    /// Remove the run folder, retrying whilst killed descendants finish closing
    /// files, as the Node driver's `rm` did (10 retries, 200 ms apart).
    fn remove_run(path: &Path) -> io::Result<()> {
        let mut retries = 10;
        loop {
            match fs::remove_dir_all(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(_) if retries > 0 => {
                    retries -= 1;
                    thread::sleep(Duration::from_millis(200));
                }
                result => return result,
            }
        }
    }

    fn last_run(result: &mut Value) -> &mut Value {
        result["runs"]
            .as_array_mut()
            .and_then(|runs| runs.last_mut())
            .expect("a run was recorded")
    }

    fn summarize(mut values: Vec<Value>) -> Value {
        values.sort_by(|a, b| a.as_f64().partial_cmp(&b.as_f64()).expect("finite timings"));
        let n = values.len();
        let at = |i: usize| values.get(i).cloned().unwrap_or(Value::Null);
        json!({
            "n": n,
            "min": at(0),
            "median": at(n.div_ceil(2).saturating_sub(1)),
            "p95_nearest_rank": at(((n as f64 * 0.95).ceil() as usize).saturating_sub(1)),
            "max": at(n.saturating_sub(1)),
        })
    }

    const NEW_DOCUMENT: &str = r##"window.measurement={longTasks:[],frames:[],firstCard:null};new PerformanceObserver(list=>measurement.longTasks.push(...list.getEntries().map(e=>e.duration))).observe({type:'longtask',buffered:true});let last;function frame(t){if(last&&measurement.frames.length<1000)measurement.frames.push(t-last);last=t;if(!measurement.firstCard&&document.querySelector('#grid > .card'))measurement.firstCard=performance.now();if(measurement.frames.length<1000||!measurement.firstCard)requestAnimationFrame(frame);}requestAnimationFrame(frame);"##;
    const LOADED: &str = r##"typeof state!=="undefined" && state.loadedSellers===10 && document.querySelector("#grid")?.getAttribute("aria-busy")==="false" && !!document.querySelector("#grid > .card")"##;
    const WARM_LOADED: &str = r##"window.measurement && typeof state!=="undefined" && state.loadedSellers===10 && document.querySelector("#grid")?.getAttribute("aria-busy")==="false" && !!document.querySelector("#grid > .card")"##;
    const MEASUREMENT: &str = r##"({viewport:[innerWidth,innerHeight],complete_ms:performance.now(),raw:[...state.shardAlbums.values()].reduce((n,a)=>n+a.length,0),selected:state.albums.length,filtered:state.filtered.length,loadedSellers:state.loadedSellers,failedShards:state.failedShards,rendered:document.querySelectorAll('#grid > .card').length,first_card_ms:measurement.firstCard,long_tasks_ms:measurement.longTasks})"##;
    const INITIAL_FOCUS: &str = r##"({stops:document.querySelectorAll('#grid > .card[tabindex="0"]').length,scrollY,cursor:state.view.cursor,first:[...document.querySelectorAll('#grid > .card')].slice(0,3).map(c=>({tab:c.getAttribute('tabindex'),pos:c.getAttribute('aria-posinset')}))})"##;

    fn qualify(d: &mut Driver, options: &Options, result: &mut Value) -> Outcome<()> {
        let run = d.run.clone();
        let downloads = d.downloads.clone();
        let data = run.join("data");
        d.generate(&options.generator, &data)?;
        let port = TcpListener::bind("127.0.0.1:0")?
            .local_addr()?
            .port()
            .to_string();
        // The CLI requires a nonzero port. A bind race fails this run rather than reusing a server.
        let serving = Regex::new(r"(?i)serving on (http://127\.0\.0\.1:\d+/)").expect("pattern");
        let lamp_args = [
            OsStr::new("--data"),
            data.as_os_str(),
            OsStr::new("serve"),
            OsStr::new("--port"),
            OsStr::new(&port),
            OsStr::new("--no-browser"),
        ];
        let origin = d.launch(options.lamp.as_os_str(), &lamp_args, &serving)?;
        let profile = run.join("browser-profile");
        fs::create_dir(&profile)?;
        let mut user_data = OsString::from("--user-data-dir=");
        user_data.push(&profile);
        let mut browser_args: Vec<OsString> = [
            "--headless=new",
            "--disable-dev-shm-usage",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-sync",
            "--disable-extensions",
            "--no-first-run",
            "--remote-debugging-port=0",
        ]
        .map(OsString::from)
        .into();
        browser_args.extend([user_data, "about:blank".into()]);
        let listening = Regex::new(r"DevTools listening on (ws://\S+)").expect("pattern");
        let endpoint = d.launch(&options.browser, &browser_args, &listening)?;
        let deadline = d.deadline;
        let cdp = d
            .cdp
            .insert(Cdp::connect(&endpoint, origin.clone(), deadline)?);
        result["browser"] = cdp.call("Browser.getVersion", json!({}), None)?;

        for repetition in 1..=options.repeats {
            let mut previous_export: Option<(PathBuf, Value)> = None;
            for (width, height) in [(1440, 900), (390, 844)] {
                let context = cdp.call("Target.createBrowserContext", json!({}), None)?;
                let context_id = &context["browserContextId"];
                cdp.call(
                    "Browser.setDownloadBehavior",
                    json!({"behavior": "allowAndName", "downloadPath": text(&downloads)?, "eventsEnabled": true, "browserContextId": context_id}),
                    None,
                )?;
                let target = cdp.call(
                    "Target.createTarget",
                    json!({"url": "about:blank", "browserContextId": context_id}),
                    None,
                )?;
                let attached = cdp.call(
                    "Target.attachToTarget",
                    json!({"targetId": target["targetId"], "flatten": true}),
                    None,
                )?;
                let session = attached["sessionId"]
                    .as_str()
                    .ok_or_else(|| fail("Target.attachToTarget returned no sessionId"))?
                    .to_string();
                let session = session.as_str();
                for domain in ["Page", "Runtime", "Performance"] {
                    cdp.call(&format!("{domain}.enable"), json!({}), Some(session))?;
                }
                cdp.call(
                    "Fetch.enable",
                    json!({"patterns": [{"urlPattern": "*", "requestStage": "Request"}]}),
                    Some(session),
                )?;
                cdp.call(
                    "Emulation.setDeviceMetricsOverride",
                    json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": false}),
                    Some(session),
                )?;
                cdp.call(
                    "Page.addScriptToEvaluateOnNewDocument",
                    json!({"source": NEW_DOCUMENT}),
                    Some(session),
                )?;
                cdp.call("Page.navigate", json!({"url": origin}), Some(session))?;
                cdp.wait_true(session, LOADED, "100k catalogue load", LOAD_TIMEOUT)?;
                let mut measurement = cdp.evaluate(session, MEASUREMENT)?;
                ensure(measurement.is_object(), "measurement is an object")?;
                ensure_eq(&measurement["raw"], &json!(100000), "raw records")?;
                ensure_eq(&measurement["loadedSellers"], &json!(10), "loaded sellers")?;
                ensure_eq(&measurement["failedShards"], &json!([]), "failed shards")?;
                ensure(
                    finite(&measurement["first_card_ms"]),
                    "first card time was observed",
                )?;
                cdp.frames(session)?;
                measurement["initial_focus_state"] = cdp.evaluate(session, INITIAL_FOCUS)?;
                measurement["repetition"] = json!(repetition);
                measurement["cache"] =
                    json!("fresh browser context; OS/server caches uncontrolled");
                let filtered = measurement["filtered"].clone();
                result["runs"]
                    .as_array_mut()
                    .expect("runs array")
                    .push(measurement);

                // A warm reload is measured independently before any saved-field edits.
                cdp.evaluate(session, "window.measurement=null")?;
                cdp.call("Page.reload", json!({"ignoreCache": false}), Some(session))?;
                cdp.wait_true(session, WARM_LOADED, "warm catalogue load", LOAD_TIMEOUT)?;
                let warm = cdp.evaluate(
                    session,
                    "({complete_ms:performance.now(),first_card_ms:measurement.firstCard,long_tasks_ms:measurement.longTasks})",
                )?;
                let warm_first_card = finite(&warm["first_card_ms"]);
                last_run(result)["warm_reload"] = warm;
                ensure(warm_first_card, "warm first card time was observed")?;

                cdp.key(session, "Tab")?;
                let skip = cdp.evaluate(
                    session,
                    r##"({tag:document.activeElement.tagName,id:document.activeElement.id,href:document.activeElement.getAttribute("href"),scrollY})"##,
                )?;
                last_run(result)["native_entry"] = json!([skip]);
                ensure_eq(
                    &skip["href"],
                    &json!("#grid"),
                    "Tab reaches the #grid skip link",
                )?;
                cdp.key(session, "Enter")?;
                cdp.key(session, "Tab")?;
                let entered = cdp.focus_state(session)?;
                let native_entry = |result: &mut Value, state: &Value| {
                    last_run(result)["native_entry"]
                        .as_array_mut()
                        .expect("native entry list")
                        .push(state.clone());
                };
                native_entry(result, &entered);
                ensure_eq(
                    &entered["activeCard"],
                    &json!(true),
                    "Enter then Tab focuses the active card",
                )?;
                cdp.key(session, "ArrowRight")?;
                let advanced = cdp.focus_state(session)?;
                native_entry(result, &advanced);
                ensure_eq(
                    &advanced["pos"],
                    &json!(entered["pos"].as_f64().map(|pos| pos + 1.0)),
                    "ArrowRight advances one card",
                )?;
                cdp.key(session, "Home")?;
                if let Some((path, backup)) = &previous_export {
                    ensure_eq(
                        &cdp.upload(session, path)?,
                        &json!("Imported 4 saved fields. Existing values were kept."),
                        "import of the previous context's export",
                    )?;
                    let (_, immediate) = cdp.download(session, &downloads)?;
                    ensure_entries(
                        &immediate["entries"],
                        &backup["entries"],
                        "immediate export matches the previous context's export",
                    )?;
                    last_run(result)["roundtrip_verified_before_edits"] = json!(true);
                }

                // Set an explicit keyboard-test starting viewport; retain the initial anchoring observation above.
                cdp.evaluate(session, "window.scrollTo(0,0)")?;
                cdp.frames(session)?;
                cdp.evaluate(
                    session,
                    r##"document.querySelector('#grid > .card[tabindex="0"]').focus()"##,
                )?;
                let mut positions = Vec::new();
                for (pressed, expected) in [
                    ("ArrowRight", json!(2)),
                    ("End", filtered.clone()),
                    ("Home", json!(1)),
                ] {
                    cdp.key(session, pressed)?;
                    let focus = cdp.focus_state(session)?;
                    ensure_eq(
                        &focus["stops"],
                        &json!(1),
                        &format!("{pressed}: one tab stop"),
                    )?;
                    ensure_eq(
                        &focus["connected"],
                        &json!(true),
                        &format!("{pressed}: focus connected"),
                    )?;
                    ensure_eq(
                        &focus["activeCard"],
                        &json!(true),
                        &format!("{pressed}: active card focused"),
                    )?;
                    ensure_eq(&focus["pos"], &expected, &format!("{pressed}: position"))?;
                    ensure_eq(&focus["size"], &filtered, &format!("{pressed}: set size"))?;
                    let mut position = Map::new();
                    position.insert("pressed".into(), json!(pressed));
                    position.extend(focus.as_object().cloned().unwrap_or_default());
                    positions.push(Value::Object(position));
                }
                cdp.evaluate(
                    session,
                    r##"document.querySelector("#grid > .card .fav-btn").focus();window.scrollTo(0,document.documentElement.scrollHeight)"##,
                )?;
                cdp.frames(session)?;
                let rescued = cdp.evaluate(
                    session,
                    r##"({connected:document.activeElement.isConnected,safe:document.activeElement.matches(".fav-btn, #grid")})"##,
                )?;
                ensure_eq(
                    &rescued["connected"],
                    &json!(true),
                    "recycled focus stays connected",
                )?;
                ensure_eq(&rescued["safe"], &json!(true), "recycled focus is safe")?;
                cdp.evaluate(session, "window.scrollTo(0,0)")?;
                cdp.frames(session)?;
                cdp.evaluate(
                    session,
                    r##"document.querySelector("#grid > .card").focus()"##,
                )?;
                cdp.key(session, "Home")?;
                cdp.key(session, "p")?;
                cdp.wait_true(
                    session,
                    r##"document.querySelector("#album-preview").open && document.querySelector("#album-preview").contains(document.activeElement)"##,
                    "preview focus",
                    WAIT_TIMEOUT,
                )?;
                cdp.evaluate(
                    session,
                    r##"document.querySelector('#preview-notes').value='Fabricated offline note';document.querySelector('#preview-tags').value='synthetic';document.querySelector('#preview-collection').value='Fixture';document.querySelector('#preview-form').requestSubmit()"##,
                )?;
                ensure_eq(
                    &cdp.evaluate(
                        session,
                        r##"document.querySelector("#preview-save-status").textContent"##,
                    )?,
                    &json!("Saved."),
                    "preview save status",
                )?;
                cdp.evaluate(
                    session,
                    r##"document.querySelector("#preview-close").click()"##,
                )?;
                cdp.frames(session)?;
                ensure_eq(
                    &cdp.focus_state(session)?["activeCard"],
                    &json!(true),
                    "preview close returns focus to the active card",
                )?;

                let missing = run.join(format!("browser-missing-{width}.json"));
                fs::write(
                    &missing,
                    json!({"format": "diogeneslamp", "version": 1, "entries": [{"kind": "note", "id": UNRESOLVED, "value": "Fabricated unavailable listing"}]}).to_string(),
                )?;
                cdp.upload(session, &missing)?;
                let (exported_path, exported) = cdp.download(session, &downloads)?;
                ensure_eq(&exported["format"], &json!("diogeneslamp"), "backup format")?;
                ensure_eq(&exported["version"], &json!(1), "backup version")?;
                ensure_eq(
                    &exported["catalogue_status"],
                    &json!("complete"),
                    "backup catalogue status",
                )?;
                ensure(
                    exported["unresolved_ids"]
                        .as_array()
                        .is_some_and(|ids| ids.contains(&json!(UNRESOLVED))),
                    "backup keeps the unresolved id",
                )?;
                let entries = &exported["entries"];
                ensure_eq(
                    &json!(entries.as_array().map(Vec::len)),
                    &json!(4),
                    "exported entry count",
                )?;
                let mut saved_ids: Vec<&Value> = entries
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|entry| &entry["id"])
                    .filter(|id| id.as_str() != Some(UNRESOLVED))
                    .collect();
                saved_ids.sort_by_key(|id| id.to_string());
                saved_ids.dedup();
                ensure(
                    saved_ids.len() == 1,
                    "the 3 saved fields belong to one listing",
                )?;
                let saved_id = saved_ids[0];
                let expected = json!([
                    {"kind": "note", "id": UNRESOLVED, "value": "Fabricated unavailable listing"},
                    {"kind": "collection", "id": saved_id, "value": "Fixture"},
                    {"kind": "tags", "id": saved_id, "value": ["synthetic"]},
                    {"kind": "note", "id": saved_id, "value": "Fabricated offline note"},
                ]);
                ensure_entries(entries, &expected, "exported saved fields")?;
                if let Some((_, backup)) = &previous_export {
                    ensure_entries(
                        entries,
                        &backup["entries"],
                        "export matches the previous context's export",
                    )?;
                }
                ensure_eq(
                    &cdp.upload(session, &exported_path)?,
                    &json!(NONE_IMPORTED),
                    "reimporting the export adds nothing",
                )?;
                let metrics = cdp.call("Performance.getMetrics", json!({}), Some(session))?;
                let heap = metrics["metrics"].as_array().and_then(|metrics| {
                    metrics
                        .iter()
                        .find(|metric| metric["name"] == "JSHeapUsedSize")
                });
                let frames = cdp.evaluate(session, "measurement.frames")?;
                let measurement = last_run(result);
                if let Some(heap) = heap {
                    measurement["renderer_js_heap_used_bytes"] = heap["value"].clone();
                }
                measurement["focus"] = Value::Array(positions);
                measurement["recycled_focus"] = rescued;
                measurement["saved_fields"] = entries.clone();
                measurement["unresolved_ids"] = exported["unresolved_ids"].clone();
                measurement["frame_intervals_ms"] = frames;

                if options.benchmark && repetition == 1 && width == 390 {
                    measurement["large_backups"] = json!([]);
                    for count in [1000, 10000] {
                        let notes: Vec<Value> = (0..count)
                            .map(|i| json!({"kind": "note", "id": (800_000_000_000_000_u64 + i).to_string(), "value": format!("Fabricated saved note {i}")}))
                            .collect();
                        let file = run.join(format!("backup-{count}.json"));
                        fs::write(
                            &file,
                            json!({"format": "diogeneslamp", "version": 1, "entries": notes})
                                .to_string(),
                        )?;
                        let started = Instant::now();
                        let status = cdp.upload(session, &file)?;
                        let import_ms = started.elapsed().as_secs_f64() * 1000.0;
                        let (_, roundtrip) = cdp.download(session, &downloads)?;
                        let import_export_ms = started.elapsed().as_secs_f64() * 1000.0;
                        let mut all = entries.as_array().cloned().unwrap_or_default();
                        all.extend(notes);
                        ensure_entries(
                            &roundtrip["entries"],
                            &Value::Array(all),
                            &format!("{count}-note backup round trip"),
                        )?;
                        ensure_eq(
                            &cdp.upload(session, &file)?,
                            &json!(NONE_IMPORTED),
                            &format!("reimporting the {count}-note backup adds nothing"),
                        )?;
                        last_run(result)["large_backups"]
                            .as_array_mut()
                            .expect("large backup list")
                            .push(json!({"entries": count, "status": status, "import_ms": import_ms, "import_export_ms": import_export_ms, "exact_roundtrip": true}));
                    }
                }
                previous_export = Some((exported_path, exported));
                cdp.call(
                    "Target.disposeBrowserContext",
                    json!({"browserContextId": context["browserContextId"]}),
                    None,
                )?;
            }
        }

        let runs = result["runs"].as_array().cloned().unwrap_or_default();
        let distribution: Vec<Value> = [1440, 390]
            .into_iter()
            .map(|width| {
                let samples: Vec<&Value> = runs
                    .iter()
                    .filter(|run| run["viewport"][0] == width)
                    .collect();
                json!({
                    "width": width,
                    "fresh_complete_ms": summarize(samples.iter().map(|run| run["complete_ms"].clone()).collect()),
                    "warm_complete_ms": summarize(samples.iter().map(|run| run["warm_reload"]["complete_ms"].clone()).collect()),
                })
            })
            .collect();
        result["distribution"] = Value::Array(distribution);
        Ok(())
    }

    pub fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let options = match parse(&args) {
            Ok(options) => options,
            Err(message) => {
                eprintln!("Error: {message}");
                return ExitCode::FAILURE;
            }
        };
        let temporary = match tempfile::Builder::new().prefix("lamp-browser-").tempdir() {
            Ok(dir) => dir,
            Err(error) => {
                eprintln!("Error: {error}");
                return ExitCode::FAILURE;
            }
        };
        let run = temporary.path().to_path_buf();
        let mut env: Vec<(OsString, OsString)> = ["PATH", "LANG", "LC_ALL", "LD_LIBRARY_PATH"]
            .into_iter()
            .filter_map(|name| {
                std::env::var_os(name)
                    .filter(|value| !value.is_empty())
                    .map(|value| (name.into(), value))
            })
            .collect();
        for name in ["HOME", "TMPDIR", "TEMP", "TMP"] {
            env.push((name.into(), run.clone().into()));
        }
        // lamp keeps Python's stdout buffering; unbuffered, its readiness line arrives at once.
        env.push(("PYTHONUNBUFFERED".into(), "1".into()));
        let mut result = json!({
            "scope": "Fabricated release-build catalogue, 100000 raw records; fresh contexts and warm reloads, not OS-cache-cold or mobile hardware",
            "repeats": options.repeats,
            "runs": [],
            "memory": {"scope": "Linux sampled aggregate RSS of owned child process groups; shared pages counted per process; not physical memory or an unsampled peak", "interval_ms": 250, "samples": 0, "peak_bytes": null},
        });
        let groups = Arc::new(Mutex::new(Vec::new()));
        let memory = Arc::new(Mutex::new(Memory::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let sampler = cfg!(target_os = "linux")
            .then(|| sample_memory(groups.clone(), memory.clone(), stop.clone()));
        let limit = Duration::from_secs(if options.benchmark { 600 } else { 180 });
        let mut driver = Driver {
            repo: Path::new(env!("CARGO_MANIFEST_DIR")),
            downloads: run.join("browser-downloads"),
            run,
            env,
            deadline: Instant::now() + limit,
            owned: Vec::new(),
            groups,
            cdp: None,
        };
        let outcome = fs::create_dir(&driver.downloads)
            .map_err(Failure::from)
            .and_then(|()| qualify(&mut driver, &options, &mut result));

        // One finalisation path for pass, failure and the overall deadline.
        stop.store(true, Ordering::Relaxed);
        if let Some(sampler) = sampler {
            let _ = sampler.join();
        }
        let memory = memory.lock().expect("memory record");
        result["memory"]["samples"] = json!(memory.samples);
        result["memory"]["peak_bytes"] = json!(memory.peak_bytes);
        if let Some(error) = &memory.error {
            result["memory"]["error"] = json!(error);
        }
        driver.cleanup();
        let removed = remove_run(temporary.path());
        let code = match outcome {
            Ok(()) => {
                result["status"] = json!("pass");
                0
            }
            Err(Failure::Deadline) => {
                result["status"] = json!("fail");
                result["error"] = json!("Overall deadline exceeded");
                124
            }
            Err(Failure::Error(message)) => {
                result["status"] = json!("fail");
                result["error"] = json!(message);
                1
            }
        };
        let mut code = match removed {
            Ok(()) => code,
            Err(error) => {
                eprintln!("Error: temporary folder not removed: {error}");
                if code == 0 { 1 } else { code }
            }
        };
        let json = serde_json::to_string_pretty(&result).expect("JSON values serialise");
        if let Some(output) = &options.output
            && let Err(error) = fs::write(output, format!("{json}\n"))
        {
            eprintln!("Error: {}: {error}", output.display());
            if code == 0 {
                code = 1;
            }
        }
        println!("{json}");
        ExitCode::from(code)
    }
}
