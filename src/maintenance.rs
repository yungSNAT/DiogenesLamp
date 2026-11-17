// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Read-only diagnosis and explicit local metadata recovery.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::covers::{cover_facts, resize_cover};
use crate::errors::{Error, Result, value};
use crate::locking::CatalogueLock;
use crate::pyerr::{self, Call};
use crate::pyjson::{self, Map, Value};
use crate::schema::{canonical_album, data_path, validate_metadata};
use crate::storage::atomic_write_json;

pub const MAX_METADATA_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_METADATA_FILES: usize = 10000;

fn stat_len(path: &Path) -> Result<u64> {
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| Error::Os(pyerr::os_error(&e, Call::Os, path)))
}

pub fn read_metadata(path: &Path, relative: &str) -> Result<Value> {
    if stat_len(path)? > MAX_METADATA_BYTES {
        return value("metadata file exceeds 256 MB");
    }
    let text = pyerr::read_text(path).map_err(|f| Error::Os(f.message))?;
    let parsed = pyjson::loads(&text).map_err(|e| Error::Value(e.0))?;
    validate_metadata(&parsed, relative)?;
    Ok(parsed)
}

/// `sorted(dir.glob("*.json"))`: case-insensitive matching and ordering on
/// Windows, as pathlib does there.
fn json_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            if cfg!(windows) {
                name.to_lowercase().ends_with(".json")
            } else {
                name.ends_with(".json")
            }
        })
        .collect();
    if cfg!(windows) {
        names.sort_by_key(|n| n.to_lowercase());
    } else {
        names.sort();
    }
    names
        .into_iter()
        .map(|n| (n.clone(), dir.join(n)))
        .collect()
}

/// Metadata files as (relative posix path, path).
pub fn metadata_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    let index = root.join("index.json");
    if index.exists() {
        files.push(("index.json".to_string(), index));
    }
    for dir in ["catalog", "runs"] {
        for (name, path) in json_files(&root.join(dir)) {
            files.push((format!("{dir}/{name}"), path));
        }
    }
    if files.len() > MAX_METADATA_FILES {
        return value("too many metadata files");
    }
    for (relative, _) in &files {
        data_path(root, relative, false)?;
    }
    let mut total = 0u64;
    for (_, path) in &files {
        total += stat_len(path)?;
    }
    if total > MAX_METADATA_BYTES {
        return value("metadata exceeds 256 MB");
    }
    Ok(files)
}

fn sellers_of(index: &Value) -> Vec<Map> {
    index
        .as_object()
        .and_then(|m| m.get("sellers"))
        .and_then(Value::as_array)
        .map(|s| s.iter().filter_map(|v| v.as_object().cloned()).collect())
        .unwrap_or_default()
}

fn albums_of(shard: &Value) -> Vec<Map> {
    shard
        .as_object()
        .and_then(|m| m.get("albums"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_object().cloned()).collect())
        .unwrap_or_default()
}

fn id_of(album: &Map) -> String {
    album.get("id").map(Value::py_str).unwrap_or_default()
}

fn truthy(map: &Map, key: &str) -> bool {
    map.get(key).is_some_and(Value::truthy)
}

/// The albums the index selects, seller by seller in index and shard order:
/// those in a seller's `album_ids`, or all of its shard's albums in an
/// older index without them. A missing or invalid index or shard, or an
/// indexed album missing from its shard, is an error, so a reader never
/// works from part of the catalogue.
pub fn selected_albums(root: &Path) -> Result<Vec<Map>> {
    let index = read_metadata(&data_path(root, "index.json", false)?, "index.json")?;
    let mut selected = Vec::new();
    for seller in sellers_of(&index) {
        let relative = seller.get("shard").map(Value::py_str).unwrap_or_default();
        let albums = albums_of(&read_metadata(
            &data_path(root, &relative, false)?,
            &relative,
        )?);
        let Some(Value::Array(ids)) = seller.get("album_ids") else {
            selected.extend(albums);
            continue;
        };
        let ids: std::collections::HashSet<String> = ids.iter().map(Value::py_str).collect();
        let before = selected.len();
        selected.extend(albums.into_iter().filter(|a| ids.contains(&id_of(a))));
        if selected.len() - before != ids.len() {
            return value(format!("{relative}: indexed album missing from shard"));
        }
    }
    Ok(selected)
}

