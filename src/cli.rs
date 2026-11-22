// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! lamp command line interface.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::errors::Error;
use crate::ingest::{load_sources, merge_sources, parse_dump, write_sources};
use crate::out;
use crate::pyerr;
use crate::pyjson::{self, Style};
use crate::pystr;
use crate::scrape::{self, Interrupt, Options, SellerSummary};

fn as_path(text: &str) -> Result<String, String> {
    Ok(pyerr::pathlib_normalise(text))
}

/// Whether argparse took a value starting with '-' as a value rather than an
/// option: when '-' is followed by an ASCII digit, or by '.' and one, or when
/// the value holds a space.
fn number_shaped(text: &str) -> bool {
    let mut chars = text.chars();
    if chars.next() != Some('-') || text.contains(' ') {
        return true;
    }
    match chars.next() {
        Some('.') => chars.next().is_some_and(|c| c.is_ascii_digit()),
        c => c.is_some_and(|c| c.is_ascii_digit()),
    }
}

fn whole(text: &str) -> Result<i128, String> {
    number_shaped(text)
        .then(|| pystr::parse_int(text))
        .flatten()
        .ok_or_else(|| "must be a whole number".into())
}

fn positive_int(text: &str) -> Result<i128, String> {
    let value = whole(text)?;
    if value < 1 {
        return Err("must be 1 or greater".into());
    }
    Ok(value)
}

fn positive_float(text: &str) -> Result<f64, String> {
    let value = number_shaped(text)
        .then(|| pystr::parse_float(text))
        .flatten()
        .ok_or("must be a number")?;
    if !value.is_finite() || value <= 0.0 {
        return Err("must be a finite number greater than 0".into());
    }
    Ok(value)
}

fn export_format(text: &str) -> Result<String, String> {
    match text {
        "csv" | "json" => Ok(text.into()),
        _ => Err("must be csv or json".into()),
    }
}

/// A calendar date as YYYY-MM-DD.
fn day(text: &str) -> Result<String, String> {
    let shape = text.len() == 10
        && text.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if !shape || crate::isodate::parse_date(text).is_none() {
        return Err("must be a date as YYYY-MM-DD".into());
    }
    Ok(text.into())
}

fn match_limit(text: &str) -> Result<i128, String> {
    let value = whole(text)?;
    if !(1..=20).contains(&value) {
        return Err("must be between 1 and 20".into());
    }
    Ok(value)
}

fn port(text: &str) -> Result<i128, String> {
    let value = whole(text)?;
    if !(1..=65535).contains(&value) {
        return Err("port must be between 1 and 65535".into());
    }
    Ok(value)
}

/// An option taking a value, absent unless given or defaulted. Numbers that
/// start with '-' are values, so a negative one reaches the option's check.
fn opt(long: &'static str, value_name: &'static str) -> Arg {
    Arg::new(long)
        .long(long)
        .value_name(value_name)
        .hide_default_value(true)
        .allow_negative_numbers(true)
}

/// A numeric option. Any value starting with '-' is its value, so `-.5`
/// reaches the option's check, which refuses what argparse read as an option.
fn num(long: &'static str, value_name: &'static str) -> Arg {
    opt(long, value_name).allow_hyphen_values(true)
}

fn flag(long: &'static str) -> Arg {
    Arg::new(long).long(long).action(ArgAction::SetTrue)
}

fn positional(id: &'static str, value_name: &'static str) -> Arg {
    Arg::new(id)
        .value_name(value_name)
        .required(true)
        .value_parser(as_path)
}

