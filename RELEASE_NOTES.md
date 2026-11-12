# v0.1.1

The first release of `lamp`, the DiogenesLamp command: one executable with the
viewer built in, so installing needs no Rust toolchain.

- Executables for Linux (x86_64, statically linked) and Windows (x86_64). No
  macOS build yet; on macOS, install from source with Cargo.
- `lamp demo` serves the viewer over synthetic data, so you can see it before
  scraping anything.
- Check a download before running it, as
  [Install](https://github.com/yungSNAT/DiogenesLamp/blob/v0.1.1/README.md#from-a-release)
  describes.

This release replaces the Python package. What changed, including the
documented differences from the Python version, is in
[CHANGELOG.md](https://github.com/yungSNAT/DiogenesLamp/blob/v0.1.1/CHANGELOG.md).

v0.1.0 was tagged but not released, because the release audit refused its
binary for rustix 1.1.5 (RUSTSEC-2026-0486); this release takes rustix 1.1.6.
