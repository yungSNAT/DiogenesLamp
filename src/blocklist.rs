// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Seller blocklist handling for sources.toml, import and scrape.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use crate::errors::{Error, Result, value};
use crate::ingest::{
    ImportReport, Source, Sources, classify_url, format_sources_lines, parse_sources_table,
    toml_value, write_sources_text,
};
use crate::pyerr;
use crate::pyjson::{self, Value};

/// Sellers refused during `lamp import` and omitted from `lamp scrape`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blocklist {
    entries: BTreeSet<String>,
}

impl Blocklist {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &BTreeSet<String> {
        &self.entries
    }

    pub fn insert_raw(&mut self, raw: &str) -> Result<()> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return value("blocklist skip_sellers entries must be non-empty strings");
        }
        let normalized = if trimmed.contains("://") || trimmed.contains('.') {
            match classify_url(trimmed) {
                Some(src) => format!("{}:{}", src.kind, src.id),
                None => trimmed.to_ascii_lowercase(),
            }
        } else {
            trimmed.to_ascii_lowercase()
        };
        self.entries.insert(normalized);
        Ok(())
    }

    pub fn matches(&self, src: &Source) -> bool {
        let id = src.id.to_ascii_lowercase();
        let key = format!("{}:{id}", src.kind);
        self.entries.contains(&id) || self.entries.contains(&key)
    }

    pub fn matches_yupoo_id(&self, seller_id: &str) -> bool {
        let id = seller_id.to_ascii_lowercase();
        let key = format!("yupoo:{id}");
        self.entries.contains(&id) || self.entries.contains(&key)
    }

    pub fn append_toml_lines(&self, lines: &mut Vec<String>) {
        if self.entries.is_empty() {
            return;
        }
        lines.push("[blocklist]".into());
        let values: Vec<String> = self
            .entries
            .iter()
            .map(|entry| toml_value(&Value::Str(entry.clone())))
            .collect();
        lines.push(format!("skip_sellers = [{}]", values.join(", ")));
        lines.push(String::new());
    }
}

/// Sources and blocklist loaded together from `sources.toml`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourcesConfig {
    pub sources: Sources,
    pub blocklist: Blocklist,
}

fn parse_skip_sellers_array(list: &toml::Value, blocklist: &mut Blocklist) -> Result<()> {
    let Some(items) = list.as_array() else {
        return value("[blocklist].skip_sellers must be an array of strings");
    };
    for item in items {
        let Some(text) = item.as_str() else {
            return value("[blocklist].skip_sellers entries must be strings");
        };
        blocklist.insert_raw(text)?;
    }
    Ok(())
}

pub(crate) fn parse_blocklist_table(data: &toml::Table) -> Result<Blocklist> {
    let mut blocklist = Blocklist::new();
    if let Some(top) = data.get("skip_sellers") {
        parse_skip_sellers_array(top, &mut blocklist)?;
    }
    if let Some(section) = data.get("blocklist") {
        let Some(table) = section.as_table() else {
            return value("[blocklist] must be a table");
        };
        if let Some(list) = table.get("skip_sellers") {
            parse_skip_sellers_array(list, &mut blocklist)?;
        }
    }
    Ok(blocklist)
}

pub fn load_sources_config(path: &Path) -> Result<SourcesConfig> {
    if !path.exists() {
        return Ok(SourcesConfig::default());
    }
    let text = pyerr::read_text(path).map_err(|f| Error::Value(f.message))?;
    let data: toml::Table = text.parse().map_err(|e: toml::de::Error| {
        Error::Value(format!(
            "invalid sources file {}: {}",
            pyerr::path_str(path),
            e.message()
        ))
    })?;
    let blocklist = parse_blocklist_table(&data)?;
    let sources = parse_sources_table(path, &data)?;
    Ok(SourcesConfig { sources, blocklist })
}

pub fn write_sources_config(path: &Path, config: &SourcesConfig) -> Result<()> {
    let mut lines = format_sources_lines(&config.sources)?;
    config.blocklist.append_toml_lines(&mut lines);
    write_sources_text(path, &lines)
}

pub fn merge_sources_with_blocklist(
    existing: &Sources,
    raws: &[String],
    blocklist: &Blocklist,
) -> (Sources, ImportReport) {
    let mut merged = existing.clone();
    let mut report = ImportReport {
        rows: raws.len(),
        ..Default::default()
    };
    let mut seen: HashSet<String> = HashSet::new();
    for raw in raws {
        let Some(src) = classify_url(raw) else {
            report.invalid += 1;
            continue;
        };
        let key = format!("{}:{}", src.kind, src.id);
        if blocklist.matches(&src) {
            report.blocked.push((key, src.url.py_str()));
            continue;
        }
        if merged.contains_key(&key) || seen.contains(&key) {
            if let Some(held) = merged.get(&key)
                && !pyjson::py_eq(&held.url, &src.url)
            {
                report
                    .collisions
                    .push((key.clone(), held.url.py_str(), src.url.py_str()));
            }
            if !seen.contains(&key) {
                if src.kind == "yupoo" {
                    report.yupoo_known += 1;
                } else {
                    report.shops_known += 1;
                }
            }
            seen.insert(key);
            continue;
        }
        seen.insert(key.clone());
        if src.kind == "yupoo" {
            report.yupoo_new += 1;
        } else {
            report.shops_new += 1;
            let platform = src.platform.py_str();
            match report
                .shops_by_platform
                .iter_mut()
                .find(|(p, _)| *p == platform)
            {
                Some(entry) => entry.1 += 1,
                None => report.shops_by_platform.push((platform, 1)),
            }
        }
        merged.insert(key, src);
    }
    (merged, report)
}

/// Remove blocked entries before scraping, returning the filtered sources
/// and the sorted list of blocked Yupoo seller IDs.
pub fn filter_blocked_sources(sources: &Sources, blocklist: &Blocklist) -> (Sources, Vec<String>) {
    let mut kept = Sources::new();
    let mut blocked_yupoo = Vec::new();
    for (key, src) in sources {
        if blocklist.matches(src) {
            if src.kind == "yupoo" {
                blocked_yupoo.push(src.id.clone());
            }
            continue;
        }
        kept.insert(key.clone(), src.clone());
    }
    blocked_yupoo.sort();
    (kept, blocked_yupoo)
}