pub fn build_parser() -> Command {
    let port_option = || num("port", "PORT").default_value("8000").value_parser(port);
    let scrape = Command::new("scrape")
        .about("scrape Yupoo sellers into data/")
        .arg(opt("seller", "SELLER").help("scrape only this seller id"))
        .arg(flag("full").help("re-fetch unchanged albums"))
        .arg(
            num("refresh-details-days", "N")
                .value_parser(positive_int)
                .help("also refresh unchanged details last checked N days ago"),
        )
        .arg(
            num("refresh-limit", "N").default_value("100")
                .value_parser(positive_int)
                .help("maximum extra unchanged-detail requests across this run"),
        )
        .arg(flag("retry-failed").help("retry only album IDs recorded as failed in earlier runs"))
        .arg(num("workers", "WORKERS").default_value("8").value_parser(positive_int))
        .arg(
            num("rps", "RPS").default_value("8.0")
                .value_parser(positive_float)
                .help("global request rate cap"),
        )
        .arg(num("max-age-days", "N").value_parser(whole).help(
            "skip albums posted more than N days ago (0 = no age limit); stock that old is usually gone",
        ))
        .arg(
            flag("dry-run")
                .help("preview scrape work without fetching album pages or saving files"),
        )
        .arg(opt("notify", "COMMAND").help(
            "after a run that stores or changes albums, run COMMAND in the system shell with them as JSON on its stdin (not after --dry-run or Ctrl+C)",
        ));
    let export = Command::new("export")
        .about("print the indexed albums as CSV or JSON, or write them as a static site")
        .arg(
            opt("format", "csv|json").default_value("csv")
                .value_parser(export_format)
                .help("csv (the viewer's columns) or json"),
        )
        .arg(
            opt("since", "YYYY-MM-DD")
                .value_parser(day)
                .help("only albums first stored on this date or later"),
        )
        .arg(opt("site", "DIR").value_parser(as_path).help(
            "instead, write the viewer and the whole catalogue as static files in DIR, a new or empty folder, to open from a web host",
        ));
    Command::new("lamp")
        .about("lamp command line interface.")
        .version(env!("CARGO_PKG_VERSION"))
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .subcommand_required(true)
        .infer_long_args(true)
        .arg(
            opt("sources", "SOURCES")
                .default_value("sources.toml")
                .value_parser(as_path),
        )
        .arg(
            opt("data", "DATA")
                .default_value("data")
                .value_parser(as_path),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .action(ArgAction::Version)
                .help("Print version"),
        )
        .subcommands([
            Command::new("import")
                .about("import a raw link dump into sources.toml")
                .arg(positional("dump", "DUMP")),
            scrape,
            Command::new("serve")
                .about("serve the local viewer")
                .arg(port_option())
                .arg(flag("no-browser")),
            Command::new("demo")
                .about("serve the viewer over synthetic data in a temporary directory")
                .arg(port_option())
                .arg(flag("no-browser")),
            Command::new("doctor").about("check catalogue metadata and cached cover coverage"),
            Command::new("snapshot").about("save a bounded metadata snapshot (excludes covers)"),
            Command::new("reindex")
                .about("index the newest duplicate listings from local captures"),
            Command::new("restore")
                .about("restore metadata from a local snapshot")
                .arg(positional("archive", "ARCHIVE")),
            Command::new("prepare-covers")
                .about("resize and fingerprint cached covers locally")
                .arg(
                    num("limit", "LIMIT")
                        .default_value("1000")
                        .value_parser(positive_int),
                ),
            Command::new("find-photo")
                .about("list albums whose cover looks like an image file")
                .arg(positional("image", "IMAGE"))
                .arg(
                    num("limit", "N")
                        .default_value("8")
                        .value_parser(match_limit)
                        .help("most albums to list, 1 to 20"),
                ),
            export,
        ])
}

/// The parsed command line: the command's own options, then the global ones.
struct Args(ArgMatches);

impl Args {
    fn get<T: Clone + Send + Sync + 'static>(&self, key: &str) -> Option<T> {
        let own = self.0.subcommand().map(|(_, own)| own);
        own.into_iter()
            .chain([&self.0])
            .find_map(|m| match m.try_get_one::<T>(key) {
                Ok(value) => value.cloned(),
                // A global option, looked for in the command's own matches.
                Err(clap::parser::MatchesError::UnknownArgument { .. }) => None,
                Err(e) => panic!("argument {key}: {e}"),
            })
    }
    fn str(&self, key: &str) -> String {
        if key == "command" {
            return self.0.subcommand_name().unwrap_or_default().into();
        }
        self.get(key).unwrap_or_default()
    }
    fn path(&self, key: &str) -> PathBuf {
        PathBuf::from(self.str(key))
    }
    fn int(&self, key: &str) -> i128 {
        self.get(key).unwrap_or_default()
    }
    fn float(&self, key: &str) -> f64 {
        self.get(key).unwrap_or_default()
    }
    fn flag(&self, key: &str) -> bool {
        self.get(key).unwrap_or_default()
    }
}

