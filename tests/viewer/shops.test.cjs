// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";

// app.js is the DOM layer and cannot be required, so this suite lifts
// renderShops out of the shipped source, as title-refresh.test.cjs does.

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const SOURCE = fs.readFileSync(
  path.join(__dirname, "..", "..", "viewer", "app.js"), "utf8");
const { safeHttpUrl } = require("../../viewer/shortlist.js");

function element(tag) {
  return { tagName: tag, children: [], addEventListener() {},
    append(...children) { this.children.push(...children); },
    appendChild(child) { this.children.push(child); },
    replaceChildren(...children) { this.children = children; } };
}

test("shop groups are headed in platform-name order and omit non-HTTP(S) URLs", () => {
  const found = SOURCE.match(/\nfunction renderShops\([\s\S]*?\n\}\n/);
  assert.ok(found, "app.js no longer declares renderShops");
  const panel = element("div");
  const renderShops = new Function("document", "els", "setShopsOpen", "safeHttpUrl",
    found[0] + "\nreturn renderShops;")({ createElement: element }, { shopsPanel: panel }, () => {}, safeHttpUrl);
  renderShops([
    { platform: "taobao global", name: "B", url: "https://b.example" },
    { platform: "taobao", name: "A", url: "http://a.example" },
    { platform: "1688", name: "C", url: "https://c.example" },
    { platform: "1688", name: "BadJS", url: "javascript:alert(1)" },
    { platform: "1688", name: "BadCreds", url: "https://user:pass@shop.example/" },
    { platform: "evil-only", name: "BadData", url: "data:text/html,<h1>x</h1>" },
    { platform: "evil-file", name: "BadFile", url: "file:///etc/passwd" },
    { platform: "evil-space", name: "BadHost", url: "http://shop example/" },
    { platform: "evil-schemeless", name: "NoScheme", url: "weidian.com/?userid=1" },
    // A hand-edited non-string name or platform is shown as text.
    { platform: 7, name: "Numbered", url: "https://p.example" },
    { platform: "web", name: 5, url: "https://n.example", preferred: "yes" },
    // An object, array or empty name shows the shop's address; an object platform files under web.
    { platform: { bad: 1 }, name: { first: "x" }, url: "https://o.example" },
    { platform: "web", name: ["a"], url: "https://q.example" },
    { platform: "web", name: "", url: "https://r.example" },
    null,
    [],
  ]);
  const headings = panel.children.filter(c => c.tagName === "h3").map(c => c.textContent);
  assert.deepEqual(headings, ["1688", "7", "taobao", "taobao global", "web"]);
  const links = panel.children.filter(c => c.tagName === "a");
  assert.deepEqual(links.map(l => [l.textContent, l.href]), [
    ["C", "https://c.example/"],
    ["Numbered", "https://p.example/"],
    ["A", "http://a.example/"],
    ["B", "https://b.example/"],
    ["5", "https://n.example/"],
    ["https://o.example/", "https://o.example/"],
    ["https://q.example/", "https://q.example/"],
    ["https://r.example/", "https://r.example/"],
  ]);
  // Only a boolean true earns the badge.
  assert.equal(links.find(l => l.textContent === "5").children.length, 0);
});