pub fn diagnose(root: &Path) -> Map {
    let mut counts: Vec<(&str, i128)> = vec![
        ("albums", 0),
        ("unadmitted", 0),
        ("missing_covers", 0),
        ("unpriced", 0),
        ("without_detail_date", 0),
        ("without_description", 0),
        ("links_truncated", 0),
        ("gone", 0),
        ("detail_failures", 0),
    ];
    let mut issues: Vec<String> = Vec::new();
    let add = |counts: &mut Vec<(&str, i128)>, key: &str, n: i128| {
        if let Some(entry) = counts.iter_mut().find(|(k, _)| *k == key) {
            entry.1 += n;
        }
    };
    let index = data_path(root, "index.json", false).and_then(|p| read_metadata(&p, "index.json"));
    match index {
        Err(e) => issues.push(format!("index.json: {e}")),
        Ok(index) => {
            let selected_index = matches!(
                index.as_object().and_then(|m| m.get("selection_version")),
                Some(Value::Int(1))
            );
            let mut ids: std::collections::HashSet<String> = Default::default();
            for seller in sellers_of(&index) {
                let relative = seller.get("shard").map(Value::py_str).unwrap_or_default();
                let outcome = (|| -> Result<()> {
                    let shard = read_metadata(&data_path(root, &relative, false)?, &relative)?;
                    let albums = albums_of(&shard);
                    let selected: std::collections::HashSet<String> = match seller.get("album_ids")
                    {
                        Some(Value::Array(items)) => items.iter().map(Value::py_str).collect(),
                        _ => albums.iter().map(id_of).collect(),
                    };
                    let present: std::collections::HashSet<String> =
                        albums.iter().map(id_of).collect();
                    if !selected.is_subset(&present) {
                        issues.push(format!("{relative}: indexed album missing from shard"));
                    }
                    let count = albums
                        .iter()
                        .filter(|a| selected.contains(&id_of(a)) && !truthy(a, "gone_since"))
                        .count();
                    if let Some(listed) = seller.get("album_count")
                        && !pyjson::py_eq(listed, &Value::from(count))
                    {
                        issues.push(format!("{relative}: index count differs from shard"));
                    }
                    for album in &albums {
                        add(&mut counts, "albums", 1);
                        let admitted = canonical_album(album);
                        add(&mut counts, "unadmitted", i128::from(!admitted));
                        let id = id_of(album);
                        if selected_index && selected.contains(&id) && !admitted {
                            issues
                                .push(format!("{relative}: selected album is inadmissible: {id}"));
                        }
                        if ids.contains(&id) {
                            issues.push(format!(
                                "{relative}: duplicate album ID across shards: {id}"
                            ));
                        }
                        ids.insert(id);
                        add(&mut counts, "unpriced", i128::from(!truthy(album, "price")));
                        add(&mut counts, "gone", i128::from(truthy(album, "gone_since")));
                        add(
                            &mut counts,
                            "without_detail_date",
                            i128::from(!truthy(album, "updated_at")),
                        );
                        add(
                            &mut counts,
                            "without_description",
                            i128::from(!album.contains_key("description")),
                        );
                        add(
                            &mut counts,
                            "links_truncated",
                            i128::from(truthy(album, "links_truncated")),
                        );
                        if let Some(Value::Str(thumb)) = album.get("thumb")
                            && !thumb.is_empty()
                            && !data_path(root, thumb, true)?.is_file()
                        {
                            add(&mut counts, "missing_covers", 1);
                        }
                    }
                    let run_relative = format!(
                        "runs/{}.json",
                        seller.get("id").map(Value::py_str).unwrap_or_default()
                    );
                    let run_path = data_path(root, &run_relative, false)?;
                    if run_path.exists() {
                        let run = read_metadata(&run_path, &run_relative)?;
                        let failed = run
                            .as_object()
                            .and_then(|m| m.get("failed_ids"))
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len);
                        add(&mut counts, "detail_failures", failed as i128);
                    }
                    Ok(())
                })();
                if let Err(e) = outcome {
                    issues.push(format!("{relative}: {e}"));
                }
            }
        }
    }
    let mut report = crate::obj! { "schema_version" => 1i64 };
    for (key, n) in counts {
        report.insert(key.into(), Value::Int(n));
    }
    report.insert(
        "issues".into(),
        Value::Array(issues.into_iter().map(Value::Str).collect()),
    );
    report
}

fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n.div_ceil(2)];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()[..n].to_string()
}

fn zip_time(path: &Path) -> zip::DateTime {
    let local = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| jiff::ZonedDateTime::try_from(modified).ok())
        .unwrap_or_else(jiff::ZonedDateTime::now);
    zip::DateTime::from_date_and_time(
        local.year().clamp(1980, 2107) as u16,
        local.month() as u8,
        local.day() as u8,
        local.hour() as u8,
        local.minute() as u8,
        local.second() as u8,
    )
    .unwrap_or_default()
}

fn snapshot_inner(root: &Path, validate: bool) -> Result<PathBuf> {
    let files = metadata_files(root)?;
    if validate {
        let index = read_metadata(&data_path(root, "index.json", false)?, "index.json")?;
        for seller in sellers_of(&index) {
            let shard = seller.get("shard").map(Value::py_str).unwrap_or_default();
            if !data_path(root, &shard, false)?.is_file() {
                return value(format!("{shard}: missing indexed shard"));
            }
        }
        for (relative, path) in &files {
            read_metadata(path, relative)?;
        }
    }
    let directory = root.join("snapshots");
    if let Err(e) = std::fs::create_dir(&directory)
        && !(e.kind() == std::io::ErrorKind::AlreadyExists && directory.is_dir())
    {
        return Err(Error::Os(pyerr::os_error(&e, Call::Os, &directory)));
    }
    if !pyerr::resolve(&directory).starts_with(pyerr::resolve(root)) {
        return value("snapshot directory escapes data directory");
    }
    let name = format!(
        "{}-{}.zip",
        jiff::Timestamp::now().strftime("%Y%m%dT%H%M%S"),
        random_hex(8)
    );
    let path = directory.join(name);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| Error::Os(pyerr::os_error(&e, Call::Open, &path)))?;
    let written = (|| -> Result<()> {
        let mut archive = zip::ZipWriter::new(file);
        for (relative, source) in &files {
            let data = pyerr::read_bytes(source).map_err(|f| Error::Os(f.message))?;
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .last_modified_time(zip_time(source));
            archive
                .start_file(relative.as_str(), options)
                .map_err(|e| Error::Os(e.to_string()))?;
            archive
                .write_all(&data)
                .map_err(|e| Error::Os(e.to_string()))?;
        }
        archive.finish().map_err(|e| Error::Os(e.to_string()))?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

/// Save a bounded metadata snapshot (covers excluded).
pub fn snapshot(root: &Path) -> Result<PathBuf> {
    let _lock = CatalogueLock::acquire(root)?;
    snapshot_inner(root, true)
}

/// Restore metadata from a snapshot; returns where the previous files went.
pub fn restore(root: &Path, archive_path: &Path) -> Result<Option<PathBuf>> {
    let _lock = CatalogueLock::acquire(root)?;
    let file = std::fs::File::open(archive_path)
        .map_err(|e| Error::Os(pyerr::os_error(&e, Call::Open, archive_path)))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| Error::BadZip("File is not a zip file".into()))?;
    let mut total = 0u64;
    let mut names = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|_| Error::BadZip("File is not a zip file".into()))?;
        total += entry.size();
        names.push(entry.name().map(|n| n.into_owned()).unwrap_or_default());
    }
    if names.len() > MAX_METADATA_FILES || total > MAX_METADATA_BYTES {
        return value("snapshot exceeds recovery limits");
    }
    let mut records: Vec<(String, Value)> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        data_path(root, name, false)?;
        if archive.has_duplicate_names() || records.iter().any(|(n, _)| n == name) {
            return value("duplicate snapshot member");
        }
        let mut entry = archive
            .by_index(i)
            .map_err(|e| Error::BadZip(e.to_string()))?;
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|_| {
            Error::BadZip(format!("Bad CRC-32 for file {}", crate::pystr::repr(name)))
        })?;
        let parsed = pyjson::loads_bytes(&data).map_err(Error::Value)?;
        validate_metadata(&parsed, name)?;
        records.push((name.clone(), parsed));
    }
    let Some((_, index)) = records.iter().find(|(n, _)| n == "index.json") else {
        return value("snapshot has no index");
    };
    for seller in sellers_of(index) {
        let shard = seller.get("shard").map(Value::py_str).unwrap_or_default();
        if !records.iter().any(|(n, _)| *n == shard) {
            return value("snapshot is missing an indexed shard");
        }
    }
    let previous = if metadata_files(root)?.is_empty() {
        None
    } else {
        Some(snapshot_inner(root, false)?)
    };
    for (relative, payload) in &records {
        if relative != "index.json" {
            atomic_write_json(&data_path(root, relative, false)?, payload)?;
        }
    }
    let index = records
        .iter()
        .find(|(n, _)| n == "index.json")
        .map(|(_, v)| v.clone())
        .unwrap_or(Value::Null);
    atomic_write_json(&data_path(root, "index.json", false)?, &index)?;
    Ok(previous)
}

