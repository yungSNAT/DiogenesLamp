// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Scrape Yupoo sellers into catalogue shards.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::age::is_stale;
use crate::covers::cover_facts;
use crate::errors::{Error, Result};
use crate::http::{self, Client, FetchError, RateLimiter};
use crate::ingest::{Source, Sources};
use crate::isodate;
use crate::links::extract_links;
use crate::locking::CatalogueLock;
use crate::parse::{AlbumStub, parse_album_list, parse_album_page, price_evidence};
use crate::pyerr;
use crate::pyjson::{Map, Value, py_eq};
use crate::pystr;
use crate::storage::{
    Shard, load_shard, merge_album, read_pending, read_run, record_change, write_index,
    write_pending, write_run, write_shard,
};

/// Stored album text: enough for product codes, sizes and batch names.
pub const DESCRIPTION_LIMIT: usize = 500;
const BLOCKED_STREAK_LIMIT: u32 = 3;
const THUMB_BLOCKED_STREAK_LIMIT: u32 = 3;
/// Distinct unparsed album links before reporting ambiguous listing coverage.
const LAYOUT_CHANGE_MIN_LINKS: usize = 3;
/// A listing missing this many known albums (listed before, or failed and
/// unrecorded) marks only those also missing from the previous such listing
/// gone, and none if a block aborts its run. shortcut: a fixed count, not a
/// page-aware check; it misses a truncated listing that drops fewer albums
/// (those come back when listed again). Upgrade if Yupoo exposes a page or
/// album total to compare against.
const SHORT_LISTING_MIN_LOST: usize = 5;
const FLUSH_EVERY: usize = 100;

/// What a scraper needs from an HTTP client; tests substitute fakes.
pub trait ScrapeClient: Send {
    fn get(&self, url: &str, referer: Option<&str>) -> std::result::Result<String, FetchError>;
    fn download(
        &self,
        url: &str,
        dest: &Path,
        referer: Option<&str>,
    ) -> std::result::Result<bool, FetchError>;
}

impl ScrapeClient for Client {
    fn get(&self, url: &str, referer: Option<&str>) -> std::result::Result<String, FetchError> {
        Client::get(self, url, referer).map(|page| page.text())
    }

    fn download(
        &self,
        url: &str,
        dest: &Path,
        referer: Option<&str>,
    ) -> std::result::Result<bool, FetchError> {
        Client::download(self, url, dest, referer)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SellerSummary {
    pub seller: String,
    pub scraped: i64,
    pub skipped: i64,
    pub failed: i64,
    pub error: String,
    pub failed_ids: Vec<String>,
    pub thumbs_blocked: bool,
    pub dated: i64,
    pub skipped_old: i64,
    pub thumbs_refetched: i64,
    pub refreshed: i64,
    pub deferred: i64,
    pub listing_complete: bool,
    pub resolved_ids: Vec<String>,
    pub short_missing: Vec<String>,
    /// Albums this run stored for the first time, albums listed again after
    /// being marked gone, and albums whose listing shows a different photo
    /// count. Reported, not written to the run summary: the shard's
    /// `first_seen` and history already record them.
    pub new_ids: Vec<String>,
    pub returned_ids: Vec<String>,
    pub recounted_ids: Vec<String>,
    /// Albums with a word whose purchase-link scan stopped early, so links
    /// later in that word, or an agent link's wrapped URL, were not stored.
    /// Reported and kept in the run summary.
    pub partly_scanned_ids: Vec<String>,
}

impl SellerSummary {
    pub fn new(seller: &str) -> SellerSummary {
        SellerSummary {
            seller: seller.into(),
            ..Default::default()
        }
    }

    /// `dataclasses.asdict`.
    pub fn as_map(&self) -> Map {
        let strings =
            |items: &[String]| Value::Array(items.iter().map(|s| Value::Str(s.clone())).collect());
        crate::obj! {
            "seller" => self.seller.as_str(),
            "scraped" => self.scraped,
            "skipped" => self.skipped,
            "failed" => self.failed,
            "error" => self.error.as_str(),
            "failed_ids" => strings(&self.failed_ids),
            "thumbs_blocked" => self.thumbs_blocked,
            "dated" => self.dated,
            "skipped_old" => self.skipped_old,
            "thumbs_refetched" => self.thumbs_refetched,
            "refreshed" => self.refreshed,
            "deferred" => self.deferred,
            "listing_complete" => self.listing_complete,
            "resolved_ids" => strings(&self.resolved_ids),
            "short_missing" => strings(&self.short_missing),
            "partly_scanned_ids" => strings(&self.partly_scanned_ids),
        }
    }
}

/// Unchanged-detail refreshes shared across one run.
pub struct RefreshBudget {
    remaining: Mutex<i128>,
}

impl RefreshBudget {
    pub fn new(limit: i128) -> RefreshBudget {
        RefreshBudget {
            remaining: Mutex::new(limit),
        }
    }

    pub fn take(&self) -> bool {
        let mut remaining = self.remaining.lock().unwrap_or_else(|e| e.into_inner());
        if *remaining <= 0 {
            return false;
        }
        *remaining -= 1;
        true
    }
}

fn utc_micros(text: &str) -> Option<i64> {
    let (local, offset) = isodate::parse_datetime(text)?;
    Some(local - offset.unwrap_or(0))
}

pub fn detail_due(album: &Map, run_ts: &str, days: i128) -> bool {
    if days <= 0 {
        return false;
    }
    let previous = match album.get("updated_at") {
        None => utc_micros(""),
        Some(Value::Str(s)) => utc_micros(s),
        Some(_) => None,
    };
    match (previous, utc_micros(run_ts)) {
        (Some(previous), Some(now)) => {
            i128::from(now - previous) >= days.saturating_mul(86_400).saturating_mul(1_000_000)
        }
        _ => true,
    }
}

/// Run-wide options, as the CLI passes them.
#[derive(Clone, Debug)]
pub struct Options {
    pub full: bool,
    pub max_age_days: i128,
    pub dry_run: bool,
    pub refresh_days: i128,
    pub refresh_limit: i128,
    pub retry_failed: bool,
    /// Read the albums the run stored or changed (`--notify`'s rows) before
    /// the catalogue lock is released, so a later run cannot change them.
    pub run_rows: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            full: false,
            max_age_days: 0,
            dry_run: false,
            refresh_days: 0,
            refresh_limit: 100,
            retry_failed: false,
            run_rows: false,
        }
    }
}