/// Text encoding of a raw dump, by byte order mark.
fn sniff_encoding(blob: &[u8]) -> &'static str {
    if blob.starts_with(&[0xff, 0xfe]) {
        "utf-16-le"
    } else if blob.starts_with(&[0xfe, 0xff]) {
        "utf-16-be"
    } else if blob.starts_with(&[0xef, 0xbb, 0xbf]) {
        "utf-8-sig"
    } else {
        "utf-8"
    }
}

fn cmd_import(args: &Args) -> i32 {
    let dump = args.path("dump");
    let sources_path = args.path("sources");
    if !dump.exists() {
        out::err(&format!(
            "error: dump file not found: {}",
            pyerr::path_str(&dump)
        ));
        return 1;
    }
    let blob = match pyerr::read_bytes(&dump) {
        Ok(blob) => blob,
        Err(fail) => {
            out::err(&format!("error: {}", fail.message));
            return 1;
        }
    };
    let encoding = sniff_encoding(&blob);
    let decoded = match encoding {
        "utf-16-le" => pyerr::decode_utf16(&blob, false, encoding),
        "utf-16-be" => pyerr::decode_utf16(&blob, true, encoding),
        "utf-8-sig" => pyerr::decode_utf8(&blob[3..]),
        _ => pyerr::decode_utf8(&blob),
    };
    let text = match decoded {
        Ok(text) => text,
        Err(failure) => {
            out::err(&format!(
                "error: could not read {} as {encoding} ({} at byte {}); re-save it as UTF-8 and try again",
                pyerr::path_str(&dump),
                failure.reason,
                failure.start
            ));
            return 1;
        }
    };
    let raws = parse_dump(&text);
    let result = load_sources(&sources_path).and_then(|existing| {
        let (merged, report) = merge_sources(&existing, &raws);
        write_sources(&sources_path, &merged)?;
        Ok(report)
    });
    let report = match result {
        Ok(report) => report,
        Err(e) => {
            out::err(&format!("error: {e}"));
            return 1;
        }
    };
    let mut platforms = String::new();
    if !report.shops_by_platform.is_empty() {
        let mut items = report.shops_by_platform.clone();
        items.sort();
        let detail: Vec<String> = items.iter().map(|(k, v)| format!("{k} {v}")).collect();
        platforms = format!(" ({})", detail.join(", "));
    }
    out::out(&format!(
        "{} rows read, {} invalid\nyupoo sellers: {} new, {} known\nshops: {} new, {} known{platforms}\nwrote {}",
        report.rows,
        report.invalid,
        report.yupoo_new,
        report.yupoo_known,
        report.shops_new,
        report.shops_known,
        pyerr::path_str(&sources_path)
    ))
    .unwrap_or_else(|e| out::uncaught(&e));
    if !report.collisions.is_empty() {
        out::err(&format!(
            "{} id collision(s), second URL not stored:",
            report.collisions.len()
        ));
        for (key, kept, dropped) in &report.collisions {
            out::err(&format!("  {key}: kept {kept}, dropped {dropped}"));
        }
    }
    0
}