/// Resize and fingerprint cached covers, at most `limit` per run.
pub fn prepare_covers(root: &Path, limit: i128) -> Result<Map> {
    let mut processed: i128 = 0;
    let mut resized: i128 = 0;
    let mut before_bytes: i128 = 0;
    let mut after_bytes: i128 = 0;
    let mut missing_covers: i128 = 0;
    let mut errors: Vec<String> = Vec::new();
    let _lock = CatalogueLock::acquire(root)?;
    // Taken before the first cover is touched, so a batch with nothing to
    // prepare writes no snapshot.
    let mut snapshot_path: Option<PathBuf> = None;
    let index = read_metadata(&data_path(root, "index.json", false)?, "index.json")?;
    for seller in sellers_of(&index) {
        if processed >= limit {
            break;
        }
        let relative = seller.get("shard").map(Value::py_str).unwrap_or_default();
        let mut shard = read_metadata(&data_path(root, &relative, false)?, &relative)?;
        let mut changed = false;
        if let Some(Value::Array(albums)) = shard.as_object_mut().and_then(|m| m.get_mut("albums"))
        {
            for album in albums.iter_mut() {
                if processed >= limit {
                    break;
                }
                let Some(album) = album.as_object_mut() else {
                    continue;
                };
                let thumb = match album.get("thumb") {
                    Some(Value::Str(t)) if !t.is_empty() => t.clone(),
                    _ => continue,
                };
                let path = data_path(root, &thumb, true)?;
                // A cover missing from disk is the next scrape's to fetch, so
                // it neither takes a snapshot nor counts against the limit.
                if !path.is_file() {
                    missing_covers += 1;
                    continue;
                }
                if album
                    .get("cover_prepared")
                    .is_some_and(|v| pyjson::py_eq(v, &Value::Int(1)))
                {
                    continue;
                }
                if snapshot_path.is_none() {
                    snapshot_path = Some(snapshot_inner(root, true)?);
                }
                processed += 1;
                let attempt = (|| -> Result<()> {
                    let size = stat_len(&path)?;
                    if size > 8 * 1024 * 1024 {
                        return value("cover exceeds 8 MB");
                    }
                    before_bytes += i128::from(size);
                    resized += i128::from(resize_cover(&path)?);
                    after_bytes += i128::from(stat_len(&path)?);
                    for (k, v) in cover_facts(&path) {
                        album.insert(k, v);
                    }
                    album.insert("cover_prepared".into(), Value::Int(1));
                    Ok(())
                })();
                match attempt {
                    Ok(()) => changed = true,
                    Err(e) => errors.push(format!("{}: {e}", id_of(album))),
                }
            }
        }
        if changed {
            if let Some(map) = shard.as_object_mut() {
                map.insert("schema_version".into(), Value::Int(1));
            }
            atomic_write_json(&data_path(root, &relative, false)?, &shard)?;
        }
    }
    let mut result = crate::obj! {
        "processed" => Value::Int(processed),
        "resized" => Value::Int(resized),
        "before_bytes" => Value::Int(before_bytes),
        "after_bytes" => Value::Int(after_bytes),
        "missing_covers" => Value::Int(missing_covers),
    };
    result.insert(
        "errors".into(),
        Value::Array(errors.into_iter().map(Value::Str).collect()),
    );
    result.insert(
        "snapshot".into(),
        snapshot_path.map_or(Value::Null, |p| pyerr::path_str(&p).into()),
    );
    Ok(result)
}