enum ListingError {
    Stopped,
    Message(String),
}

/// `re.findall(r"/albums/\d+", html)` as a set.
fn album_links(html: &str) -> HashSet<String> {
    let mut links = HashSet::new();
    let mut rest = html;
    while let Some(pos) = rest.find("/albums/") {
        let after = &rest[pos + 8..];
        let digits: String = after
            .chars()
            .take_while(|&c| pystr::decimal_value(c).is_some())
            .collect();
        if digits.is_empty() {
            rest = &rest[pos + 1..];
        } else {
            rest = &after[digits.len()..];
            links.insert(format!("/albums/{digits}"));
        }
    }
    links
}

fn fetch_all_stubs(
    client: &dyn ScrapeClient,
    source: &Source,
    data_dir: &Path,
    save_debug: bool,
    stop: &AtomicBool,
) -> std::result::Result<Vec<AlbumStub>, ListingError> {
    let base = source.url_str();
    let mut stubs: Vec<AlbumStub> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut page = 1u64;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Err(ListingError::Stopped);
        }
        let url = format!("{base}/albums?page={page}");
        let html = client.get(&url, Some(&base)).map_err(|e| match e {
            FetchError::Blocked(_) => ListingError::Message(format!("blocked: {e}")),
            FetchError::Fetch(m) => ListingError::Message(m),
        })?;
        let batch = parse_album_list(&html);
        let listed: HashSet<String> = batch.iter().map(|s| format!("/albums/{}", s.id)).collect();
        let unreadable = album_links(&html).difference(&listed).count();
        let ambiguous = unreadable >= LAYOUT_CHANGE_MIN_LINKS;
        let mut saved_note = String::new();
        if save_debug && (ambiguous || (batch.is_empty() && page == 1)) {
            let debug = data_dir
                .join("debug")
                .join(format!("{}-page{page}.html", source.id));
            if let Some(parent) = debug.parent() {
                let _ = pyerr::mkdir_parents(parent);
            }
            let _ = std::fs::write(&debug, pyerr::native_newlines(&html));
            saved_note = format!("; saved {}", pyerr::path_str(&debug));
        }
        if ambiguous {
            return Err(ListingError::Message(format!(
                "page {page} of {} has {unreadable} album links that could not be parsed{saved_note} - possible Yupoo layout change?",
                source.id
            )));
        }
        if batch.is_empty() {
            if page == 1 {
                return Err(ListingError::Message(format!(
                    "page 1 of {} contains no albums{saved_note} - empty or closed seller?",
                    source.id
                )));
            }
            return Ok(stubs);
        }
        let new: Vec<AlbumStub> = batch
            .into_iter()
            .filter(|s| !seen.contains(&s.id))
            .collect();
        if new.is_empty() {
            return Ok(stubs);
        }
        for stub in &new {
            seen.insert(stub.id.clone());
        }
        stubs.extend(new);
        page += 1;
    }
}