fn run_timestamp() -> String {
    #[cfg(feature = "fixture-transport")]
    if let Ok(fixed) = std::env::var("LAMP_FIXED_NOW") {
        return fixed;
    }
    jiff::ZonedDateTime::now()
        .strftime("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
}

pub fn report_line(s: &SellerSummary, reported: usize, total: usize, dry_run: bool) -> String {
    let verb = if dry_run { "would scrape" } else { "scraped" };
    let mut line = format!(
        "[{reported}/{total}] {}: {} {verb}, {} skipped, {} failed",
        s.seller, s.scraped, s.skipped, s.failed
    );
    if s.skipped_old != 0 {
        line.push_str(&format!(", {} too old", s.skipped_old));
    }
    if s.thumbs_refetched != 0 {
        line.push_str(&format!(", {} covers fetched again", s.thumbs_refetched));
    }
    if !s.new_ids.is_empty() {
        line.push_str(&format!(", {} new", s.new_ids.len()));
    }
    if !s.returned_ids.is_empty() {
        line.push_str(&format!(", {} returned", s.returned_ids.len()));
    }
    if !s.recounted_ids.is_empty() {
        line.push_str(&format!(", {} recounted", s.recounted_ids.len()));
    }
    if !s.partly_scanned_ids.is_empty() {
        line.push_str(&format!(", {} partly scanned", s.partly_scanned_ids.len()));
    }
    // The first 5 IDs of each list, so a scheduled run's log names them.
    for (label, ids) in [
        ("failed", &s.failed_ids),
        ("new", &s.new_ids),
        ("returned", &s.returned_ids),
        ("recounted", &s.recounted_ids),
        ("partly scanned", &s.partly_scanned_ids),
    ] {
        if !ids.is_empty() {
            let shown = ids.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
            let more = if ids.len() > 5 { ", ..." } else { "" };
            line.push_str(&format!(" ({label} albums: {shown}{more})"));
        }
    }
    if s.thumbs_blocked {
        line.push_str(" (thumbs blocked - hotlink protection?)");
    }
    if s.scraped != 0 && s.dated == 0 && !dry_run {
        line.push_str(" (no publish dates found - Yupoo markup change?)");
    }
    if s.refreshed != 0 || s.deferred != 0 {
        line.push_str(&format!(
            " ({} details refreshed, {} deferred by budget)",
            s.refreshed, s.deferred
        ));
    }
    if !s.error.is_empty() {
        line.push_str(&format!("  ERROR: {}", s.error));
    }
    line
}

fn cmd_scrape(args: &Args) -> i32 {
    let sources = match load_sources(&args.path("sources")) {
        Ok(sources) => sources,
        Err(e) => {
            out::err(&format!("error: {e}"));
            return 1;
        }
    };
    if !sources.values().any(|s| s.kind == "yupoo") {
        out::err("error: no sources - run 'lamp import' first");
        return 1;
    }
    let seller = args.str("seller");
    let only: Option<HashSet<String>> = if seller.is_empty() {
        None
    } else {
        if !sources.contains_key(&format!("yupoo:{seller}")) {
            out::err(&format!("error: unknown seller: {seller}"));
            return 1;
        }
        Some(HashSet::from([seller]))
    };
    let run_ts = run_timestamp();
    let total = match &only {
        Some(only) => only.len(),
        None => sources
            .values()
            .filter(|s| s.kind == "yupoo" && !s.skip)
            .count(),
    };
    let dry_run = args.flag("dry-run");
    let options = Options {
        full: args.flag("full"),
        max_age_days: args.int("max-age-days"),
        dry_run,
        refresh_days: args.int("refresh-details-days"),
        refresh_limit: args.int("refresh-limit"),
        retry_failed: args.flag("retry-failed"),
        run_rows: !args.str("notify").is_empty(),
    };
    let interrupt = Arc::new(Interrupt::default());
    {
        let interrupt = interrupt.clone();
        let _ = ctrlc::set_handler(move || interrupt.press());
    }
    let mut reported = 0usize;
    let mut on_done = |s: &SellerSummary| {
        reported += 1;
        out::out_flush(&report_line(s, reported, total, dry_run))
            .unwrap_or_else(|e| out::uncaught(&e));
    };
    let outcome = scrape::scrape_all(
        &sources,
        &args.path("data"),
        &run_ts,
        &options,
        args.int("workers"),
        args.float("rps"),
        only.as_ref(),
        None,
        &mut on_done,
        &interrupt,
    );
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(e) => {
            out::err(&format!("error: {e}"));
            return 1;
        }
    };
    if outcome.interrupted {
        let saved = if dry_run {
            ""
        } else {
            "; finished work is saved and index.json is written"
        };
        out::err(&format!(
            "interrupted after {reported} of {total} sellers{saved}"
        ));
        return 130;
    }
    let errored = outcome
        .summaries
        .iter()
        .filter(|s| !s.error.is_empty())
        .count();
    let mut skipped: Vec<&str> = sources
        .values()
        .filter(|s| s.kind == "yupoo" && s.skip && only.as_ref().is_none_or(|o| !o.contains(&s.id)))
        .map(|s| s.id.as_str())
        .collect();
    skipped.sort();
    let tail = if skipped.is_empty() {
        String::new()
    } else {
        format!(", {} skipped ({})", skipped.len(), skipped.join(", "))
    };
    let dry_tag = if dry_run {
        " [DRY RUN - no files written]"
    } else {
        ""
    };
    out::out(&format!(
        "{} sellers, {errored} with errors{tail}{dry_tag}",
        outcome.summaries.len()
    ))
    .unwrap_or_else(|e| out::uncaught(&e));
    let sent = outcome
        .rows
        .map_err(|e| e.to_string())
        .and_then(|rows| notify(&args.str("notify"), rows));
    if let Err(e) = sent {
        out::err(&format!("error: --notify: {e}"));
        return 1;
    }
    i32::from(errored > 0)
}

