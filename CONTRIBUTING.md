# Contributing

Report bugs and request features in
[GitHub issues](https://github.com/yungSNAT/DiogenesLamp/issues). For a
suspected security vulnerability, follow [SECURITY.md](SECURITY.md) instead of
opening an issue.

## Pull requests

Propose changes as a pull request against `main`. A change lands on `main` only
when the `test (ubuntu-latest)` and `test (windows-latest)` checks from
[ci.yml](.github/workflows/ci.yml) pass. The `No AI attribution` check fails a
pull request whose title, commit trailers or identities credit an AI tool.
To catch one before it is pushed, install the local `commit-msg` hook once per
clone from an up-to-date `main`; the commands are in the header of
[.githooks/commit-msg](.githooks/commit-msg). The hook runs only the guard
binary built from `tools/commit-msg` and installed beside it in your hooks
folder, never code from the checked-out branch, so rerun the install after a
change to either lands on `main`. A hook installed without its guard refuses
every commit with a message naming the install.

Run what CI runs before opening one. The tests need node: they run the viewer
suite and check the scraper's duplicate, URL and age rules against the
viewer's own code, and fail rather than skip without it:

```bash
npm ci --ignore-scripts
npm run typecheck
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
cargo test --locked --manifest-path tools/commit-msg/Cargo.toml
```

New functionality and bug fixes come with automated tests in `tests/`, run by
`cargo test --locked` in CI. Keep verification fixture-based: use fabricated
catalogue pages and the synthetic demo data the Development section below
describes, never private source lists, fetched catalogues, credentials or
cookies. Read [DESIGN.md](DESIGN.md) and [PRODUCT.md](PRODUCT.md) before
changing `viewer/`.

`tests/repo_policy.rs` pins the CI workflow, the Dependabot configuration,
SECURITY.md and the toolchain file. A change to one of them is a security
change: update its expected text in the same pull request and say why it
changed. The CI contract leaves out which commit each action is pinned to, so
Dependabot's action bumps pass; it still requires a full commit SHA.

## Development

The parser canary runs only on request:

    cargo test -- --ignored       # canary against a saved live sample

The ignored test checks the parsers against a real page saved by hand to
`data/debug/sample-list.html`: fetch a seller's `albums?page=1` there and the
canary says whether the selectors still match live Yupoo markup (the test's
comment has the command). Nothing writes that file for you: a scrape only
dumps pages it *failed* to parse, which is the opposite of a canary. So on a
fresh clone it fails by design rather than passes vacuously.

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
describes catalogue state and the elements owned by `index.html`; a test in
`tests/viewer.rs` checks the declared element IDs and tags. `viewer/browser-globals.d.ts` gives the
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
