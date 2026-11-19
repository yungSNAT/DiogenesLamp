// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! The port of Python's `json` gives what CPython gives. Catalogue files
//! pass through it, so a difference here changes what is stored. The
//! expected results in `fixtures/py_formats.json` were recorded from CPython
//! by `fixtures/py_parity.py`; its `generator` field names the command.
mod common;

use common::{Mismatches, get, rows, text};
use lamp::pyjson::{self, Style, Value};

fn fixture() -> Value {
    common::json(&common::fixture("py_formats.json"))
}

#[test]
fn json_matches_python() {
    let fixture = fixture();
    let mut m = Mismatches::default();
    for case in rows(&fixture, "json") {
        let input = text(get(case, "in")).unwrap();
        let loaded = pyjson::loads(input);
        // A deliberate difference: CPython keeps a lone surrogate, which a Rust
        // string cannot hold, so the port refuses the document instead.
        if case
            .as_object()
            .unwrap()
            .contains_key("differs_from_cpython")
        {
            let error = loaded.map(|_| ()).unwrap_err().0;
            m.check(
                "lone surrogate",
                input,
                error.starts_with("lone surrogate \\ud83d is not supported"),
                true,
            );
            continue;
        }
        match (loaded, text(get(case, "error"))) {
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