/// Run `command` in the system shell with `rows` as JSON on its stdin, or
/// nothing when there are none. Scraped text reaches the command only on
/// stdin, never in its arguments or the shell's command line.
pub fn notify(command: &str, rows: Vec<pyjson::Map>) -> Result<(), String> {
    use std::io::Write;
    if rows.is_empty() {
        return Ok(());
    }
    let rows = pyjson::Value::Array(rows.into_iter().map(pyjson::Value::Object).collect());
    let json = pyjson::dumps(&rows, Style::Indent(2)) + "\n";
    // The command shares stdout, so lamp's own lines come first.
    let _ = std::io::stdout().flush();
    let mut child = shell(command)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the command: {e}"))?;
    // Dropping stdin closes it. A command may exit without reading it all.
    let written = child
        .stdin
        .take()
        .map_or(Ok(()), |mut stdin| stdin.write_all(json.as_bytes()));
    let status = child
        .wait()
        .map_err(|e| format!("could not wait for the command: {e}"))?;
    if !status.success() {
        return Err(format!("the command failed ({status})"));
    }
    match written {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => {
            Err(format!("could not write to the command: {e}"))
        }
        _ => Ok(()),
    }
}

/// `command` run by the system shell: `/bin/sh -c` or `%ComSpec% /d /s /c`.
fn shell(command: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let comspec = std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into());
        let mut shell = std::process::Command::new(comspec);
        // cmd strips the outer quotes that /s marks and runs the rest as typed.
        shell.raw_arg(format!("/d /s /c \"{command}\""));
        shell
    }
    #[cfg(not(windows))]
    {
        let mut shell = std::process::Command::new("/bin/sh");
        shell.arg("-c").arg(command);
        shell
    }
}