fn get_str<'a>(map: Option<&'a Map>, key: &str) -> Option<&'a Value> {
    map.and_then(|m| m.get(key))
}

#[allow(clippy::too_many_arguments)]
pub fn scrape_seller(
    client: &dyn ScrapeClient,
    source: &Source,
    data_dir: &Path,
    run_ts: &str,
    options: &Options,
    stop: &AtomicBool,
    budget: &RefreshBudget,
) -> Result<SellerSummary> {
    let mut summary = SellerSummary::new(&source.id);
    scrape_into(
        &mut summary,
        &mut 0,
        client,
        source,
        data_dir,
        run_ts,
        options,
        stop,
        budget,
    )?;
    Ok(summary)
}

/// `scrape_seller` into a caller's summary, so a seller that fails still
/// has it. `saved_new` counts the leading `new_ids` the last shard save stored.
#[allow(clippy::too_many_arguments)]
fn scrape_into(
    summary: &mut SellerSummary,
    saved_new: &mut usize,
    client: &dyn ScrapeClient,
    source: &Source,
    data_dir: &Path,
    run_ts: &str,
    options: &Options,
    stop: &AtomicBool,
    budget: &RefreshBudget,
) -> Result<()> {
    let loaded = load_shard(data_dir, &source.id).and_then(|shard| {
        let run = read_run(data_dir, &source.id)?;
        let failures: BTreeSet<String> = match run.get("failed_ids") {
            Some(Value::Array(ids)) => ids.iter().map(Value::py_str).collect(),
            _ => BTreeSet::new(),
        };
        let missing_before: HashSet<String> = match run.get("short_missing") {
            Some(Value::Array(ids)) => ids.iter().map(Value::py_str).collect(),
            _ => HashSet::new(),
        };
        Ok((shard, failures, missing_before))
    });
    let (mut shard, pending_failures, missing_before) = match loaded {
        Ok(found) => found,
        Err(Error::Value(message)) => {
            summary.error = message;
            return Ok(());
        }
        Err(other) => return Err(other),
    };
    let today = isodate::parse_date(pystr::prefix_chars(run_ts, 10)).ok_or_else(|| {
        Error::Value(format!(
            "Invalid isoformat string: {}",
            pystr::repr(pystr::prefix_chars(run_ts, 10))
        ))
    })?;
    let base = source.url_str();
    let mut blocked_streak = 0;
    let mut thumb_blocked_streak = 0;
    let mut thumbs_disabled = false;

    let stubs = match fetch_all_stubs(client, source, data_dir, !options.dry_run, stop) {
        Ok(stubs) => stubs,
        Err(ListingError::Stopped) => {
            summary.error = "interrupted before the listing finished".into();
            return Ok(());
        }
        Err(ListingError::Message(message)) => {
            summary.error = message;
            return Ok(());
        }
    };
    let listed: HashMap<String, i128> = stubs
        .iter()
        .map(|s| (s.id.clone(), s.photo_count))
        .collect();
    // Known albums the listing no longer carries, including failed ones that
    // never got a record.
    let missing: HashSet<String> = shard
        .iter()
        .filter(|(_, album)| !album.get("gone_since").is_some_and(Value::truthy))
        .map(|(id, _)| id)
        .chain(&pending_failures)
        .filter(|id| !listed.contains_key(*id))
        .cloned()
        .collect();
    let mut stubs = stubs;
    if options.refresh_days != 0 {
        stubs.sort_by_key(
            |stub| match shard.get(&stub.id).and_then(|a| a.get("updated_at")) {
                Some(Value::Str(s)) => s.clone(),
                _ => String::new(),
            },
        );
    }
    let mut scraped_since_flush = 0;
    for stub in &stubs {
        if stop.load(Ordering::SeqCst) {
            summary.error = "interrupted; progress so far is saved".into();
            break;
        }
        let old = shard.get(&stub.id).cloned();
        let old_ref = old.as_ref();
        if options.retry_failed && !pending_failures.contains(&stub.id) {
            summary.skipped += 1;
            continue;
        }
        let thumb_rel = format!("thumbs/{}/{}.jpg", source.id, stub.id);
        let thumb_path = pyerr::join_rel(data_dir, &thumb_rel);
        let held_url = get_str(old_ref, "thumb_url")
            .cloned()
            .unwrap_or_else(|| stub.thumb_url.as_str().into());
        let mut cover_current = stub.thumb_url.is_empty()
            || (py_eq(&held_url, &Value::Str(stub.thumb_url.clone())) && thumb_path.exists());
        let known = match get_str(old_ref, "date_published") {
            Some(Value::Str(s)) => s.as_str(),
            _ => "",
        };
        let numeric_id = pystr::parse_int(&stub.id).map_or(u128::MAX, |n| n.max(0) as u128);
        if is_stale(numeric_id, options.max_age_days, today, known) {
            summary.skipped_old += 1;
            continue;
        }
        let unchanged = old_ref.is_some_and(|o| {
            o.get("list_title")
                .is_some_and(|t| py_eq(t, &Value::Str(stub.title.clone())))
                && o.contains_key("price")
        });
        let mut refresh = false;
        if let Some(o) = old_ref
            && unchanged
            && !options.full
            && !options.retry_failed
            && detail_due(o, run_ts, options.refresh_days)
        {
            refresh = budget.take();
            if !refresh {
                summary.deferred += 1;
            }
        }
        let mut fetch_thumb = |url: &str, rel_path: &Path, summary: &mut SellerSummary| -> bool {
            if thumbs_disabled {
                return false;
            }
            match client.download(url, rel_path, Some(&base)) {
                Ok(fetched) => {
                    thumb_blocked_streak = 0;
                    fetched
                }
                Err(FetchError::Blocked(_)) => {
                    thumb_blocked_streak += 1;
                    if thumb_blocked_streak >= THUMB_BLOCKED_STREAK_LIMIT {
                        thumbs_disabled = true;
                        summary.thumbs_blocked = true;
                    }
                    false
                }
                Err(FetchError::Fetch(_)) => false,
            }
        };
        if let Some(o) = old_ref
            && unchanged
            && !options.full
            && !options.retry_failed
            && !refresh
        {
            summary.skipped += 1;
            if !options.dry_run
                && !cover_current
                && fetch_thumb(&stub.thumb_url, &thumb_path, summary)
            {
                summary.thumbs_refetched += 1;
                let mut updated = o.clone();
                updated.insert("thumb".into(), thumb_rel.clone().into());
                updated.insert("thumb_url".into(), stub.thumb_url.clone().into());
                for (k, v) in cover_facts(&thumb_path) {
                    updated.insert(k, v);
                }
                updated.insert("cover_prepared".into(), Value::Int(1));
                shard.insert(stub.id.clone(), updated);
            }
            continue;
        }
        if options.dry_run {
            summary.refreshed += i64::from(refresh);
            summary.scraped += 1;
            continue;
        }
        let album_url = format!("{base}/albums/{}?uid=1", stub.id);
        let detail = match client.get(&album_url, Some(&base)) {
            Ok(text) => {
                blocked_streak = 0;
                parse_album_page(&text)
            }
            Err(FetchError::Blocked(status)) => {
                blocked_streak += 1;
                summary.failed += 1;
                summary.failed_ids.push(stub.id.clone());
                if blocked_streak >= BLOCKED_STREAK_LIMIT {
                    summary.error = format!(
                        "blocked: aborted after {blocked_streak} consecutive ({})",
                        FetchError::Blocked(status)
                    );
                    break;
                }
                continue;
            }
            Err(FetchError::Fetch(_)) => {
                summary.failed += 1;
                summary.failed_ids.push(stub.id.clone());
                continue;
            }
        };
        if detail.title.is_empty() && detail.description.is_empty() {
            summary.failed += 1;
            summary.failed_ids.push(stub.id.clone());
            continue;
        }
        let title = if detail.title.is_empty() {
            stub.title.clone()
        } else {
            detail.title.clone()
        };
        let (links, scanned) =
            extract_links(&format!("{title}\n{}\n{}", stub.title, detail.description));
        if !scanned {
            summary.partly_scanned_ids.push(stub.id.clone());
        }
        let mut evidence = price_evidence(&format!("{title} {}", detail.description));
        if evidence.is_empty() {
            evidence = price_evidence(&format!("{title} {} {}", stub.title, detail.description));
        }
        if !evidence.is_empty() {
            let token = evidence.get("token").map(Value::py_str).unwrap_or_default();
            let field = [
                ("title", &title),
                ("listing title", &stub.title),
                ("description", &detail.description),
            ]
            .into_iter()
            .find(|(_, text)| text.contains(token.as_str()))
            .map_or("album text", |(name, _)| name);
            evidence.insert("field".into(), field.into());
        }
        let price = evidence.get("amount").cloned().unwrap_or(Value::Int(0));
        let mut cover_downloaded = false;
        if !stub.thumb_url.is_empty() && (options.full || !cover_current) {
            cover_current = fetch_thumb(&stub.thumb_url, &thumb_path, summary);
            cover_downloaded = cover_current;
        }
        let thumb_url_value: Value = if cover_current {
            stub.thumb_url.clone().into()
        } else {
            get_str(old_ref, "thumb_url")
                .cloned()
                .unwrap_or_else(|| "".into())
        };
        let link_values: Vec<Value> = links
            .iter()
            .map(|l| {
                Value::Object(
                    crate::obj! { "platform" => l.platform.as_str(), "url" => l.url.as_str() },
                )
            })
            .collect();
        // The card's count, which recounts compare, so a refetch agrees with
        // them; the album page's only when the card shows none.
        let photo_count = if stub.photo_count != 0 {
            stub.photo_count
        } else {
            detail.photo_count
        };
        let mut record = crate::obj! {
            "id" => stub.id.as_str(),
            "seller" => source.id.as_str(),
            "title" => title.as_str(),
            "list_title" => stub.title.as_str(),
            "thumb" => if stub.thumb_url.is_empty() { "" } else { thumb_rel.as_str() },
            "thumb_url" => thumb_url_value,
            "album_url" => album_url.as_str(),
            "links" => link_values,
            "category" => detail.category.as_str(),
            "description" => pystr::prefix_chars(&detail.description, DESCRIPTION_LIMIT),
            "price" => price,
            "price_evidence" => Value::Object(evidence),
            "photo_count" => Value::Int(photo_count),
            "date_published" => detail.date_published.as_str(),
            "date_modified" => detail.date_modified.as_str(),
            "first_seen" => "",
            "updated_at" => "",
        };
        if pystr::char_len(&detail.description) > DESCRIPTION_LIMIT {
            record.insert("description_truncated".into(), true.into());
        }
        if !scanned {
            record.insert("links_truncated".into(), true.into());
        }
        if !stub.thumb_url.is_empty() && cover_current && thumb_path.is_file() {
            for (k, v) in cover_facts(&thumb_path) {
                record.insert(k, v);
            }
            if cover_downloaded {
                record.insert("cover_prepared".into(), Value::Int(1));
            }
        }
        // merge_album reads an empty record as none, so first_seen is this run.
        if old_ref.is_none_or(Map::is_empty) {
            summary.new_ids.push(stub.id.clone());
        }
        let merged = merge_album(old_ref, &record, run_ts);
        shard.insert(stub.id.clone(), merged);
        summary.scraped += 1;
        summary.refreshed += i64::from(refresh);
        summary.resolved_ids.push(stub.id.clone());
        if !detail.date_published.is_empty() || !detail.date_modified.is_empty() {
            summary.dated += 1;
        }
        scraped_since_flush += 1;
        if scraped_since_flush >= FLUSH_EVERY {
            write_shard(data_dir, &source.id, &shard)?;
            scraped_since_flush = 0;
            *saved_new = summary.new_ids.len();
        }
    }
    // A short listing holds back the albums it misses, except those the
    // previous short listing also missed; a blocked run confirms none.
    let blocked = blocked_streak >= BLOCKED_STREAK_LIMIT;
    let (absent, unconfirmed): (HashSet<String>, HashSet<String>) =
        if missing.len() < SHORT_LISTING_MIN_LOST {
            (missing, HashSet::new())
        } else if blocked {
            (HashSet::new(), missing)
        } else {
            missing
                .into_iter()
                .partition(|id| missing_before.contains(id))
        };
    summary.resolved_ids.extend(
        pending_failures
            .iter()
            .filter(|id| absent.contains(*id))
            .cloned(),
    );
    if unconfirmed.is_empty() {
        summary.listing_complete = true;
    } else {
        let note = format!(
            "{} albums missing from the listing are not marked gone until another run confirms it - possible block or layout change?",
            unconfirmed.len()
        );
        summary.error = if summary.error.is_empty() {
            note
        } else {
            format!("{}; {note}", summary.error)
        };
        summary.short_missing = unconfirmed.into_iter().collect();
        summary.short_missing.sort();
    }
    let attempted = stubs.len() as i64 - summary.skipped;
    if attempted > 0 && summary.scraped == 0 && summary.failed > 0 && summary.error.is_empty() {
        summary.error = format!("all {} album fetches failed", summary.failed);
    }
    if !options.dry_run {
        (summary.returned_ids, summary.recounted_ids) =
            stamp_listing(&mut shard, &listed, &absent, run_ts);
        write_shard(data_dir, &source.id, &shard)?;
        *saved_new = summary.new_ids.len();
    }
    Ok(())
}