/// Most bits in which a cover's fingerprint may differ from the image's and
/// still be suggested; the viewer's Similar covers uses the same threshold.
const FIND_PHOTO_DISTANCE: u32 = 8;

/// A fingerprint other than 16 lower-case hex digits, as the viewer reads
/// them, or a uniform one (all 0 or all 1 bits, a blank picture), says
/// nothing about layout.
fn layout_hash(hash: &str) -> Option<u64> {
    let digits = hash.len() == 16 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    let bits = u64::from_str_radix(hash, 16).ok().filter(|_| digits)?;
    (bits != 0 && bits != u64::MAX).then_some(bits)
}

/// `lamp find-photo`: the selected albums whose cover fingerprint is within
/// `FIND_PHOTO_DISTANCE` bits of the image's, nearest first, then by ID, at
/// most `limit` of them. Suggestions only: nothing is merged or written.
pub fn find_photo(root: &Path, image: &Path, limit: usize) -> Result<Map> {
    let unusable = |reason: String| {
        Error::Value(format!(
            "cannot fingerprint {}: {reason}",
            pyerr::path_str(image)
        ))
    };
    let hash = crate::covers::image_hash(image).map_err(unusable)?;
    let Some(bits) = layout_hash(&hash) else {
        return Err(unusable("it is a blank or single-colour picture".into()));
    };
    let (mut searched, mut without_hash) = (0usize, 0usize);
    let mut matches: Vec<(u32, String, Map)> = Vec::new();
    for album in selected_albums(root)? {
        let Some(cover) = album
            .get("cover_hash")
            .and_then(|h| layout_hash(&h.py_str()))
        else {
            without_hash += 1;
            continue;
        };
        searched += 1;
        let distance = (bits ^ cover).count_ones();
        if distance <= FIND_PHOTO_DISTANCE {
            matches.push((distance, id_of(&album), album));
        }
    }
    matches.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    matches.truncate(limit);
    let field = |album: &Map, key: &str| album.get(key).cloned().unwrap_or(Value::Null);
    let matches = matches
        .into_iter()
        .map(|(distance, id, album)| {
            Value::Object(crate::obj! {
                "id" => id,
                "seller" => field(&album, "seller"),
                "distance" => Value::Int(i128::from(distance)),
                "title" => field(&album, "title"),
                "album_url" => field(&album, "album_url"),
                "gone_since" => field(&album, "gone_since"),
            })
        })
        .collect();
    Ok(crate::obj! {
        "image_hash" => hash,
        "searched" => searched,
        "without_hash" => without_hash,
        "matches" => Value::Array(matches),
    })
}

/// `lamp export` columns, in the viewer's CSV order where the CLI has the
/// data; the viewer's kind and agent links are computed in the browser.
pub const EXPORT_COLUMNS: [&str; 11] = [
    "seller",
    "album_id",
    "title",
    "currency",
    "price",
    "details_checked",
    "album_url",
    "purchase_links",
    "description",
    "first_seen",
    "gone_since",
];

/// `lamp export`: one row per selected album, in index and shard order, with
/// the `EXPORT_COLUMNS` fields. `since` (YYYY-MM-DD) keeps the albums whose
/// `first_seen` date, the local date of the run that first stored them, is
/// that day or later; a `first_seen` with no readable date leaves the album
/// out. An unknown price, date or text is null.
pub fn export_rows(root: &Path, since: Option<&str>) -> Result<Vec<Map>> {
    let since = match since.map(|day| (day, crate::isodate::parse_date(day))) {
        None => None,
        Some((_, Some(date))) => Some(date.ordinal),
        Some((day, None)) => return value(format!("not a date: {day}")),
    };
    let mut rows = Vec::new();
    for album in selected_albums(root)? {
        if let Some(since) = since {
            let first_day = match album.get("first_seen") {
                Some(Value::Str(text)) => crate::isodate::parse_datetime(text)
                    .map(|(local, _)| local.div_euclid(86_400_000_000)),
                _ => None,
            };
            if first_day.is_none_or(|day| day < since) {
                continue;
            }
        }
        rows.push(export_row(&album));
    }
    Ok(rows)
}