fn maintenance(command: &str, args: &Args) -> Result<i32, Error> {
    let data = args.path("data");
    match command {
        "doctor" => {
            let report = crate::maintenance::diagnose(&data);
            out::out(&pyjson::dumps(&report.clone().into(), Style::Indent(2)))
                .map_err(out::os_error)?;
            let issues = report
                .get("issues")
                .and_then(|v| v.as_array())
                .is_some_and(|i| !i.is_empty());
            return Ok(i32::from(issues));
        }
        "reindex" => {
            let (indexed, duplicates) = crate::storage::reindex(&data)?;
            let map = crate::obj! { "indexed" => pyjson::Value::Int(indexed), "duplicates" => duplicates };
            out::out(&pyjson::dumps(&map.into(), Style::Default)).map_err(out::os_error)?;
        }
        "snapshot" => {
            let path = crate::maintenance::snapshot(&data)?;
            out::out(&pyerr::path_str(&path)).map_err(out::os_error)?;
        }
        "export" if !args.str("site").is_empty() => {
            if !args.str("since").is_empty() {
                return Err(Error::Value(
                    "--site writes the whole catalogue; leave out --since".into(),
                ));
            }
            let site = args.path("site");
            let written = crate::maintenance::export_site(&data, &site)?;
            let missing = match written.missing_covers {
                0 => String::new(),
                n => format!(" ({n} not cached, shown without a cover)"),
            };
            out::out(&format!(
                "Wrote {}: the viewer, {} sellers and {} covers{missing}. Put the folder on a web host and open its address; anyone who can open it can read the catalogue.",
                pyerr::path_str(&site),
                written.sellers,
                written.covers
            ))
            .map_err(out::os_error)?;
        }
        "export" => {
            let since = args.str("since");
            let since = Some(since.as_str()).filter(|s| !s.is_empty());
            let rows = crate::maintenance::export_rows(&data, since)?;
            if args.str("format") == "json" {
                let rows = rows.into_iter().map(pyjson::Value::Object).collect();
                out::out(&pyjson::dumps(
                    &pyjson::Value::Array(rows),
                    Style::Indent(2),
                ))
            } else {
                out::out_bytes(crate::maintenance::export_csv(&rows).as_bytes())
            }
            .map_err(out::os_error)?;
        }
        "find-photo" => {
            let limit = usize::try_from(args.int("limit")).unwrap_or(8);
            let report = crate::maintenance::find_photo(&data, &args.path("image"), limit)?;
            out::out(&pyjson::dumps(&report.into(), Style::Indent(2))).map_err(out::os_error)?;
        }
        "restore" => {
            let previous = crate::maintenance::restore(&data, &args.path("archive"))?;
            let shown =
                previous.map_or_else(|| "empty target".to_string(), |p| pyerr::path_str(&p));
            out::out(&format!(
                "Metadata restored. Previous files saved at: {shown}"
            ))
            .map_err(out::os_error)?;
        }
        _ => {
            let report = crate::maintenance::prepare_covers(&data, args.int("limit"))?;
            out::out(&pyjson::dumps(&report.clone().into(), Style::Indent(2)))
                .map_err(out::os_error)?;
            let errors = report
                .get("errors")
                .and_then(|v| v.as_array())
                .is_some_and(|e| !e.is_empty());
            return Ok(i32::from(errors));
        }
    }
    Ok(0)
}

fn cmd_serve(args: &Args) -> i32 {
    let data = args.path("data");
    if !data.join("index.json").exists() {
        out::err(&format!(
            "note: no index.json under {} - the viewer will show an empty catalog until 'lamp scrape' writes one",
            pyerr::path_str(&data)
        ));
    }
    let Some(server) = listen(&data, args) else {
        return 1;
    };
    crate::serve::serve(
        &server,
        !args.flag("no-browser"),
        interrupt_flag().unwrap_or_default(),
    );
    0
}

/// Bind the viewer's port for `data`, reporting a failure.
fn listen(data: &std::path::Path, args: &Args) -> Option<crate::serve::Server> {
    let port = u16::try_from(args.int("port")).unwrap_or(8000);
    crate::serve::make_server(crate::serve::Viewer::Embedded, data, port)
        .inspect_err(|e| out::err(&format!("error: could not listen on 127.0.0.1:{port}: {e}")))
        .ok()
}

/// Set when the user presses Ctrl+C, instead of ending the process.
fn interrupt_flag() -> Result<Arc<AtomicBool>, ctrlc::Error> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))?;
    Ok(stop)
}

/// Sellers and albums per seller that `lamp demo` writes.
const DEMO_SELLERS: usize = 5;
const DEMO_ALBUMS: usize = 2000;

/// `lamp demo`: synthetic data in a new directory under the system temporary
/// directory, never `--data`, served until Ctrl+C and then removed.
fn cmd_demo(args: &Args) -> i32 {
    let dir = std::env::temp_dir().join(crate::storage::temp_name("lamp-demo-", ""));
    // Bind first, so a busy port fails before anything is written.
    let Some(server) = listen(&dir, args) else {
        return 1;
    };
    // Ctrl+C whilst writing stops before serving and still removes the data.
    // Without the handler it would end the process first, so that is an error.
    let stop = match interrupt_flag() {
        Ok(stop) => stop,
        Err(e) => {
            out::err(&format!("error: could not handle Ctrl+C: {e}"));
            return 1;
        }
    };
    // A directory this could not create is not ours to remove.
    if let Err(e) = std::fs::create_dir(&dir) {
        out::err(&format!("error: could not write demo data: {e}"));
        return 1;
    }
    let code = match crate::demo::write(&dir, DEMO_SELLERS, DEMO_ALBUMS, DEMO_ALBUMS) {
        Ok(covers) => {
            if let Err(e) = out::out(&format!(
                "demo catalogue: {DEMO_SELLERS} sellers x {DEMO_ALBUMS} albums ({covers} covers) in {}; removed when you stop",
                pyerr::path_str(&dir)
            )) {
                remove_demo(&dir);
                out::uncaught(&e);
            }
            if !stop.load(Ordering::SeqCst) {
                crate::serve::serve(&server, !args.flag("no-browser"), stop);
            }
            0
        }
        Err(e) => {
            out::err(&format!("error: could not write demo data: {e}"));
            1
        }
    };
    remove_demo(&dir);
    code
}

