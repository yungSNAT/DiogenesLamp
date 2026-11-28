// Copyright 2026 yungSNAT
// SPDX-License-Identifier: Apache-2.0

//! The ports of Python's `json`, `urllib.parse` and `datetime.fromisoformat`
//! give what CPython gives. Catalogue files, links and dates pass through
//! them, so a difference here changes what is stored or matched. The
//! expected results in `fixtures/py_formats.json` were recorded from CPython
//! by `fixtures/py_parity.py`; its `generator` field names the command.
mod common;

use common::{Mismatches, get, option, rows, text};
use lamp::pyjson::{self, Style, Value};
use lamp::{isodate, pyurl};

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

#[test]
fn urllib_parse_matches_python() {
    let fixture = fixture();
    let mut m = Mismatches::default();
    for case in rows(&fixture, "urlsplit") {
        let input = text(get(case, "in")).unwrap();
        let split = match (pyurl::urlsplit(input), text(get(case, "error"))) {
            (Err(error), want) => {
                m.check("urlsplit error", input, Some(error.as_str()), want);
                continue;
            }
            (Ok(split), Some(want)) => {
                m.check(
                    "urlsplit accepted",
                    input,
                    format!("{split:?}"),
                    want.into(),
                );
                continue;
            }
            (Ok(split), None) => split,
        };
        let parts = get(case, "parts").as_array().unwrap();
        let got = [
            // An absent scheme is empty here and None with missing_as_none.
            (!split.scheme.is_empty()).then(|| split.scheme.clone()),
            split.netloc.clone(),
            Some(split.path.clone()),
            split.query.clone(),
            split.fragment.clone(),
        ];
        let want: Vec<Option<String>> = parts.iter().map(option).collect();
        m.check("parts", input, got.to_vec(), want);
        m.check(
            "hostname",
            input,
            split.hostname(),
            option(get(case, "hostname")),
        );
        m.check(
            "username",
            input,
            split.username(),
            option(get(case, "username")),
        );
        m.check(
            "password",
            input,
            split.password(),
            option(get(case, "password")),
        );
        let port = split.port().map(|p| p.map(|p| p.to_string()));
        let want = match text(get(case, "port_error")) {
            Some(error) => Err(error.to_string()),
            None => Ok(option(get(case, "port"))),
        };
        m.check("port", input, port, want);
    }
    for case in rows(&fixture, "urlunsplit") {
        let case = case.as_array().unwrap();
        let p: Vec<&str> = case[0]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let got = pyurl::urlunsplit(p[0], p[1], p[2], p[3], p[4]);
        m.check("urlunsplit", &p.join("|"), got, case[1].py_str());
    }
    for case in rows(&fixture, "urljoin") {
        let case = case.as_array().unwrap();
        let (base, url) = (text(&case[0]).unwrap(), text(&case[1]).unwrap());
        let want = match &case[2] {
            Value::Null => Err(case[3].py_str()),
            joined => Ok(joined.py_str()),
        };
        m.check(
            "urljoin",
            &format!("{base} {url}"),
            pyurl::urljoin(base, url),
            want,
        );
    }
    for case in rows(&fixture, "unquote") {
        let case = case.as_array().unwrap();
        let input = text(&case[0]).unwrap();
        m.check("unquote", input, pyurl::unquote(input), case[1].py_str());
        m.check(
            "unquote strict",
            input,
            pyurl::unquote_with(input, true).ok(),
            option(&case[2]),
        );
    }
    for case in rows(&fixture, "quote") {
        let case = case.as_array().unwrap();
        let (input, safe) = (text(&case[0]).unwrap(), text(&case[1]).unwrap());
        m.check("quote", input, pyurl::quote(input, safe), case[2].py_str());
    }
    for case in rows(&fixture, "parse_qsl") {
        let case = case.as_array().unwrap();
        let input = text(&case[0]).unwrap();
        let want: Vec<(String, String)> = case[1]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let p = p.as_array().unwrap();
                (p[0].py_str(), p[1].py_str())
            })
            .collect();
        m.check("parse_qsl", input, pyurl::parse_qsl(input), want);
    }
    m.assert_none();
}

fn int(value: &Value) -> Option<i64> {
    match value {
        Value::Int(i) => Some(i64::try_from(*i).unwrap()),
        Value::Null => None,
        other => panic!("{other:?}"),
    }
}

#[test]
fn isoformat_parsing_matches_python() {
    let fixture = fixture();
    let mut m = Mismatches::default();
    for case in rows(&fixture, "iso") {
        let input = text(get(case, "in")).unwrap();
        m.check(
            "date",
            input,
            isodate::parse_date(input).map(|d| d.ordinal),
            int(get(case, "date")),
        );
        let want = match get(case, "datetime") {
            Value::Array(pair) => Some((int(&pair[0]).unwrap(), int(&pair[1]))),
            _ => None,
        };
        m.check("datetime", input, isodate::parse_datetime(input), want);
    }
    m.assert_none();
}
