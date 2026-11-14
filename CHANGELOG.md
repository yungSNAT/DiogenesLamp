# Changelog

## Unreleased

- An album whose purchase-link scan stopped early carries
  `links_truncated: true` in its record, so it stays marked after later runs
  skip it as unchanged, and `lamp doctor` counts such albums as
  `links_truncated`. Before, only that run's summary named it, and nothing did
  after the next run. A fetch that scans every word clears the mark.

## 0.1.2 (2026-11-13)

- `lamp` with no arguments, as a double-click on `lamp.exe` runs it, prints
  the next steps and exits 0 instead of a usage error with exit 2. On Windows,
  when stdin is a console, it waits for Enter so the window stays open. Any
  argument goes to the parser as before.
- Each seller's `lamp scrape` result line counts and names its new albums
  (stored for the first time) and returned albums (listed again after being
  marked gone). The run summaries in `data/runs/` are unchanged.
- The viewer's field baselines, checkbox boxes and preview form fields are
  drawn in the faint ink, at 5.57:1 or more on either leaf in both themes.
  The strong rule they used measured 1.79:1 to 2.32:1, under the 3:1 a
  control's edge needs.
- A focused search field or select keeps the viewer's 2px ink focus ring as
  well as its red baseline. The red baseline alone differs from the resting
  one mostly by hue, at 1.11:1 in the dark theme and 1.12:1 in the light.
- The viewer's album and seller tally and its description coverage line stay
  silent to screen readers whilst shards load, then announce their finished
  figures once. Before, they were announced up to 5 times a second.
- Purchase links are found word by word. Before, one word of a few hundred
  dotted parts with no space, such as `a.a.a.`, stopped the whole scan
  silently and every later link in the album was lost. Now only the rest of
  that word is skipped, and the album is counted and named on the result line
  as partly scanned and listed in its run summary as `partly_scanned_ids`.
- `lamp prepare-covers` saves its metadata snapshot only when a batch has a
  cover to prepare. A batch with nothing left reports `"snapshot": null`
  instead of adding another snapshot to `data/snapshots/`. An album whose
  cover file is missing from disk is skipped without counting against
  `--limit` and counted in the report's new `missing_covers`, for
  `lamp scrape` to fetch again; the count covers the albums the batch passed.
  Before, every run took a snapshot for it, spent a place in the batch on it
  and reported the same error, exiting 1, so with `--limit` or more such
  albums ahead, later covers were never prepared. A missing cover no longer
  makes the run exit 1.
- An album page with tens of thousands of void tags and stray closing tags, or
  one tag with tens of thousands of attributes, parses in linear time. At
  40,000 each took 0.49 s and 1.11 s before and 18 ms and 9 ms after.
- Duplicate detection records each duplicate's alias without searching every
  alias found so far, so its time no longer grows with the square of the
  number of duplicates: on 160,000 demo albums it falls from about 50 s to
  3 s. The selected albums and aliases are unchanged.

## 0.1.1 (2026-11-12)

The first release. v0.1.0 was tagged but never released: the release audit
refused its binary for rustix 1.1.5, which RUSTSEC-2026-0486 marks unsound.

- Takes rustix 1.1.6.

## 0.1.0 (Rust port, 2026-11-12, not released)

`lamp` is now a single Rust binary. It replaces the Python package
(`src/lamp`), its wheel packaging and its pytest suite. The command,
subcommands, flags, exit codes, status lines and written files match the
Python implementation at d8c03c3 on the fabricated samples and generated
differential corpus, apart from the divergences below. The viewer is
unchanged apart from the fixes under Viewer.

### Documented divergences

- **Re-encoded covers.** A cover that is resized or rotated (downloads and
  `prepare-covers`) is re-encoded by the `image` crate, not Pillow, so its
  bytes, `after_bytes` and `thumb_version` differ. WebP covers are re-encoded
  losslessly (Pillow wrote lossy quality 85), so they are larger. Covers that
  need no change keep their bytes, as before.
- **JPEG fingerprints.** `cover_hash` uses a port of Pillow's greyscale
  conversion and LANCZOS resampling, and matches exactly for PNG and WebP. JPEG
  covers decode through zune-jpeg rather than libjpeg-turbo: in a probe of 75
  generated JPEGs, 12 fingerprints differed, by 1 to 3 of 64 bits (most by 1),
  well inside the viewer's similarity threshold.
- **Image format coverage.** Fingerprints and resizing read JPEG, PNG and WebP.
  A GIF or other format placed in the cover cache by hand gets an empty
  `cover_hash` (Pillow fingerprinted it).
- **Snapshot archives.** Member names and contents match; the archive bytes do
  not (a different deflate implementation and member metadata).
