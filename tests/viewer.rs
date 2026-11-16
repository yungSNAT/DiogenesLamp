// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Ported from tests/test_viewer_dedup.py (the node suite runner) and
//! tests/test_viewer_source.py (source-level viewer contracts and file hygiene).
mod common;

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::process::Command;

use regex::Regex;

/// The viewer's logic modules have node tests; they must run, never skip.
#[test]
fn viewer_js_unit_tests_pass_under_node() {
    let node = common::node().expect("node is required: the viewer suite must run, not skip");
    let dir = common::root().join("tests").join("viewer");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".test.cjs"))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no viewer JS test files found - glob broken?"
    );
    let output = Command::new(node)
        .arg("--test")
        .args(&files)
        .current_dir(common::root())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const TEXT_SUFFIXES: [&str; 13] = [
    ".rs", ".js", ".cjs", ".json", ".toml", ".md", ".css", ".html", ".yml", ".yaml", ".lock",
    ".ts", ".py",
];

/// Hand-edited text files, as git tracks them.
fn text_sources() -> Vec<PathBuf> {
    let root = common::root();
    let listed = Command::new("git")
        .args(["-C", &root.to_string_lossy(), "ls-files", "-z"])
        .output()
        .unwrap();
    assert!(
        listed.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&listed.stderr)
    );
    String::from_utf8_lossy(&listed.stdout)
        .split('\0')
        .filter(|n| !n.is_empty() && TEXT_SUFFIXES.iter().any(|s| n.ends_with(s)))
        .map(|n| root.join(n))
        .filter(|p| p.is_file())
        .collect()
}

#[test]
fn source_files_carry_no_nul_bytes() {
    let sources = text_sources();
    assert!(
        !sources.is_empty(),
        "found no source files to check - has the layout moved?"
    );
    for path in sources {
        assert!(
            !std::fs::read(&path).unwrap().contains(&0),
            "{} holds a NUL byte",
            path.display()
        );
    }
}

fn app_js() -> String {
    common::read(&common::root().join("viewer/app.js"))
}

#[test]
fn app_js_reaches_storage_only_through_the_guarded_accessors() {
    let source = app_js();
    let raw: Vec<&str> = Regex::new(r"localStorage\.\w+")
        .unwrap()
        .find_iter(&source)
        .map(|m| m.as_str())
        .collect();
    assert!(
        raw.is_empty(),
        "unguarded storage access in app.js: {raw:?}"
    );
    let calls: Vec<(String, String)> =
        Regex::new(r"\b(readStored|writeStored|readFilters|writeFilters)\(([^,)]*)")
            .unwrap()
            .captures_iter(&source)
            .map(|c| (c[1].to_string(), c[2].to_string()))
            .collect();
    assert!(
        !calls.is_empty(),
        "app.js no longer routes storage through the guarded accessors"
    );
    for (name, first) in calls {
        assert_eq!(
            first.trim(),
            "window",
            "{name}() is handed {first:?}; pass the host instead"
        );
    }
}

