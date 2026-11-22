// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! The parser that reads album titles and descriptions gives the text and
//! attributes `BeautifulSoup(html, "html.parser")` gives. The expected
//! results in `fixtures/html_parity.json` were recorded from that parser by
//! `fixtures/html_parity.py`; the file's `generator` field names the versions.
mod common;

use common::get;
use lamp::html::parse;
use lamp::pyjson::Value;

/// Attributes the fixture may hold; any the reference parser did not give
/// as a string must be absent here too.
const KEYS: [&str; 12] = [
    "title",
    "b",
    "c",
    "d",
    "e",
    "f",
    "y",
    "=",
    "==x",
    "/",
    "class",
    "\"unterminated",
];

#[test]
fn text_and_attributes_match_the_reference_parser() {
    let fixture = &common::json(&common::fixture("html_parity.json"));
    let cases = get(fixture, "cases").as_array().unwrap();
    assert_eq!(cases.len(), 90);
    let mut failures = Vec::new();
    for case in cases {
        let html = get(case, "html").as_str().unwrap();
        let doc = parse(html);
        let mut check = |what: &str, got: Option<&str>, want: Option<&str>| {
            if got != want {
                failures.push(format!("{html:?} {what}: got {got:?}, want {want:?}"));
            }
        };
        let root = doc.root();
        check(
            "text",
            Some(doc.get_text(root, "", false).as_str()),
            get(case, "text").as_str(),
        );
        check(
            "stripped",
            Some(doc.get_text(root, "|", true).as_str()),
            get(case, "stripped").as_str(),
        );
        for tag in get(case, "tags").as_array().unwrap() {
            let name = get(tag, "name").as_str().unwrap();
            let Some(id) = doc.select_one(root, name) else {
                check(name, None, Some("an element"));
                continue;
            };
            check(
                &format!("{name} text"),
                Some(doc.get_text(id, "", false).as_str()),
                get(tag, "text").as_str(),
            );
            let Value::Object(attrs) = get(tag, "attrs") else {
                panic!("attrs");
            };
            for key in attrs.keys().filter(|k| !KEYS.contains(&k.as_str())) {
                check(&format!("{name}[{key}]"), Some("not in KEYS"), None);
            }
            for key in KEYS {
                check(
                    &format!("{name}[{key}]"),
                    doc.attr(id, key),
                    attrs.get(key).and_then(Value::as_str),
                );
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
