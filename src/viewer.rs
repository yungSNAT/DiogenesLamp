// Copyright 2026 yungSNAT
// SPDX-License-Identifier: Apache-2.0

//! Viewer assets bundled into the binary (the files setup.py copied into
//! wheels: .html, .css, .js and .json under viewer/).

pub static FILES: &[(&str, &[u8])] = &[
    ("age.js", include_bytes!("../viewer/age.js")),
    ("agents.js", include_bytes!("../viewer/agents.js")),
    ("anchor.js", include_bytes!("../viewer/anchor.js")),
    ("app.js", include_bytes!("../viewer/app.js")),
    ("catalogue.js", include_bytes!("../viewer/catalogue.js")),
    ("chips.js", include_bytes!("../viewer/chips.js")),
    ("dedup.js", include_bytes!("../viewer/dedup.js")),
    ("discovery.js", include_bytes!("../viewer/discovery.js")),
    ("filters.js", include_bytes!("../viewer/filters.js")),
    ("index.html", include_bytes!("../viewer/index.html")),
    ("kinds.js", include_bytes!("../viewer/kinds.js")),
    ("manifest.json", include_bytes!("../viewer/manifest.json")),
    ("prefs.js", include_bytes!("../viewer/prefs.js")),
    ("saved-work.js", include_bytes!("../viewer/saved-work.js")),
    ("shortcuts.js", include_bytes!("../viewer/shortcuts.js")),
    ("shortlist.js", include_bytes!("../viewer/shortlist.js")),
    ("style.css", include_bytes!("../viewer/style.css")),
    ("user-store.js", include_bytes!("../viewer/user-store.js")),
    ("view-state.js", include_bytes!("../viewer/view-state.js")),
];

pub fn get(name: &str) -> Option<&'static [u8]> {
    FILES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, data)| *data)
}
