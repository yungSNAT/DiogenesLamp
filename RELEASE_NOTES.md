# v0.1.2

Fixes and small improvements to `lamp`, the DiogenesLamp command. As before,
there are executables for Linux (x86_64, statically linked) and Windows
(x86_64); on macOS, install from source with Cargo.

- `lamp` with no arguments, as a double-click on `lamp.exe` runs it, prints
  the next steps instead of a usage error.
- `lamp scrape` names each seller's new and returned albums on its result
  line.
- A word of a few hundred dotted parts no longer stops an album's purchase
  link scan: only the rest of that word is skipped, and the album is reported
  as partly scanned.
- `lamp prepare-covers` saves no snapshot for a batch with nothing to prepare,
  and skips covers missing from disk instead of failing on them every run.
- Album pages with tens of thousands of void tags or attributes parse in
  linear time.
- In the viewer, control edges clear 3:1 contrast, focused fields keep the
  focus ring, and loading counts are announced to screen readers once.

Check a download before running it, as
[Download](https://github.com/yungSNAT/DiogenesLamp/blob/v0.1.2/README.md#download)
describes. Everything that changed is in
[CHANGELOG.md](https://github.com/yungSNAT/DiogenesLamp/blob/v0.1.2/CHANGELOG.md).