- **Viewer assets** are built into the binary. Their responses carry no
  `Last-Modified` header and never answer `If-Modified-Since` with 304; data
  files under `/data/` behave as before. The `Server` header names
  DiogenesLamp instead of the Python version. The "viewer files not found"
  error can no longer occur.
- **Uncaught Python exceptions.** Where the Python CLI crashed with a traceback
  (an unreadable or invalid `sources.toml`, an unsafe id in `sources.toml`, a
  port already in use, an unreadable dump file), `lamp` prints `error: ...`
  and exits 1.
- **Seller ids.** A `[yupoo.*]` id outside `[a-z0-9_-]` is refused when
  `sources.toml` loads, with the id and the fix; Python loaded it, and every
  scrape of that seller then failed. An import row whose host gives an empty
  id is counted as invalid; Python stopped the whole import.
- **Short listings.** A listing that ends with 5 or more known albums
  missing (previously listed, or failed and never recorded; an empty or
  repeated page part-way through, as a soft block could cause) marks none of
  them gone and keeps their pending failures. The seller reports an error and
  records the missing IDs as `short_missing` in its run summary. A later run
  marks an album gone only if it is missing again from the next listing and
  that run is not aborted by blocking. The
  Python version marked them gone at once.
- **Unexpected seller failures.** A seller that fails unexpectedly reports
  `unexpected error: <message>` and writes the message to stderr instead of a
  Python traceback and exception repr.
- **Network error wording.** Messages that quote the HTTP library's own text
  (`request failed for ...: <detail>`, DNS failures on Linux) carry ureq's or
  the operating system's wording, not requests' or getaddrinfo's. Charsets
  other than UTF-8, ASCII and Latin-1 decode as UTF-8, and a page with no
  declared charset decodes as UTF-8 rather than by charset detection. A
  non-ASCII host is encoded with UTS 46 rather than Python's IDNA 2003 codec.
  A stalled response body times out after 120 seconds in total rather than 30
  seconds per read.
- **Retry-After.** A 429 carrying a valid `Retry-After` is retried once after
  that wait before it counts as blocked, and a 503's `Retry-After` replaces a
  shorter backoff. Each wait is capped at 60 seconds. The Python version
  ignored the header.
- **JSON edge cases.** A JSON string holding a lone surrogate escape
  (`"\ud800"`) is refused as corrupt metadata, because a Rust string cannot hold
  it; Python accepted it on read but could not write it back. Nesting deeper
  than 128 levels is refused with an error instead of a RecursionError.
- **TOML values.** A datetime value in `sources.toml` name, URL or platform is
  kept as its TOML text.
- **Terminal output** is never coloured (Python 3.15's argparse colours help
  on a terminal) and is always UTF-8. On Windows, as with Python, line breaks
  are written as CRLF.
- **Unicode tables.** Character classes follow Python 3.15 (Unicode 17); NFKC
  normalisation and IDNA mapping come from Rust crates, which can differ for
  characters added in the newest Unicode versions.

### New commands

- `lamp demo` writes synthetic sellers to a new directory under the system
  temporary directory, serves the viewer over them and removes them when you
  stop it with Ctrl+C. It never writes to `--data` or the working directory.
- `lamp --version` prints `lamp 0.1.0`.

### Viewer

- The collection, tag and saved-search filters, and the shops panel's
  platform groups, list in alphabetical order. They had used code-unit order,
  so 'Beta' came before 'alpha' and 'taobao global' before 'taobao'.
- Experimental: an English garment, colour, fit or material word in a search
  also finds the Chinese words sellers write for it ('hoodie' finds 卫衣), a
  card prints a Chinese title's words in English before the title, and
  Colour, Fit and Material filters read those words from English or Chinese
  titles. The words are a fixed table in `viewer/kinds.js`, with no brand
  names.
- The kind filter reads T恤 as a tee. The token was stored in uppercase and
  never matched a lowercased title.

### Development

- `cargo test` replaces pytest. It runs the viewer's node suite and the
  browser parity checks, and fails rather than skips without node.
- `cargo run --example make_demo_data` replaces `tests/make_demo_data.py`
  (same shape of data; its pseudo-random titles and ages differ). Its
  generator lives in the library as `lamp::demo`, which `lamp demo` shares.
- A `v*` tag on main publishes Linux and Windows executables through
  release-policy's binary release workflow. `VERSION` holds the release
  version and `rust-toolchain.toml` pins the exact compiler, 1.99.1.
- The `fixture-transport` feature (differential testing only) answers HTTPS
  requests from a local fixture directory.
- The opt-in browser qualification (`npm run test:browser` and
  `bench:browser`) runs the Rust example `browser_catalogue`, which replaces
  `tests/browser/catalogue.mjs`. It also writes `--output` when the overall
  deadline is exceeded.
