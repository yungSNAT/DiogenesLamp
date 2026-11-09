// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Ported from tests/test_workflow_security.py: fail-closed contracts for
//! the security-bearing policy files. A change here must be reviewed for its
//! security effect and the expected text updated explicitly.
mod common;

use sha2::{Digest, Sha256};

const EXPECTED_CI: &str = r#"name: ci
on:
  push:
    branches: [main]
  pull_request:
  workflow_dispatch:

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

permissions:
  contents: read

jobs:
  test:
    timeout-minutes: 30
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@<pin>
        with:
          persist-credentials: false
      # The Rust tests run the viewer's node suite and the browser parity
      # checks, and fail rather than skip without node, so it is installed
      # explicitly. The separate type check does not execute tests.
      - uses: actions/setup-node@<pin>
        with:
          node-version: "22"
          package-manager-cache: false
      - run: npm ci --ignore-scripts
      - run: npm run typecheck
      # Only main saves, so pull requests cannot replace the shared entries.
      # 1 crate on 2 systems stays well inside the 10 GB cache limit.
      - uses: Swatinem/rust-cache@<pin>
        with:
          save-if: ${{ github.ref == 'refs/heads/main' }}
      # rust-toolchain.toml pins the compiler and its clippy and rustfmt components.
      - run: cargo fmt --check
      - run: cargo clippy --locked --all-targets -- -D warnings
      # fixture-transport is linted but not tested: it stops RateLimiter::wait
      # from sleeping, which tests/http.rs asserts.
      - run: cargo clippy --locked --all-targets --all-features -- -D warnings
      - run: cargo test --locked
        if: matrix.os != 'ubuntu-latest'
      # Coverage feeds the Codecov badge only and has no floor. It needs
      # llvm-tools and a prebuilt cargo-llvm-cov, with no fallback to compiling it.
      - run: rustup component add llvm-tools
        if: matrix.os == 'ubuntu-latest'
      - uses: taiki-e/install-action@<pin>
        if: matrix.os == 'ubuntu-latest'
        with:
          tool: cargo-llvm-cov@0.9.1
          fallback: none
      - run: cargo llvm-cov --locked --lcov --output-path coverage-rust.lcov
        if: matrix.os == 'ubuntu-latest'
      - uses: actions/upload-artifact@<pin>
        if: matrix.os == 'ubuntu-latest'
        with:
          name: coverage-rust
          path: coverage-rust.lcov
          retention-days: 1
      - run: cargo build --locked --release

  # The upload runs apart from the tests so the OIDC token is never available
  # while test code runs. A Codecov outage fails only this job.
  codecov:
    name: upload coverage to Codecov
    needs: test
    if: github.event_name == 'push' || github.event.pull_request.head.repo.full_name == github.repository
    runs-on: ubuntu-latest
    timeout-minutes: 10
    continue-on-error: true
    permissions:
      contents: read
      id-token: write # Authenticate the Codecov upload with OIDC; no stored token.
    steps:
      - uses: actions/checkout@<pin>
        with:
          persist-credentials: false
      - uses: actions/download-artifact@<pin>
        with:
          name: coverage-rust
      - uses: codecov/codecov-action@<pin>
        with:
          use_oidc: true
          files: coverage-rust.lcov
          disable_search: true
          fail_ci_if_error: true
"#;

const EXPECTED_RELEASE: &str = r#"name: Release

# A v* tag on main's head publishes lamp through release-policy's binary
# family: Linux (x86_64, static) and Windows (x86_64) executables, source
# archives, SBOMs, SHA256SUMS and signed provenance in one immutable release.
# The family builds no macOS target yet. VERSION, Cargo.toml and the first
# line of RELEASE_NOTES.md must agree (tests/repo_policy.rs); release-policy
# refuses a tag that names another version.

on:
  push:
    tags:
      - "v*"

permissions:
  contents: read

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: false

jobs:
  release:
    # A job that calls a reusable workflow accepts no timeout-minutes; every
    # job in release-binary.yml sets its own.
    permissions:
      actions: read # Read this repository's own runs to require the mandatory checks.
      attestations: write # Record provenance and SBOM attestations for release assets.
      contents: write # Create the draft release, upload assets and publish it.
      id-token: write # Obtain the OIDC identity required to sign attestations.
    uses: ryanduguid/release-policy/.github/workflows/release-binary.yml@ca5492bd09693be9e1fd47d980e2740ef8daf023 # canaries.json current pin (#273)
    with:
      artifact-stem: diogenes-lamp
      binary: lamp
      # The job contexts branch protection requires on main.
      required-checks: |
        .github/workflows/ci.yml: test (ubuntu-latest)
        .github/workflows/ci.yml: test (windows-latest)
