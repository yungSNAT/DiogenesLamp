// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! The port of Python's `json` gives what CPython gives. Catalogue files
//! pass through it, so a difference here changes what is stored. The
//! expected results in `fixtures/py_formats.json` were produced by the
//! CPython its `generator` field names.
mod common;

use lamp::pyjson::{self, Style, Value};

fn fixture() -> Value {
    common::json(&common::fixture("py_formats.json"))
}

fn rows<'a>(fixture: &'a Value, key: &str) -> &'a [Value] {
    let rows = get(fixture, key).as_array().unwrap();
    assert!(!rows.is_empty(), "{key}");
    rows
}

static NULL: Value = Value::Null;

/// `value[key]`, or null where the object lacks it.
fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    value
        .as_object()
        .and_then(|map| map.get(key))
        .unwrap_or(&NULL)
}

fn text(value: &Value) -> Option<&str> {
    value.as_str()
}

/// Collects mismatches so one run reports them all.
#[derive(Default)]
struct Mismatches(Vec<String>);

impl Mismatches {
    fn check<T: PartialEq + std::fmt::Debug>(&mut self, what: &str, input: &str, got: T, want: T) {
        if got != want {
            self.0
                .push(format!("{what} {input:?}: got {got:?}, want {want:?}"));
        }
    }

    fn assert_none(self) {
        assert!(self.0.is_empty(), "{}", self.0.join("\n"));
    }
}

#[test]
fn json_matches_python() {
    let fixture = fixture();
    let mut m = Mismatches::default();
    for case in rows(&fixture, "json") {
        let input = text(&get(case, "in")).unwrap();
        let loaded = pyjson::loads(input);
        if case.as_object().unwrap().contains_key("lone_surrogate") {
            let error = loaded.map(|_| ()).unwrap_err().0;
            m.check(
                "lone surrogate",
                input,
                error.starts_with("lone surrogate \\ud83d is not supported"),
                true,
            );
            continue;
        }
        match (loaded, text(&get(case, "error"))) {
            (Err(error), want) => m.check("error", input, Some(error.0.as_str()), want),
            (Ok(value), Some(want)) => m.check("accepted", input, value.py_str(), want.into()),
            (Ok(value), None) => {
                m.check(
                    "default",
                    input,
                    pyjson::dumps(&value, Style::Default),
                    get(case, "default").py_str(),
                );
                m.check(
                    "compact",
                    input,
                    pyjson::dumps(&value, Style::Compact),
                    get(case, "compact").py_str(),
                );
                m.check(
                    "indent",
                    input,
                    pyjson::dumps(&value, Style::Indent(2)),
                    get(case, "indent").py_str(),
                );
                m.check(
                    "truthy",
                    input,
                    value.truthy(),
                    matches!(get(case, "truthy"), Value::Bool(true)),
                );
            }
        }
    }
    for pair in rows(&fixture, "json_pairs") {
        let pair = pair.as_array().unwrap();
        let (a, b) = (text(&pair[0]).unwrap(), text(&pair[1]).unwrap());
        let (x, y) = (pyjson::loads(a).unwrap(), pyjson::loads(b).unwrap());
        let want = matches!(pair[2], Value::Bool(true));
        m.check("==", &format!("{a} {b}"), pyjson::py_eq(&x, &y), want);
        m.check(
            "== reversed",
            &format!("{b} {a}"),
            pyjson::py_eq(&y, &x),
            want,
        );
    }
    m.assert_none();
}