/// The output promised the data would go, so a leftover is named.
fn remove_demo(dir: &std::path::Path) {
    if let Err(e) = std::fs::remove_dir_all(dir) {
        out::err(&format!(
            "warning: could not remove {}: {e}",
            pyerr::path_str(dir)
        ));
    }
}

/// `lamp` alone, which is what a double-click on `lamp.exe` runs: the next
/// steps instead of a missing-command error.
fn next_steps() -> i32 {
    out::out_raw(concat!(
        "lamp ",
        env!("CARGO_PKG_VERSION"),
        ": every Yupoo catalogue you follow on one searchable page.\n",
        "Run it from a terminal, in the folder that should hold your data:\n",
        "\n",
        "  lamp demo                 see the viewer on synthetic data\n",
        "  lamp import links.csv     add the Yupoo links you follow\n",
        "  lamp scrape               collect their catalogues (slow on purpose)\n",
        "  lamp serve                browse them\n",
        "\n",
        "lamp --help lists every command.\n",
    ));
    // shortcut: waits whenever stdin is a console, so `lamp` typed in a
    // Windows terminal also waits for Enter. Telling a double-click apart
    // needs GetConsoleProcessList, which no dependency wraps safely and
    // this crate's forbid(unsafe_code) rules out calling directly.
    #[cfg(windows)]
    {
        use std::io::{IsTerminal, Write};
        if std::io::stdin().is_terminal() {
            out::out_raw("\nPress Enter to close.\n");
            let _ = std::io::stdout().flush();
            let _ = std::io::stdin().read_line(&mut String::new());
        }
    }
    0
}

/// Run the CLI with arguments after the program name; returns the exit code.
pub fn main(argv: &[String]) -> i32 {
    if argv.is_empty() {
        return next_steps();
    }
    let parsed = build_parser()
        .try_get_matches_from(std::iter::once("lamp").chain(argv.iter().map(String::as_str)));
    let args = match parsed {
        Ok(matches) => Args(matches),
        Err(error) => {
            let text = error.render().to_string();
            if error.use_stderr() {
                out::err_raw(&text);
            } else {
                out::out_raw(&text);
            }
            return error.exit_code();
        }
    };
    let command = args.str("command");
    match command.as_str() {
        "import" => cmd_import(&args),
        "scrape" => cmd_scrape(&args),
        "serve" => cmd_serve(&args),
        "demo" => cmd_demo(&args),
        other => match maintenance(other, &args) {
            Ok(code) => code,
            Err(e) => {
                out::err(&format!("error: {e}"));
                1
            }
        },
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Runs the named unit test again in a child process whose local time is
    /// UTC+05:30, a zone with no daylight saving, and reports which side this is.
    #[cfg(unix)]
    pub(crate) fn rerun_in_utc_plus_0530(test: &str) -> bool {
        if std::env::var_os("LAMP_TZ_CHILD").is_some() {
            return true;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test])
            .env("TZ", "IST-5:30")
            .env("LAMP_TZ_CHILD", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed"),
            "{stdout}"
        );
        false
    }

    #[test]
    fn run_timestamp_is_local_time_with_its_offset() {
        let stamp = run_timestamp();
        let shape = regex::Regex::new(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d$").unwrap();
        assert!(shape.is_match(&stamp), "{stamp}");
        #[cfg(unix)]
        if rerun_in_utc_plus_0530("cli::tests::run_timestamp_is_local_time_with_its_offset") {
            assert!(stamp.ends_with("+05:30"), "{stamp}");
        }
    }
}