/// Record which albums the seller's listing still carries, with the photo
/// count each card shows, and mark the confirmed absent ones gone. Returns
/// the gone albums listed again and the albums whose listed count changed.
fn stamp_listing(
    shard: &mut Shard,
    listed: &HashMap<String, i128>,
    absent: &HashSet<String>,
    run_ts: &str,
) -> (Vec<String>, Vec<String>) {
    let mut returned = Vec::new();
    let mut recounted = Vec::new();
    for (album_id, album) in shard.iter_mut() {
        if let Some(&count) = listed.get(album_id) {
            album.insert("last_seen".into(), run_ts.into());
            // A card without a count says nothing about the album's photos.
            if count > 0 {
                let before = album.insert("list_photo_count".into(), Value::Int(count));
                if let Some(Value::Int(before)) = before
                    && before != count
                {
                    recounted.push(album_id.clone());
                    album.insert("photo_count".into(), Value::Int(count));
                    record_change(
                        album,
                        "photo count",
                        Value::Int(before),
                        Value::Int(count),
                        run_ts,
                    );
                }
            }
            let gone = album.shift_remove("gone_since");
            if gone.is_some_and(|g| g.truthy()) {
                returned.push(album_id.clone());
                record_change(
                    album,
                    "availability",
                    "gone".into(),
                    "listed".into(),
                    run_ts,
                );
            }
        } else if absent.contains(album_id) && !album.get("gone_since").is_some_and(Value::truthy) {
            record_change(
                album,
                "availability",
                "listed".into(),
                "gone".into(),
                run_ts,
            );
            album.insert("gone_since".into(), run_ts.into());
        }
    }
    (returned, recounted)
}

