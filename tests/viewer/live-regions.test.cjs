// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

// app.js is the DOM layer and cannot be required, so this suite lifts the
// status writers out of it and runs them against recording stand-ins.
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const {readFileSync} = require("node:fs");
const {join} = require("node:path");

const source = readFileSync(join(__dirname, "../../viewer/app.js"), "utf8");
function lift(name) {
  const found = source.match(new RegExp("\\n(?:/\\*\\*[^\\n]*\\*/\\n)?function " + name + "\\([\\s\\S]*?\\n\\}\\n"));
  return found ? found[0] : "";
}

// Records, for each text write, the live setting the region had at that moment.
function region(writes, name) {
  const attrs = {};
  return {
    setAttribute(key, value) { attrs[key] = value; },
    replaceChildren() { writes.push([name, attrs["aria-live"]]); },
    set textContent(_) { writes.push([name, attrs["aria-live"]]); },
  };
}

test("status regions stay silent while shards stream and speak once loaded", () => {
  const writes = [];
  const els = {
    status: region(writes, "status"),
    descriptionCoverage: region(writes, "coverage"),
    progress: {style: {}, parentElement: {}},
    grid: {setAttribute() {}},
    filtersClear: {},
  };
  const state = {index: {sellers: [{id: "one"}, {id: "two"}]}, loadedSellers: 1,
    filtered: [], albums: [{id: "1"}], failedShards: [], dupHidden: 0, view: {groupKey: ""}};
  const writers = new Function("state", "els", "tallyFigure", "tallySeparator", "anyFilterActive",
    "currentFilters", "setGridMessage", "setLoadNote",
    ["catalogueLoading", "setLive", "setStatus", "setDescriptionCoverage"].map(lift).join("") +
    "\nreturn {setStatus, setDescriptionCoverage};"
  )(state, els, () => "", () => "", () => false, () => ({}), () => {}, () => {});

  writers.setDescriptionCoverage();
  writers.setStatus();
  state.loadedSellers = 2;
  writers.setDescriptionCoverage();
  writers.setStatus();
  assert.deepEqual(writes, [
    ["coverage", "off"], ["status", "off"],
    ["coverage", "polite"], ["status", "polite"],
  ]);
});
