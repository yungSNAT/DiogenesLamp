// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const css = fs.readFileSync(path.resolve(__dirname, "../../viewer/style.css"), "utf8");

function block(selector) {
  const start = css.indexOf(selector + " {");
  assert.ok(start >= 0, `style.css no longer has ${selector}`);
  return css.slice(start, css.indexOf("}", start));
}

function tokens(selector) {
  return Object.fromEntries([...block(selector).matchAll(/(--[\w-]+):\s*(#[0-9a-f]{6})/g)]
    .map(match => [match[1], match[2]]));
}

function luminance(hex) {
  const [r, g, b] = [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map(c => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function ratio(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

// WCAG 1.4.11: an edge that is the only sign of a control needs 3:1 against
// what it sits on. Fields and mark boxes sit on the leaf and the raised leaf.
test("control edges clear 3:1 on both leaves in both themes", () => {
  const dark = tokens(":root");
  const themes = {dark, light: {...dark, ...tokens(':root[data-theme="light"]')}};
  const edges = {
    "field baseline": [/border-bottom:\s*1px solid var\((--[\w-]+)\)/, 'input[type="search"], select'],
    "mark box": [/border:\s*1px solid var\((--[\w-]+)\)/, ".mark-check"],
    "preview field": [/border:\s*1px solid var\((--[\w-]+)\)/, "#preview-form input, #preview-form textarea"],
  };
  for (const [name, [pattern, selector]] of Object.entries(edges)) {
    const token = block(selector).match(pattern)?.[1];
    assert.ok(token, `${name}: no edge colour`);
    for (const [theme, palette] of Object.entries(themes)) {
      for (const ground of ["--leaf", "--leaf-2"]) {
        const measured = ratio(palette[token], palette[ground]);
        assert.ok(measured >= 3, `${name} ${token} on ${ground} (${theme}): ${measured.toFixed(2)}:1`);
      }
    }
  }
});