/// Ctrl+C presses, counted. The first stops queued sellers and lets running
/// ones save; the second abandons them.
#[derive(Default)]
pub struct Interrupt {
    presses: AtomicUsize,
}

impl Interrupt {
    pub fn press(&self) {
        self.presses.fetch_add(1, Ordering::SeqCst);
    }

    pub fn presses(&self) -> usize {
        self.presses.load(Ordering::SeqCst)
    }
}

pub type ClientFactory = Arc<dyn Fn() -> Box<dyn ScrapeClient> + Send + Sync>;

/// What `scrape_all` returns: summaries, whether the run was interrupted
/// (Python raises KeyboardInterrupt after writing the index), and the rows
/// `Options::run_rows` asked for (none after a dry or interrupted run), or
/// why they could not be read; the scrape's own data is saved either way.
pub struct RunOutcome {
    pub summaries: Vec<SellerSummary>,
    pub interrupted: bool,
    pub rows: Result<Vec<Map>>,
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".into()
    }
}

/// The default limiter: the operator's global cap, a fixed 1 request per
/// second per seller host as a politeness floor, and the shared cover CDN
/// at the global cap (it would otherwise serialise the whole run).
pub fn limiter_settings(rps: f64) -> (f64, f64, Vec<(&'static str, f64)>) {
    (rps, 1.0, vec![("photo.yupoo.com", rps)])
}

/// Scrape every seller, then rewrite the index from the shards on disk.
#[allow(clippy::too_many_arguments)]
pub fn scrape_all(
    sources: &Sources,
    data_dir: &Path,
    run_ts: &str,
    options: &Options,
    workers: i128,
    rps: f64,
    only: Option<&HashSet<String>>,
    client_factory: Option<ClientFactory>,
    on_done: &mut dyn FnMut(&SellerSummary),
    interrupt: &Interrupt,
) -> Result<RunOutcome> {
    let budget = Arc::new(RefreshBudget::new(options.refresh_limit));
    let lock = if options.dry_run {
        None
    } else {
        Some(CatalogueLock::acquire(data_dir)?)
    };
    // `--notify` finds the run's changes by its timestamp, which has
    // 1-second resolution: a run in the same second as an earlier scrape
    // would report that scrape's changes as its own, so it does not start.
    if options.run_rows && !options.dry_run && stamped(data_dir, run_ts) {
        return Err(Error::Busy(format!(
            "a scrape already ran at {run_ts}; run again"
        )));
    }
    // The notifying runs whose rows no command has had, this one included,
    // so the next notifying run reports a run that stops before giving them.
    // shortcut: albums are recorded as their seller's summary arrives, so a
    // seller still running at a second Ctrl+C or a kill leaves its saved
    // albums unreported as new; recording at each shard save would cover it.
    let mut pending = if options.run_rows && !options.dry_run {
        let mut runs = read_pending(data_dir)?;
        runs.entry(run_ts.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        write_pending(data_dir, &runs)?;
        Some(runs)
    } else {
        None
    };
    let factory: ClientFactory = client_factory.unwrap_or_else(|| {
        let (global, host, overrides) = limiter_settings(rps);
        let limiter = Arc::new(RateLimiter::new(
            global,
            host,
            &overrides,
            Arc::new(http::SystemClock::default()),
        ));
        Arc::new(move || Box::new(Client::new(limiter.clone())) as Box<dyn ScrapeClient>)
    });
    let mut sellers: Vec<&Source> = sources.values().filter(|s| s.kind == "yupoo").collect();
    sellers.sort_by(|a, b| a.id.cmp(&b.id));
    let mut shops: Vec<&Source> = sources.values().filter(|s| s.kind == "shop").collect();
    shops.sort_by(|a, b| a.id.cmp(&b.id));
    let to_scrape: Vec<Source> = sellers
        .iter()
        .filter(|s| match only {
            Some(only) => only.contains(&s.id),
            None => !s.skip,
        })
        .map(|s| (*s).clone())
        .collect();

    let stop = Arc::new(AtomicBool::new(false));
    let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(
        to_scrape.clone(),
    )));
    let (sender, receiver) = mpsc::channel::<SellerSummary>();
    let thread_count = usize::try_from(workers.max(0))
        .unwrap_or(usize::MAX)
        .min(to_scrape.len());
    let mut handles = Vec::new();
    for _ in 0..thread_count {
        let (queue, sender, stop, factory, budget) = (
            queue.clone(),
            sender.clone(),
            stop.clone(),
            factory.clone(),
            budget.clone(),
        );
        let data_dir = data_dir.to_path_buf();
        let run_ts = run_ts.to_string();
        let options = options.clone();
        handles.push(std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let Some(source) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front()
                else {
                    return;
                };
                let mut summary = SellerSummary::new(&source.id);
                let mut saved_new = 0;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let client = factory();
                    scrape_into(
                        &mut summary,
                        &mut saved_new,
                        &*client,
                        &source,
                        &data_dir,
                        &run_ts,
                        &options,
                        &stop,
                        &budget,
                    )
                }));
                let failure = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error.to_string()),
                    Err(payload) => Some(panic_text(&*payload)),
                };
                if let Some(text) = failure {
                    eprint_unexpected(&source.id, &text);
                    // The albums a shard save stored are first seen in this
                    // run, so it is the one to report them as new. Failed
                    // fetches stored nothing, so they stay due for a retry.
                    summary.new_ids.truncate(saved_new);
                    summary = SellerSummary {
                        error: format!("unexpected error: {text}"),
                        new_ids: summary.new_ids,
                        failed: summary.failed,
                        failed_ids: summary.failed_ids,
                        ..SellerSummary::new(&source.id)
                    };
                }
                if sender.send(summary).is_err() {
                    return;
                }
            }
        }));
    }
    drop(sender);

    let mut summaries: Vec<SellerSummary> = Vec::new();
    let mut announced = false;
    let mut drain_error: Option<Error> = None;
    loop {
        let presses = interrupt.presses();
        if presses >= 1 && !announced {
            announced = true;
            stop.store(true, Ordering::SeqCst);
            crate::out::err(
                "stopping: running sellers save their progress, Ctrl+C again to abandon them",
            );
        }
        if presses >= 2 {
            break;
        }
        match receiver.recv_timeout(Duration::from_millis(500)) {
            Ok(item) => {
                if !options.dry_run
                    && let Err(e) = write_run(data_dir, &item.seller, run_ts, item.as_map())
                        .and_then(|()| record_new(data_dir, pending.as_mut(), run_ts, &item))
                {
                    drain_error = Some(e);
                    break;
                }
                on_done(&item);
                summaries.push(item);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    stop.store(true, Ordering::SeqCst);
    let interrupted = announced;
    // Workers still running after a second Ctrl+C keep the lock until they
    // finish or the process exits.
    let release = move || {
        for handle in handles {
            let _ = handle.join();
        }
        drop(lock);
    };
    let abandoned = interrupt.presses() >= 2 || drain_error.is_some();
    let mut release = Some(release);
    let mut finish = || {
        if let Some(release) = release.take() {
            if abandoned {
                std::thread::spawn(release);
            } else {
                release();
            }
        }
    };
    if let Some(error) = drain_error {
        finish();
        return Err(error);
    }
    summaries.sort_by(|a, b| a.seller.cmp(&b.seller));
    if !options.dry_run
        && let Err(error) = write_scrape_index(data_dir, &sellers, &shops, run_ts)
    {
        finish();
        return Err(error);
    }
    let rows = match pending {
        Some(runs) if !interrupted => pending_rows(data_dir, &runs),
        _ => Ok(Vec::new()),
    };
    finish();
    Ok(RunOutcome {
        summaries,
        interrupted,
        rows,
    })
}

/// Add `item`'s new albums to this run's entry in `pending`, when it has one.
fn record_new(
    data_dir: &Path,
    pending: Option<&mut Map>,
    run_ts: &str,
    item: &SellerSummary,
) -> Result<()> {
    let Some(runs) = pending.filter(|_| !item.new_ids.is_empty()) else {
        return Ok(());
    };
    if let Some(Value::Object(sellers)) = runs.get_mut(run_ts) {
        let ids = item
            .new_ids
            .iter()
            .map(|id| Value::Str(id.clone()))
            .collect();
        sellers.insert(item.seller.clone(), Value::Array(ids));
    }
    write_pending(data_dir, runs)
}

/// The rows of every run in `pending`, which then has none left to report.
fn pending_rows(data_dir: &Path, pending: &Map) -> Result<Vec<Map>> {
    let mut new = HashSet::new();
    for sellers in pending.values().filter_map(Value::as_object) {
        for (seller, ids) in sellers {
            for id in ids.as_array().into_iter().flatten() {
                new.insert((seller.clone(), id.py_str()));
            }
        }
    }
    let runs: HashSet<String> = pending.keys().cloned().collect();
    let rows = crate::maintenance::run_rows(data_dir, &runs, &new)?;
    write_pending(data_dir, &Map::new())?;
    Ok(rows)
}

/// Whether a scrape already wrote `run_ts`: index.json's `generated_at`, or
/// an album's `updated_at` or history entry in any shard, which a scrape that
/// failed before writing the index leaves behind. A file that cannot be read
/// counts as unstamped; the scrape itself reports it.
fn stamped(data_dir: &Path, run_ts: &str) -> bool {
    let read = |path: &Path| {
        let text = std::fs::read_to_string(path).ok()?;
        crate::pyjson::loads(&text).ok()
    };
    let is_run = |value: Option<&Value>| value.and_then(Value::as_str) == Some(run_ts);
    if read(&data_dir.join("index.json"))
        .is_some_and(|index| is_run(index.as_object().and_then(|i| i.get("generated_at"))))
    {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(data_dir.join("catalog")) else {
        return false;
    };
    entries
        .flatten()
        .filter_map(|entry| read(&entry.path()))
        .any(|shard| {
            shard
                .as_object()
                .and_then(|s| s.get("albums"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .any(|album| {
                    is_run(album.get("updated_at"))
                        || album
                            .get("history")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_object)
                            .any(|entry| is_run(entry.get("at")))
                })
        })
}

fn eprint_unexpected(seller: &str, text: &str) {
    crate::out::err_from_worker(&format!("{seller}: unexpected error\n{text}"));
}

/// The index covers every yupoo source except skipped ones with no shard.
pub fn write_scrape_index(
    data_dir: &Path,
    sellers: &[&Source],
    shops: &[&Source],
    run_ts: &str,
) -> Result<()> {
    let mut seller_meta = Vec::new();
    for source in sellers {
        if source.skip && load_shard(data_dir, &source.id).is_ok_and(|shard| shard.is_empty()) {
            continue;
        }
        // write_index loads each shard and replaces this count when it can
        // read the shard; an unreadable or missing shard keeps 0.
        seller_meta.push(crate::obj! {
            "id" => source.id.as_str(),
            "name" => source.name.clone(),
            "url" => source.url.clone(),
            "album_count" => 0usize,
            "shard" => format!("catalog/{}.json", source.id),
            "run" => Value::Object(read_run(data_dir, &source.id)?),
        });
    }
    let shop_meta: Vec<Value> = shops
        .iter()
        .map(|s| {
            Value::Object(crate::obj! {
                "id" => s.id.as_str(),
                "platform" => s.platform.clone(),
                "name" => s.name.clone(),
                "url" => s.url.clone(),
            })
        })
        .collect();
    write_index(
        data_dir,
        &seller_meta,
        &Value::Array(shop_meta),
        run_ts,
        false,
    )
}