"#;

const EXPECTED_DEPENDABOT: &str = r#"version: 2
updates:
  - package-ecosystem: npm
    directory: /
    schedule:
      interval: weekly
    cooldown:
      default-days: 7
  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: weekly
    cooldown:
      default-days: 7
    groups:
      # CodeQL refuses a workflow whose init, analyze and upload-sarif steps differ in version.
      codeql-action:
        patterns:
          - "github/codeql-action*"
    ignore:
      # The release-policy reusable workflows are pinned in
      # tests/repo_policy.rs, so a bot bump can only fail CI.
      # Bump them by hand and move the pins in the same commit.
      - dependency-name: ryanduguid/release-policy/.github/workflows/attribution-policy.yml
      - dependency-name: ryanduguid/release-policy/.github/workflows/release-binary.yml
  - package-ecosystem: cargo
    directory: /
    schedule:
      interval: weekly
    cooldown:
      default-days: 7
    groups:
      rust-dependencies:
        patterns:
          - "*"
"#;

const EXPECTED_SECURITY: &str = r#"# Security policy

## Supported versions

Security fixes are applied to the latest version on the default branch.

## Reporting a vulnerability

Report it privately through GitHub's private vulnerability reporting:
https://github.com/yungSNAT/DiogenesLamp/security/advisories/new. Do not open
an issue for a suspected security vulnerability. Include a clear description,
reproduction steps, impact, and any suggested mitigation.

Use fabricated catalogue pages, source configuration and reproduction data only. Do
not submit client or private catalogue data, credentials, access tokens, cookies,
secrets or `.env` content.

## What this tool does and does not do

DiogenesLamp fetches public Yupoo catalogue pages, writes JSON and thumbnails
to a local directory, and serves a local viewer. It does not log in, hold
OAuth tokens, or write back to Yupoo.

Treat scraped titles, HTML and JSON as untrusted data in the viewer. Do not
run the importer with elevated privileges, and do not point it at a less
trusted user's working directory.

The scraper talks to the network. `lamp import` reads a local link dump and
writes `sources.toml` without fetching anything.

Keep `sources.toml` free of credentials. The expected input is a dump of public
http or https links: Yupoo catalogue URLs, which are the only ones scraped, plus
non-Yupoo shop links (taobao, tmall, weidian) and link aggregators, which are
kept as a link-out directory and never fetched.
"#;

const EXPECTED_TOOLCHAIN: &str = r#"[toolchain]
channel = "1.99.1"
components = ["clippy", "rustfmt"]
profile = "minimal"
"#;

/// Pin the reviewed attribution workflow without duplicating its script.
const ATTRIBUTION_SHA256: &str = "744eae4e533e209bef890fb253491a7a5fe540fb1cf97a47d9b7dac0455d335b";

fn read(relative: &str) -> String {
    common::read(&common::root().join(relative))
}

fn assert_exact(actual: &str, expected: &str, label: &str) {
    assert!(
        actual == expected,
        "{label} no longer matches its reviewed fail-closed contract; review the security effect and update this test explicitly"
    );
}

#[test]
fn active_workflow_inventory_is_exact() {
    let mut names: Vec<String> = std::fs::read_dir(common::root().join(".github/workflows"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".yml") || n.ends_with(".yaml"))
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "ci.yml",
            "codeql.yml",
            "no-ai-attribution.yml",
            "osv-scanner.yml",
            "release.yml",
            "scorecard.yml"
        ]
    );
}

/// Dependabot moves action pins, so the CI contract fixes each action, its
/// inputs and every other line, but not the commit an action is pinned to.
#[test]
fn ci_workflow_is_exact() {
    let actual = without_action_pins(&read(".github/workflows/ci.yml")).unwrap_or_else(|line| {
        panic!("ci.yml: `{line}` must pin a full commit SHA and name its version")
    });
    assert_exact(&actual, EXPECTED_CI, "ci.yml");
}