/// One `lamp export` row for an album.
fn export_row(album: &Map) -> Map {
    let field = |key: &str| album.get(key).cloned().unwrap_or(Value::Null);
    let price = match album.get("price") {
        Some(p) if p.truthy() => p.clone(),
        _ => Value::Null,
    };
    let links = match album.get("links") {
        Some(Value::Array(links)) => links
            .iter()
            .filter_map(|l| l.as_object().and_then(|l| l.get("url")).cloned())
            .collect(),
        _ => Vec::new(),
    };
    let values = [
        field("seller"),
        id_of(album).into(),
        field("title"),
        "CNY".into(),
        price,
        field("updated_at"),
        field("album_url"),
        Value::Array(links),
        field("description"),
        field("first_seen"),
        field("gone_since"),
    ];
    EXPORT_COLUMNS
        .iter()
        .map(|c| c.to_string())
        .zip(values)
        .collect()
}

/// `lamp scrape --notify`: the albums the runs at `runs` (their timestamps)
/// stored for the first time (`new_ids`, the (seller, album id) pairs they
/// reported as new) or changed, as `export_rows` rows plus `new` and
/// `changes`, the field, before and after of each history entry those runs
/// recorded. Every album in the shards
/// the index read counts, duplicates of another listing included: each is a
/// seller's own copy, with its own price. A seller whose shard the index
/// could not read has no `album_ids` and is left out.
pub fn run_rows(
    root: &Path,
    runs: &std::collections::HashSet<String>,
    new_ids: &std::collections::HashSet<(String, String)>,
) -> Result<Vec<Map>> {
    let index = read_metadata(&data_path(root, "index.json", false)?, "index.json")?;
    let mut rows = Vec::new();
    for seller in sellers_of(&index) {
        if !matches!(seller.get("album_ids"), Some(Value::Array(_))) {
            continue;
        }
        let id = seller.get("id").map(Value::py_str).unwrap_or_default();
        let relative = seller.get("shard").map(Value::py_str).unwrap_or_default();
        let shard = read_metadata(&data_path(root, &relative, false)?, &relative)?;
        for album in albums_of(&shard) {
            let new = new_ids.contains(&(id.clone(), id_of(&album)));
            let changes: Vec<Value> = album
                .get("history")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .filter(|entry| {
                    entry
                        .get("at")
                        .and_then(Value::as_str)
                        .is_some_and(|at| runs.contains(at))
                })
                .map(|entry| {
                    let field = |key: &str| entry.get(key).cloned().unwrap_or(Value::Null);
                    Value::Object(crate::obj! {
                        "field" => field("field"),
                        "before" => field("before"),
                        "after" => field("after"),
                    })
                })
                .collect();
            if !new && changes.is_empty() {
                continue;
            }
            let mut row = export_row(&album);
            row.insert("new".into(), new.into());
            row.insert("changes".into(), Value::Array(changes));
            rows.push(row);
        }
    }
    Ok(rows)
}

/// A CSV cell as the viewer writes one: quoted, with a leading `'` before a
/// value a spreadsheet would read as a formula.
fn csv_cell(value: &Value) -> String {
    let text = match value {
        Value::Null => String::new(),
        Value::Array(items) => items
            .iter()
            .map(Value::py_str)
            .collect::<Vec<_>>()
            .join(" "),
        other => other.py_str(),
    };
    // JavaScript's \s: Unicode White_Space without U+0085, plus U+FEFF.
    let formula = text
        .trim_start_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}')
        .starts_with(['=', '+', '@', '-'])
        || text.starts_with(['\t', '\r', '\n']);
    let text = if formula { format!("'{text}") } else { text };
    format!("\"{}\"", text.replace('"', "\"\""))
}

