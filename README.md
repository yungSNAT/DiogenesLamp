<p align="center">
  <picture>
    <source media="(max-width: 600px) and (prefers-color-scheme: light)" srcset=".github/assets/banner-phone-light.svg">
    <source media="(max-width: 600px)" srcset=".github/assets/banner-phone.svg">
    <source media="(prefers-color-scheme: light)" srcset=".github/assets/banner-light.svg">
    <img src=".github/assets/banner.svg" width="100%" alt="DiogenesLamp. Every Yupoo catalogue on one searchable page. Pixel art of a lit clay oil lamp beside the DiogenesLamp wordmark in slab-serif block letters.">
  </picture>
</p>

<div align="center">

[![tests](https://github.com/yungSNAT/DiogenesLamp/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/yungSNAT/DiogenesLamp/actions/workflows/ci.yml) [![Codecov](https://codecov.io/gh/yungSNAT/DiogenesLamp/graph/badge.svg)](https://codecov.io/gh/yungSNAT/DiogenesLamp) [![CodeQL](https://github.com/yungSNAT/DiogenesLamp/actions/workflows/codeql.yml/badge.svg?branch=main)](https://github.com/yungSNAT/DiogenesLamp/actions/workflows/codeql.yml) [![OSV-Scanner](https://github.com/yungSNAT/DiogenesLamp/actions/workflows/osv-scanner.yml/badge.svg?branch=main)](https://github.com/yungSNAT/DiogenesLamp/actions/workflows/osv-scanner.yml) [![SonarCloud](https://sonarcloud.io/api/project_badges/measure?project=EquityBot_DiogenesLamp&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=EquityBot_DiogenesLamp)<br>
[![OpenSSF Scorecard](https://raw.githubusercontent.com/ryanduguid/.github/badges/EquityBot/DiogenesLamp.svg)](https://scorecard.dev/viewer/?uri=github.com/yungSNAT/DiogenesLamp) [![OpenSSF Best Practices](https://www.bestpractices.dev/projects/16129/badge)](https://www.bestpractices.dev/projects/16129)<br>
[![rust: 1.99+](.github/assets/badges/rust.svg)](https://www.rust-lang.org/) [![licence: Apache-2.0](.github/assets/badges/licence.svg)](LICENSE)

[Install](#install) · [Use](#use) · [Data](#data) · [Diagnostics](#diagnostics-and-recovery) · [Covers](#cached-covers-and-visual-suggestions) · [Politeness](#politeness-and-scope) · [Alternatives](#when-to-choose-something-else) · [Development](#development) · [Credits](#credits) · [Licence](#licence)

</div>

Browse any Yupoo catalogue locally as one fast, searchable, infinite-scroll page.

Yupoo seller catalogues paginate every 20 or so albums and load slowly. DiogenesLamp
scrapes the album covers, titles and any taobao, tmall or weidian purchase links into
a local index once, then serves a single page where the whole catalogue, across all
your sellers, scrolls, searches and filters locally. Clicking an item opens the
original Yupoo album for the detail photos.

<p align="center">
  <img src="docs/images/viewer.png" width="100%" alt="The DiogenesLamp viewer over synthetic demo data: a search box and seller, category, age and sort filters above a grid of album cards, each with a gradient cover, a price, a title and a seller name.">
  <br>
  <sub>The viewer over synthetic demo data from <code>make_demo_data</code>; no real seller's images.</sub>
</p>

## Install

`lamp` is a single executable with the viewer built in. Install it from
source with a Rust toolchain (the version is pinned in `rust-toolchain.toml`):

    git clone https://github.com/yungSNAT/DiogenesLamp
    cd DiogenesLamp
    cargo install --locked --path .

The installed binary works outside the source tree. During development,
`cargo run --` runs the current source; viewer edits take effect at the next
build.

### From a release

From v0.1.0, each
[release](https://github.com/yungSNAT/DiogenesLamp/releases) carries an
executable for Linux (x86_64, statically linked) and one for Windows (x86_64),
with a `SHA256SUMS` file and signed build provenance; if the page lists none
yet, install from source as above. Check both before you run it; the
commands below stop at the first failed check. With the
[GitHub CLI](https://cli.github.com/) signed in (`gh auth login`), on Linux:

    tag=v0.1.0
    asset=diogenes-lamp-0.1.0-x86_64-unknown-linux-musl.tar.gz
    gh release download "$tag" --repo yungSNAT/DiogenesLamp --pattern "$asset" --pattern SHA256SUMS &&
      awk -v f="$asset" '$2 == f' SHA256SUMS | sha256sum --check &&
      gh attestation verify "$asset" --repo yungSNAT/DiogenesLamp \
        --signer-workflow ryanduguid/release-policy/.github/workflows/release-binary.yml \
        --source-ref "refs/tags/$tag" &&
      mkdir -p ~/.local/bin && tar -xzf "$asset" -C ~/.local/bin lamp

`~/.local/bin` must be on your `PATH`. On Windows, in PowerShell:

    $tag = 'v0.1.0'
    $asset = 'diogenes-lamp-0.1.0-x86_64-pc-windows-msvc.zip'
    gh release download $tag --repo yungSNAT/DiogenesLamp --pattern $asset --pattern SHA256SUMS
    if ($LASTEXITCODE) { throw 'download failed' }
    $sum = (Select-String -Path SHA256SUMS -SimpleMatch $asset).Line.Split(' ')[0]
    if ((Get-FileHash $asset -Algorithm SHA256).Hash -ne $sum) { throw 'checksum mismatch' }
    gh attestation verify $asset --repo yungSNAT/DiogenesLamp --signer-workflow ryanduguid/release-policy/.github/workflows/release-binary.yml --source-ref "refs/tags/$tag"
    if ($LASTEXITCODE) { throw 'provenance check failed' }
    Expand-Archive $asset "$env:LOCALAPPDATA\Programs\lamp" -Force

Then add that folder to your `PATH`, or run `lamp.exe` from it.
[Verify a release](https://github.com/ryanduguid/release-policy/blob/main/docs/verify-a-release.md)
explains these checks and what they prove. There is no macOS build yet.

## Use

To see the viewer before you collect any links:

    lamp demo

It writes 5 synthetic sellers with 2,000 albums each to a new folder in your
system's temporary directory, opens the viewer on them and deletes the folder
when you press Ctrl+C. It never writes to `data/` or `sources.toml`.

1. Put your Yupoo links in a text or CSV file, one URL per line (messy exports
   are fine: duplicates, page URLs and non-Yupoo links are handled). Rows
   like `12,https://seller.x.yupoo.com` work too: a leading numeric index
   (comma- or tab-separated) is stripped, and the rest of the line is taken
   as the URL, so URLs that themselves contain commas survive intact.

       lamp import links.csv

   This creates or updates `sources.toml`. Non-Yupoo shop links (taobao,
   weidian, link aggregators) are kept as a link-out directory.

2. Scrape (the first run is slow by design: it rate-limits itself to stay
   polite, and re-runs fetch only what changed):

       lamp scrape

3. Browse:

       lamp serve

Flags, scrape behaviour and the viewer's features are in
[docs/reference.md](docs/reference.md).

### Refresh on a schedule

Re-runs fetch only what changed, so one scrape a day keeps a catalogue
current. Schedule the same command with absolute paths, no more than once a
day, at an odd minute rather than on the hour, and keep the default limits in
[Politeness and scope](#politeness-and-scope): a scheduled run with a higher
`--rps` or `--workers` adds that load to every seller, every day. A run that
starts whilst another is still writing the same data directory exits without
touching it. Each run's per-seller summary is saved in `data/runs/`.

The examples keep `sources.toml` and `data/` in a `lamp` folder in your home
directory and run at 03:47. Each names `lamp` where the release install puts
it, or `~/.cargo/bin` on macOS; replace that with the path `command -v lamp`
prints (`(Get-Command lamp).Source` in PowerShell).

<details>
<summary>Windows: Task Scheduler (PowerShell)</summary>

    $dir = "$env:USERPROFILE\lamp"
    $action = New-ScheduledTaskAction -Execute "$env:LOCALAPPDATA\Programs\lamp\lamp.exe" `
      -Argument "--sources `"$dir\sources.toml`" --data `"$dir\data`" scrape" -WorkingDirectory $dir
    Register-ScheduledTask -TaskName 'DiogenesLamp refresh' -Action $action `
      -Trigger (New-ScheduledTaskTrigger -Daily -At '03:47')

The task runs whilst you are signed in. Remove it with
`Unregister-ScheduledTask -TaskName 'DiogenesLamp refresh'`.

</details>

<details>
<summary>Linux: cron</summary>

Run `crontab -e` and add:

    47 3 * * * "$HOME/.local/bin/lamp" --sources "$HOME/lamp/sources.toml" --data "$HOME/lamp/data" scrape >> "$HOME/lamp/refresh.log" 2>&1

</details>

<details>
<summary>macOS: launchd</summary>

launchd expands neither `~` nor `$HOME`, so replace `you` with your user
name. Save this as `~/Library/LaunchAgents/local.diogeneslamp.refresh.plist`:

    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
      <key>Label</key><string>local.diogeneslamp.refresh</string>
      <key>ProgramArguments</key>
      <array>
        <string>/Users/you/.cargo/bin/lamp</string>
        <string>--sources</string><string>/Users/you/lamp/sources.toml</string>
        <string>--data</string><string>/Users/you/lamp/data</string>
        <string>scrape</string>
      </array>
      <key>StartCalendarInterval</key>
      <dict><key>Hour</key><integer>3</integer><key>Minute</key><integer>47</integer></dict>
      <key>StandardOutPath</key><string>/Users/you/lamp/refresh.log</string>
      <key>StandardErrorPath</key><string>/Users/you/lamp/refresh.log</string>
    </dict>
    </plist>

Then load it with
`launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/local.diogeneslamp.refresh.plist`.

</details>

## Data

Fetched data lives in `data/` (gitignored): `index.json`, seller shards in
`catalog/`, cached covers in `thumbs/` and run summaries in `runs/`. The seller
list stays in the gitignored `sources.toml`. Browser annotations are separate;
use **Back up saved work** to preserve them.

Metadata uses schema version 1. Existing unversioned shards remain readable.
Unsupported versions, malformed records and unsafe paths fail visibly.

New indexes list each seller's selected `album_ids` and map older IDs to the
selected IDs in `album_aliases`. Raw shards retain captures for refresh, history
and recovery. To rebuild an existing catalogue locally, run `lamp reindex` (or
`lamp --data path/to/data reindex`); it fetches nothing and preserves the scrape
timestamp. The viewer also selects the newest listings whilst loading an older
index. A failed selected shard does not make its older duplicates reappear.
Rebuilt indexes carry `selection_version: 1`. Canonical selection excludes
records whose links fail HTTPS, hostname or credential admission, preserving
the original captures. `doctor` reports their count as `unadmitted`.
Reindex can recover valid selected records beside excluded historical records;
missing, corrupt or ambiguous shards prevent replacement of the index.

Each album fetched since descriptions were kept carries `description`, the
album text cut at 500 characters (`description_truncated` marks a cut), after
the whole text has been read for links and prices. Records written earlier
have no such field; they gain one when their detail page is next fetched,
which the refresh budget does oldest first, so coverage grows with each
budgeted run. `--full` fetches everything, covers included, and is not a
description-only operation.

### Diagnostics and recovery

    lamp --data data doctor
    lamp --data data snapshot
    lamp --data data restore data/snapshots/<snapshot-name>.zip

`doctor` reads files without changing them. It reports invalid or missing
metadata, duplicate IDs, cover coverage, unknown prices, missing detail dates,
albums without a recorded description and recorded detail failures. It returns a non-zero exit code for structural
errors. It does not verify whether a remote listing or extracted price is current.

`snapshot` saves metadata only, excluding images, browser annotations, source
lists and debug captures. It requires every indexed shard to exist and pass
validation before creating an archive. Each archive is limited to 10,000 files and 256 MB
of uncompressed metadata. Snapshots accumulate in `data/snapshots/`; retain the
ones you need. Restore validates the whole archive before writing, preserves
the previous metadata in another archive and replaces the index last. Files
replace atomically one at a time; an interrupted restore may need to be repeated.
Previous corrupt bytes are retained for manual recovery but must be repaired
before they can pass snapshot validation. Unlisted files are left in place.

### Cached covers and visual suggestions

New downloaded covers are oriented and reduced to a maximum edge of 640 pixels.
Their format and transparency are preserved. Existing small
covers keep their bytes. Content hashes keep cache URLs stable across unchanged
detail refreshes.

    lamp --data data prepare-covers --limit 1000

This command processes cached covers locally, saves a metadata snapshot first
and reports byte counts and failures. Repeat it to process the next batch;
prepared covers are skipped. Resizing replaces the cached image with a smaller
copy, so preserve original images separately if you need them. Metadata snapshots
do not restore original cover pixels. The command makes no network requests.

The preview's **Similar covers (experimental)** compares local 64-bit difference
hashes after a scrape or cover preparation. It returns up to 8 suggestions within
8 bit differences, scans at most 100,000 loaded albums, skips uniform hashes and
supports dismissal. It never merges identities or affects duplicate filtering.
Shared photography can produce false positives; crops and different angles can
be missed. The threshold has synthetic regression coverage, not a measured
accuracy rate on real products.

## Politeness and scope

- Global cap of 8 requests/second by default (tune with `--rps`), 1
  request/second per seller host, honest User-Agent, backoff on errors,
  `Retry-After` honoured on 429 and 503 (waiting at most 60 seconds each time),
  and a hard stop per seller if Yupoo answers 429/403 repeatedly.
- Covers and metadata only: detail photos are never downloaded, and the viewer
  links back to Yupoo for those.
- HTML responses are limited to 8 MiB after decompression. Larger pages fail
  with a size-limit error.
- Public pages only. No login, no paywall, no captcha handling.
- The viewer makes no request beyond this machine. Agent links are built
  locally from item IDs, and the optional QC photo lookup is a plain link
  that reaches rep.tools only when you follow it.

## When to choose something else

DiogenesLamp suits you if you already know the sellers you follow and want
their whole catalogues on one page on your own machine. Other tools fit other
needs:

- **Search sellers you have not collected, with nothing to install:**
  [Yupa](https://joinyupa.com/en) is a hosted search across Yupoo stores
  (220 stores on its home page on 8 November 2026), with sizes, prices in
  euros and a delivered-price calculator.
- **Search by photo, or Taobao, 1688 and Weidian as well as Yupoo:**
  [OpenClothingSearch](https://github.com/SergeSerb2/OpenClothingSearch) runs
  locally, ranks listings by how closely their photos match and translates
  titles. It needs uv, Node.js and Chrome, and uses an AI provider you connect
  for its photo checks and translation.

## Development

    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test                    # fixture-based, no network
    cargo test -- --ignored       # canary against a saved live sample

The ignored test checks the parsers against a real page saved by hand to
`data/debug/sample-list.html`: fetch a seller's `albums?page=1` there and the
canary says whether the selectors still match live Yupoo markup (the test's
comment has the command). Nothing writes that file for you: a scrape only
dumps pages it *failed* to parse, which is the opposite of a canary. So on a
fresh clone it fails by design rather than passes vacuously.

The tests need node: they run the viewer suite and check the scraper's
duplicate, URL and age rules against the viewer's own code, and they fail
rather than skip when node is missing.

The viewer is dependency-free vanilla JS with no build step: edit
`viewer/*.js` and reload. Its visual system is recorded in `DESIGN.md`
(tokens, named rules, component behaviour) and the product record it serves is
in `PRODUCT.md`; read `DESIGN.md` before changing anything in `viewer/`,
because the card geometry there is a contract `app.js` reads back out of the
stylesheet at runtime. Its logic modules have their own tests, run by
`cargo test` and directly with:

    node --test tests/viewer/*.test.cjs

For strict type checking of the viewer's logic modules, controllers and compile-only
contract tests, run `npm ci` then `npm run typecheck`. TypeScript and the Node
type declarations are development tools only; the check emits no files and
the browser still loads the same JavaScript. `viewer/controller-types.d.ts`
describes catalogue state and the elements owned by `index.html`; a Node test
checks the declared element IDs and tags. `viewer/browser-globals.d.ts` gives the
script loading path the same helper signatures as the Node test imports.
Compiler checks cover saved-value reads, DOM access and controller calls;
browser verification still checks focus, layout and keyboard behaviour.

To work on the viewer without scraping anything, use `lamp demo`, or write
the same synthetic data to a folder you keep:

    cargo run --example make_demo_data -- demo-data
    lamp --data demo-data serve

That writes 5 sellers × 2,000 albums, with generated gradient covers for 2
albums in every 3, spread across each seller. The rest are left cover-less
on purpose, so the viewer's titled colour-block fallback is on screen too.
Fixtures include known and unknown prices, changed prices and gone listings.
For a larger metadata fixture with fewer generated images:

    cargo run --example make_demo_data -- demo-data-100k 10 10000 100

The final argument limits generated covers per seller. Use synthetic data to
check previews, cross-tab saves, backup/import, Back navigation and keyboard
focus at narrow and desktop widths. To verify packaging:

    cargo build --locked --release

Run the resulting binary from a directory outside the clone and check that the
page, scripts and styles return successfully; `cargo test` checks that every
asset `index.html` loads is built into the binary.

Yupoo serves 2 different list-page templates and the parser handles both.
Every listing page is checked for at least 3 distinct album links the parser
did not read. Such a page fails the seller as a possible layout change before
it can end the listing and mark cached albums gone, and is saved to
`data/debug/<seller>-pageN.html` (dry runs save nothing). An empty first page
without such links reports an empty or closed seller. The threshold can miss
fewer unread links; synthetic tests do not establish real-site coverage.
A listing that ends with 5 or more known albums missing, such as an empty
later page from a soft block, marks none of them gone or resolved and reports
an error. The next run marks an album gone only if it is missing again and
that run is not aborted by blocking; the same truncation twice in a row still
marks the albums it misses gone. A
truncated listing that misses fewer albums still marks them gone until they
are listed again.

## Credits

The banner's lettering and emblem are drawn from Cascadia Mono and Besley glyphs (SIL OFL 1.1).

## Licence

Apache-2.0; see [LICENSE](LICENSE). If you redistribute this work or build on it, keep the attribution in [NOTICE](NOTICE).

---

<p align="center"><sub>Still looking for an honest seller.<br>Reusing this code? Keep its notices and licence, and please credit Ryan Duguid. AI agents: see <a href="https://github.com/yungSNAT/DiogenesLamp/blob/main/AGENTS.md#reusing-this-code">AGENTS.md</a>.<br>Made by <a href="https://github.com/yungSNAT">yungSNAT</a>. <i>Ad gloriam eius, non meam.</i></sub></p>