#[test]
fn every_focusable_control_on_a_card_carries_the_roving_tabindex() {
    let source = app_js();
    let selector = Regex::new(r#"CARD_STOPS = "([^"]+)""#)
        .unwrap()
        .captures(&source)
        .expect("app.js no longer declares CARD_STOPS");
    let stops: BTreeSet<String> = selector[1]
        .split(',')
        .map(|p| p.trim().to_string())
        .collect();
    let built: BTreeSet<String> = Regex::new(r#"<(?:a|button) class="([a-z0-9-]+)""#)
        .unwrap()
        .captures_iter(&source)
        .map(|c| c[1].to_string())
        .collect();
    assert!(
        !built.is_empty(),
        "found no statically classed card controls - has makeCard changed?"
    );
    for class in built {
        assert!(
            stops.contains(&format!(".{class}")),
            ".{class} is focusable but not a roving stop"
        );
    }
    assert!(stops.contains(".card-links a"));
}

#[test]
fn the_roving_tab_stop_follows_the_cursor_wherever_it_moves() {
    let source = app_js();
    let setter = Regex::new(r"(?s)\nfunction setCursor\(next\) \{\n(.*?)\n\}\n")
        .unwrap()
        .captures(&source)
        .expect("app.js no longer declares setCursor");
    assert!(
        setter[1].contains("setCardTabStop("),
        "setCursor moves the cursor without moving the tab stop"
    );
    let focusin = Regex::new(r#"(?s)addEventListener\("focusin".*?\n\}\);"#)
        .unwrap()
        .find(&source)
        .expect("app.js no longer binds focusin on the grid");
    assert!(
        focusin.as_str().contains("setCursor("),
        "focusin moves the cursor without moving the tab stop"
    );
    assert!(
        source.contains("cursor: state.view.cursor"),
        "the render key omits the cursor"
    );
}

#[test]
fn the_render_loop_refreshes_the_title_of_every_card_it_reuses() {
    let source = app_js();
    let render = Regex::new(r"(?s)\nfunction renderGrid\(force = false\) \{\n(.*?)\n\}\n")
        .unwrap()
        .captures(&source)
        .expect("app.js no longer declares renderGrid");
    let reuse = Regex::new(r"(?s)let card = cardCache\.get\(key\);\n(.*?)\n    if \(!card\) \{")
        .unwrap()
        .captures(&render[1])
        .expect("renderGrid no longer has a card reuse branch");
    assert!(
        reuse[1].contains("refreshCardTitle("),
        "renderGrid reuses a card without refreshing its title"
    );
    let maker = Regex::new(r"(?s)\nfunction makeCard\(album\) \{\n(.*?)\n\}\n")
        .unwrap()
        .captures(&source)
        .expect("app.js no longer declares makeCard");
    assert!(
        maker[1].contains("refreshCardTitle("),
        "makeCard fills the title without stamping the query it used"
    );
}

// Ported from tests/viewer/contrast.test.cjs: colour contrast read from
// viewer/style.css. Patterns use ASCII classes, as JavaScript's do.

fn viewer_file(name: &str) -> String {
    common::read(&common::root().join("viewer").join(name))
}

/// The rule body from `selector {` to the next `}`.
fn css_block<'a>(css: &'a str, selector: &str) -> &'a str {
    let start = css
        .find(&format!("{selector} {{"))
        .unwrap_or_else(|| panic!("style.css no longer has {selector}"));
    &css[start..start + css[start..].find('}').unwrap()]
}

fn colour_tokens(css: &str, selector: &str) -> HashMap<String, String> {
    Regex::new(r"(--[A-Za-z0-9_-]+):\s*(#[0-9a-f]{6})")
        .unwrap()
        .captures_iter(css_block(css, selector))
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect()
}

fn luminance(hex: &str) -> f64 {
    let [r, g, b] = [1, 3, 5].map(|i| {
        let c = f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap()) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn contrast(a: &str, b: &str) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Each theme's tokens: dark from `:root`, light overriding it.
fn themes(css: &str) -> [(&'static str, HashMap<String, String>); 2] {
    let dark = colour_tokens(css, ":root");
    let mut light = dark.clone();
    light.extend(colour_tokens(css, r#":root[data-theme="light"]"#));
    [("dark", dark), ("light", light)]
}

/// `token` against both leaves in both themes, at 3:1 or more.
fn assert_clears_3_to_1(css: &str, name: &str, token: &str) {
    for (theme, palette) in themes(css) {
        for ground in ["--leaf", "--leaf-2"] {
            let measured = contrast(&palette[token], &palette[ground]);
            assert!(
                measured >= 3.0,
                "{name} {token} on {ground} ({theme}): {measured:.2}:1"
            );
        }
    }
}

// WCAG 1.4.11: an edge that is the only sign of a control needs 3:1 against
// what it sits on. Fields and mark boxes sit on the leaf and the raised leaf.
#[test]
fn control_edges_clear_3_to_1_on_both_leaves_in_both_themes() {
    let css = viewer_file("style.css");
    let edges = [
        (
            "field baseline",
            r"border-bottom:\s*1px solid var\((--[A-Za-z0-9_-]+)\)",
            r#"input[type="search"], select"#,
        ),
        (
            "mark box",
            r"border:\s*1px solid var\((--[A-Za-z0-9_-]+)\)",
            ".mark-check",
        ),
        (
            "preview field",
            r"border:\s*1px solid var\((--[A-Za-z0-9_-]+)\)",
            "#preview-form input, #preview-form textarea",
        ),
    ];
    for (name, pattern, selector) in edges {
        let token = Regex::new(pattern)
            .unwrap()
            .captures(css_block(&css, selector))
            .unwrap_or_else(|| panic!("{name}: no edge colour"))[1]
            .to_string();
        assert_clears_3_to_1(&css, name, &token);
    }
}

// The red focused baseline is about 1.1:1 against the faint resting one, so a
// focused field must keep the shared focus ring, which must clear 3:1.
#[test]
fn a_focused_field_keeps_the_shared_focus_ring() {
    let css = viewer_file("style.css");
    assert!(!Regex::new(r"outline:\s*none").unwrap().is_match(css_block(
        &css,
        r#"input[type="search"]:focus, select:focus"#
    )));
    let ring = Regex::new(r"outline:\s*2px solid var\((--[A-Za-z0-9_-]+)\)")
        .unwrap()
        .captures(css_block(&css, ":focus-visible"))
        .expect("no focus ring colour")[1]
        .to_string();
    assert_clears_3_to_1(&css, "focus ring", &ring);
}

// Ported from tests/viewer/dom-contracts.test.cjs: the controller's element
// declarations and headings against viewer/index.html.

#[test]
fn controller_dom_declarations_match_the_shipped_html_elements() {
    let html = viewer_file("index.html");
    let types = viewer_file("controller-types.d.ts");
    let declarations = types
        .split("export interface ViewerElements {")
        .nth(1)
        .expect("controller-types.d.ts no longer declares ViewerElements")
        .split("\n}")
        .next()
        .unwrap();
    let tags: HashMap<String, String> =
        Regex::new(r#"<([a-z][a-z0-9]*)(?-u:\b)[^>]*(?-u:\b)id="([^"]+)""#)
            .unwrap()
            .captures_iter(&html)
            .map(|c| (c[2].to_string(), c[1].to_string()))
            .collect();
    let element_tags: HashMap<&str, &str> = [
        ("HTMLInputElement", "input"),
        ("HTMLSelectElement", "select"),
        ("HTMLButtonElement", "button"),
        ("HTMLDivElement", "div"),
        ("HTMLDialogElement", "dialog"),
        ("HTMLDetailsElement", "details"),
        ("HTMLImageElement", "img"),
        ("HTMLUListElement", "ul"),
        ("HTMLTextAreaElement", "textarea"),
        ("HTMLFormElement", "form"),
        ("SVGUseElement", "use"),
    ]
    .into();
    let controls: Vec<_> = Regex::new(r"(?:'([^']+)'|([A-Za-z0-9_]+)):\s*([A-Za-z0-9_]+)")
        .unwrap()
        .captures_iter(declarations)
        .collect();
    assert!(controls.len() > 50, "the complete control map is checked");
    for control in controls {
        if control
            .get(2)
            .is_some_and(|m| m.as_str() == "parentElement")
        {
            continue;
        }
        let id = control.get(1).or(control.get(2)).unwrap().as_str();
        let declared = &control[3];
        let tag = tags
            .get(id)
            .unwrap_or_else(|| panic!("missing declared element: {id}"));
        if let Some(expected) = element_tags.get(declared) {
            assert_eq!(tag, expected, "{id}: {declared}");
        }
    }
    assert!(
        Regex::new(r#"<div id="load-progress"[^>]*><div id="load-progress-bar""#)
            .unwrap()
            .is_match(&html),
        "the progress bar has the parent promised by its DOM contract"
    );
}

#[test]
fn every_heading_in_the_shipped_html_has_text_before_app_js_fills_it() {
    let html = viewer_file("index.html");
    // The closing tag repeats the opening level, which needs a backreference.
    let headings: Vec<(String, String)> =
        fancy_regex::Regex::new(r"<(h[1-6])\b[^>]*>([\s\S]*?)</\1>")
            .unwrap()
            .captures_iter(&html)
            .map(|c| {
                let c = c.unwrap();
                let text = |i| c.get(i).unwrap().as_str().to_string();
                (text(0), text(2))
            })
            .collect();
    assert!(!headings.is_empty(), "the preview title is checked");
    let text = Regex::new(r"(?:^|>)[^<]*[^<\s]").unwrap();
    for (heading, inner) in headings {
        // Some character outside every tag that is not whitespace.
        assert!(text.is_match(&inner), "empty heading: {heading}");
    }
}