/// Rows as the viewer's CSV export writes them: a byte order mark, a header
/// row, then one line per row, each ending in CRLF.
pub fn export_csv(rows: &[Map]) -> String {
    let header: Vec<Value> = EXPORT_COLUMNS.iter().map(|c| Value::from(*c)).collect();
    let mut csv = String::from("\u{feff}");
    let row = |r: &Map| {
        EXPORT_COLUMNS
            .iter()
            .map(|c| r.get(*c).cloned().unwrap_or(Value::Null))
            .collect()
    };
    for cells in std::iter::once(header).chain(rows.iter().map(row)) {
        let line: Vec<String> = cells.iter().map(csv_cell).collect();
        csv.push_str(&line.join(","));
        csv.push_str("\r\n");
    }
    csv
}

/// What `lamp export --site` wrote.
pub struct Site {
    pub sellers: usize,
    pub covers: usize,
    /// Covers the shards name that are not cached; the viewer shows none.
    pub missing_covers: usize,
}

/// `lamp export --site`: the viewer and the catalogue it reads as static
/// files for a web host, such as one a phone can reach. `dir` gets the
/// bundled viewer, its index.html carrying the policy `lamp serve` sends as
/// a header, and under data/ the index, every indexed shard, the cached
/// covers the shards name and the operator's aliases.json when there is one;
/// snapshots, run summaries and sources stay behind. The catalogue is locked
/// and checked before anything is written. `dir` must be new or empty and
/// outside the data directory.
pub fn export_site(root: &Path, dir: &Path) -> Result<Site> {
    let _lock = CatalogueLock::acquire(root)?;
    let os = |e: std::io::Error, path: &Path| Error::Os(pyerr::os_error(&e, Call::Os, path));
    // Checked and written by one resolved path, so `missing/../site` neither
    // fails nor makes `missing`. A relative path without a readable working
    // folder is an error, not a folder at the filesystem root.
    let dir = &pyerr::resolve(&std::path::absolute(dir).map_err(|e| os(e, dir))?);
    if pyerr::is_relative_to(dir, &pyerr::resolve(root)) {
        return value("the site folder must be outside the data directory");
    }
    match std::fs::read_dir(dir) {
        Ok(mut entries) => match entries.next() {
            None => {}
            Some(Ok(_)) => {
                return value(format!("{}: folder is not empty", pyerr::path_str(dir)));
            }
            Some(Err(e)) => return Err(os(e, dir)),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(os(e, dir)),
    }
    let index = data_path(root, "index.json", false)?;
    let sellers = sellers_of(&read_metadata(&index, "index.json")?);
    let mut copies = vec![(index, "index.json".to_string())];
    let mut covers = std::collections::BTreeSet::new();
    let mut missing = std::collections::BTreeSet::new();
    for seller in &sellers {
        let relative = seller.get("shard").map(Value::py_str).unwrap_or_default();
        let shard = data_path(root, &relative, false)?;
        for album in albums_of(&read_metadata(&shard, &relative)?) {
            if let Some(Value::Str(thumb)) = album.get("thumb")
                && !thumb.is_empty()
            {
                if data_path(root, thumb, true)?.is_file() {
                    covers.insert(thumb.clone());
                } else {
                    missing.insert(thumb.clone());
                }
            }
        }
        copies.push((shard, relative));
    }
    // Not followed through a link: the site may carry only the data
    // directory's own file.
    let aliases = root.join("aliases.json");
    match std::fs::symlink_metadata(&aliases) {
        Ok(meta) if meta.is_file() => {
            if meta.len() > MAX_METADATA_BYTES {
                return value("aliases.json exceeds 256 MB");
            }
            copies.push((aliases, "aliases.json".into()));
        }
        Ok(_) => return value("aliases.json is not a regular file"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(os(e, &aliases)),
    }
    for cover in &covers {
        copies.push((data_path(root, cover, true)?, cover.clone()));
    }
    // A failure partway removes exactly what this export made, so a file
    // another program puts in the folder meanwhile stays.
    let mut made = Made::default();
    write_site(dir, &copies, &mut made).inspect_err(|_| made.undo())?;
    Ok(Site {
        sellers: sellers.len(),
        covers: covers.len(),
        missing_covers: missing.len(),
    })
}

/// The folders and files `lamp export --site` created, in order.
#[derive(Default)]
struct Made {
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

impl Made {
    /// `path` and any missing ancestors, each recorded as it is made.
    fn dir(&mut self, path: &Path) -> Result<()> {
        let mut missing = Vec::new();
        let mut next = Some(path);
        while let Some(p) = next.filter(|p| !p.as_os_str().is_empty() && !p.exists()) {
            missing.push(p.to_path_buf());
            next = p.parent();
        }
        for p in missing.into_iter().rev() {
            std::fs::create_dir(&p).map_err(|e| Error::Os(pyerr::os_error(&e, Call::Os, &p)))?;
            self.dirs.push(p);
        }
        Ok(())
    }

    /// A new file at `target` holding `bytes`; one already there is an error.
    fn file(&mut self, target: &Path, bytes: &mut dyn Read) -> Result<()> {
        if let Some(parent) = target.parent() {
            self.dir(parent)?;
        }
        let error = |e: std::io::Error| Error::Os(pyerr::os_error(&e, Call::Open, target));
        let mut out = std::fs::File::create_new(target).map_err(error)?;
        self.files.push(target.to_path_buf());
        std::io::copy(bytes, &mut out).map_err(error)?;
        Ok(())
    }

    /// Removes the files, then each folder left empty, newest first.
    fn undo(&self) {
        for file in self.files.iter().rev() {
            let _ = std::fs::remove_file(file);
        }
        for dir in self.dirs.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// The viewer, its index.html carrying the policy, and `copies` under data/.
fn write_site(dir: &Path, copies: &[(PathBuf, String)], made: &mut Made) -> Result<()> {
    made.dir(dir)?;
    const CHARSET: &str = "<meta charset=\"utf-8\">";
    for (name, bytes) in crate::viewer::FILES {
        let policy;
        let bytes = if *name == "index.html" {
            let html = std::str::from_utf8(bytes)
                .ok()
                .filter(|html| html.contains(CHARSET))
                .ok_or_else(|| Error::Value("viewer index.html has no charset tag".into()))?;
            policy = html.replacen(
                CHARSET,
                &format!(
                    "{CHARSET}\n<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">",
                    crate::serve::CSP
                ),
                1,
            );
            policy.as_bytes()
        } else {
            bytes
        };
        made.file(&dir.join(name), &mut &bytes[..])?;
    }
    for (source, relative) in copies {
        let mut file = std::fs::File::open(source)
            .map_err(|e| Error::Os(pyerr::os_error(&e, Call::Open, source)))?;
        made.file(&dir.join("data").join(relative), &mut file)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zip member carries its file's modified time as local wall time.
    #[test]
    fn zip_time_is_the_local_modified_time() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.json");
        let file = std::fs::File::create(&path).unwrap();
        // 2027-01-15T08:00:00Z.
        let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000);
        file.set_modified(modified).unwrap();
        let time = zip_time(&path);
        assert_eq!((time.year(), time.month()), (2027, 1));
        assert!((14..=16).contains(&time.day()), "{time:?}");
        #[cfg(unix)]
        if crate::cli::tests::rerun_in_utc_plus_0530(
            "maintenance::tests::zip_time_is_the_local_modified_time",
        ) {
            let fields = (time.day(), time.hour(), time.minute(), time.second());
            assert_eq!(fields, (15, 13, 30, 0));
        }
    }

    /// A failed site export removes only what it made: a file another
    /// program put in the folder meanwhile stays, with the folders holding it.
    #[test]
    fn undo_removes_only_what_the_export_made() {
        let tmp = tempfile::tempdir().unwrap();
        let site = tmp.path().join("site");
        let mut made = Made::default();
        made.file(&site.join("data/catalog/one.json"), &mut &b"{}"[..])
            .unwrap();
        made.file(&site.join("index.html"), &mut &b"x"[..]).unwrap();
        std::fs::write(site.join("data/theirs.txt"), b"keep").unwrap();
        assert!(
            made.file(&site.join("data/theirs.txt"), &mut &b"y"[..])
                .is_err()
        );
        made.undo();
        assert_eq!(
            std::fs::read(site.join("data/theirs.txt")).unwrap(),
            b"keep"
        );
        assert!(!site.join("index.html").exists());
        assert!(!site.join("data/catalog").exists());
    }
}
