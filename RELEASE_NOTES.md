# v0.1.3

New commands and alerts for `lamp`, the DiogenesLamp command, and
executables for 6 systems: Linux (x86_64 and arm64, statically linked), macOS
(Apple silicon and Intel) and Windows (x86_64 and arm64).

- `lamp scrape --notify COMMAND` runs your command after a run that stored or
  changed albums, with them as JSON on its stdin; the README has recipes for
  a phone alert (ntfy), Telegram and email. Albums from an interrupted run are
  reported by the next `--notify` run.
- `lamp completions <shell>` prints shell completion scripts for `bash`,
  `elvish`, `fish`, `powershell` and `zsh`.
- `lamp export` prints the catalogue as CSV or JSON, optionally only what was
  added since a date, and `lamp export --site DIR` writes the viewer and
  catalogue as static files for any web host.
- `lamp find-photo IMAGE` lists the albums whose cover is the same picture,
  resized or recompressed.
- The local viewer adds a browser-stored shortlist (optional size, colour and
  note fields, a Shortlist filter and JSON or Markdown export and clipboard
  copy), keyboard shortcuts (`s`, `o`, `a`, `/` and `?`), 1-click material
  filter chips, and preferred-seller filtering and sorting.
- `sources.toml` supports `preferred = true` on sellers and shops (prioritising
  them in duplicate selection and default sort) and `[blocklist]`
  `skip_sellers` to exclude blocked sellers from `lamp import` and
  `lamp scrape`.
- The viewer's preview lists every listing of an item, cheapest first, and an
  optional `aliases.json` lets a search for your own brand and model names
  find titles that use the Chinese names and seller codes your sellers write.
- More shopping agents in the **Open purchase links on** menu: OrientDig,
  PonyBuy, Fishgoo and LoloBuy.
- A scrape notices a changed photo count from the seller's listing without
  fetching the album, and a seller that fails part-way still reports its new
  albums and records its failed fetches for `--retry-failed`.
- Release archives support `cargo binstall diogenes-lamp`, strip binary
  symbols, and use `mimalloc` (`no_thp`) with native musl errno wording on
  Linux musl builds.
- `lamp restore` refuses a snapshot that names a member twice, and
  `lamp reindex` over 400 sellers of 200 albums is 3 to 4 times faster.

Check a download before running it, as
[Download](https://github.com/yungSNAT/DiogenesLamp/blob/v0.1.3/README.md#download)
describes. Everything that changed is in
[CHANGELOG.md](https://github.com/yungSNAT/DiogenesLamp/blob/v0.1.3/CHANGELOG.md).