/// Replaces `uses: <action>@<40-hex SHA> # <version>` with `uses: <action>@<pin>`,
/// or returns the first `uses:` line that is not pinned that way.
fn without_action_pins(workflow: &str) -> Result<String, String> {
    let mut out = String::with_capacity(workflow.len());
    for line in workflow.split_inclusive('\n') {
        let step = line.trim_start().trim_start_matches("- ");
        if !step.starts_with("uses: ") || step.starts_with("uses: ./") {
            out.push_str(line);
            continue;
        }
        let ending = if line.ends_with('\n') { "\n" } else { "" };
        let (action, pin) = line
            .trim_end()
            .split_once('@')
            .ok_or_else(|| line.trim().to_string())?;
        let (sha, version) = pin.split_once(" # ").unwrap_or((pin, ""));
        if sha.len() != 40
            || !sha.bytes().all(|b| b.is_ascii_hexdigit())
            || version.trim().is_empty()
        {
            return Err(line.trim().to_string());
        }
        out.push_str(action);
        out.push_str("@<pin>");
        out.push_str(ending);
    }
    Ok(out)
}

/// A Dependabot bump of an action passes; a tag pin, another action or a
/// changed input still fails.
#[test]
fn ci_contract_ignores_only_the_pin() {
    let ci = read(".github/workflows/ci.yml");
    let checkout = ci
        .lines()
        .find(|l| l.contains("uses: actions/checkout@"))
        .unwrap();
    let with = |line: &str| ci.replace(checkout, line);

    let bumped =
        with("      - uses: actions/checkout@0123456789abcdef0123456789abcdef01234567 # v9.0.0");
    assert_eq!(without_action_pins(&bumped).unwrap(), EXPECTED_CI);

    for unpinned in [
        "      - uses: actions/checkout@v7",
        "      - uses: actions/checkout@0123456789abcdef0123456789abcdef01234567",
        "      - uses: actions/checkout@0123456 # v7.0.1",
        "      - uses: actions/checkout",
    ] {
        assert!(without_action_pins(&with(unpinned)).is_err(), "{unpinned}");
    }

    let other =
        with("      - uses: someone/checkout@0123456789abcdef0123456789abcdef01234567 # v7.0.1");
    assert_ne!(without_action_pins(&other).unwrap(), EXPECTED_CI);
    let input = ci.replacen("persist-credentials: false", "persist-credentials: true", 1);
    assert_ne!(without_action_pins(&input).unwrap(), EXPECTED_CI);
}

/// Release-time permissions, the release-policy pin and the checks the release
/// requires on main.
#[test]
fn release_workflow_is_exact() {
    assert_exact(
        &read(".github/workflows/release.yml"),
        EXPECTED_RELEASE,
        "release.yml",
    );
}

/// The release refuses a tag unless these agree with it, so check them before
/// one is pushed.
#[test]
fn release_version_files_agree() {
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(read("VERSION"), format!("{version}\n"), "VERSION");
    assert_eq!(
        read("RELEASE_NOTES.md").lines().next(),
        Some(format!("# v{version}").as_str()),
        "RELEASE_NOTES.md must start with the tag's heading"
    );
}

#[test]
fn attribution_workflow_is_pinned() {
    let digest = Sha256::digest(read(".github/workflows/no-ai-attribution.yml").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_exact(&hex, ATTRIBUTION_SHA256, "no-ai-attribution.yml");
}

#[test]
fn dependabot_document_is_exact() {
    assert_exact(
        &read(".github/dependabot.yml"),
        EXPECTED_DEPENDABOT,
        ".github/dependabot.yml",
    );
}

#[test]
fn security_document_is_exact() {
    assert_exact(&read("SECURITY.md"), EXPECTED_SECURITY, "SECURITY.md");
}

#[test]
fn toolchain_is_pinned() {
    assert_exact(
        &read("rust-toolchain.toml"),
        EXPECTED_TOOLCHAIN,
        "rust-toolchain.toml",
    );
}

/// Releases are built from the committed lock file only.
#[test]
fn cargo_lock_is_tracked() {
    let listed = std::process::Command::new("git")
        .args([
            "-C",
            &common::root().to_string_lossy(),
            "ls-files",
            "--error-unmatch",
            "Cargo.lock",
        ])
        .output()
        .unwrap();
    assert!(listed.status.success(), "Cargo.lock must be committed");
}
