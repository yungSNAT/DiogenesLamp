// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! The ports of Python's `str` behaviour (`repr`, `splitlines`, `strip`,
//! `int()` and `float()`) give what CPython gives. Command-line numbers,
//! response headers and error messages pass through them. The expected
//! results in `fixtures/py_text.json` were produced by the CPython its
//! `generator` field names.
mod common;

use lamp::pyjson::Value;
use lamp::pystr;

fn fixture() -> Value {
    common::json(&common::fixture("py_text.json"))
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

fn option(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        other => Some(other.py_str()),
    }
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
fn str_behaviour_matches_python() {
    let fixture = fixture();
    let mut m = Mismatches::default();
    for case in rows(&fixture, "repr") {
        let case = case.as_array().unwrap();
        let input = text(&case[0]).unwrap();
        m.check("repr", input, pystr::repr(input), case[1].py_str());
    }
    for case in rows(&fixture, "splitlines") {
        let case = case.as_array().unwrap();
        let input = text(&case[0]).unwrap();
        let want: Vec<String> = case[1]
            .as_array()
            .unwrap()
            .iter()
            .map(Value::py_str)
            .collect();
        let got: Vec<String> = pystr::splitlines(input)
            .into_iter()
            .map(String::from)
            .collect();
        m.check("splitlines", input, got, want);
    }
    for case in rows(&fixture, "int") {
        let case = case.as_array().unwrap();
        let input = text(&case[0]).unwrap();
        m.check(
            "int",
            input,
            pystr::parse_int(input).map(|i| i.to_string()),
            option(&case[1]),
        );
    }
    for case in rows(&fixture, "float") {
        let case = case.as_array().unwrap();
        let input = text(&case[0]).unwrap();
        m.check(
            "float",
            input,
            pystr::parse_float(input).map(pystr::repr_float),
            option(&case[1]),
        );
    }
    for case in rows(&fixture, "strip") {
        let case = case.as_array().unwrap();
        let input = text(&case[0]).unwrap();
        m.check("strip", input, pystr::strip(input), text(&case[1]).unwrap());
        m.check(
            "lstrip",
            input,
            pystr::lstrip(input),
            text(&case[2]).unwrap(),
        );
        m.check(
            "rstrip",
            input,
            pystr::rstrip(input),
            text(&case[3]).unwrap(),
        );
    }
    m.assert_none();
}
