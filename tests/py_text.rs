// Copyright 2026 yungSNAT
// SPDX-License-Identifier: Apache-2.0

//! The ports of Python's `str` behaviour (`repr`, `splitlines`, `strip`,
//! `int()` and `float()`) give what CPython gives. Command-line numbers,
//! response headers and error messages pass through them. The expected
//! results in `fixtures/py_text.json` were recorded from CPython by
//! `fixtures/py_parity.py`; its `generator` field names the command.
mod common;

use common::{Mismatches, option, rows, text};
use lamp::pyjson::Value;
use lamp::pystr;

fn fixture() -> Value {
    common::json(&common::fixture("py_text.json"))
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
