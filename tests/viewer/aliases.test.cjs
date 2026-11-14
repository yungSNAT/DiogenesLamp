// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";

// The operator's data/aliases.json. Names here are made up: the repository
// ships no brand list.
const test = require("node:test");
const assert = require("node:assert/strict");
const {readFileSync} = require("node:fs");
const {join} = require("node:path");
const {readWords, searchTerms, wordRanges, setOperatorAliases} = require("../../viewer/kinds.js");
const {compileCatalogueSearch} = require("../../viewer/discovery.js");
const {matchRanges} = require("../../viewer/catalogue.js");

const read = title => readWords(title.normalize("NFKC").toLowerCase());
const reason = (query, title) => compileCatalogueSearch(query).reason({id: "1", title});
const FILE = {version: 1, aliases: [
  {en: "Harbour Works", tokens: ["港湾", "HW"]},
  {en: "north peak", tokens: ["北峰"], forms: ["northpeak"]},
]};

test("with no aliases a title reads and searches as before", () => {
  assert.deepEqual(setOperatorAliases({version: 1, aliases: []}), {loaded: 0, skipped: 0});
  assert.deepEqual(read("港湾卫衣 HW"), {gloss: ["sweatshirt"], traits: []});
  assert.deepEqual(searchTerms("harbour works").map(term => term.words), [[], []]);
  assert.equal(reason("harbour works", "港湾卫衣"), null);
});

test("an alias finds its Chinese tokens and whole-word seller codes, and glosses first", () => {
  assert.deepEqual(setOperatorAliases(FILE), {loaded: 2, skipped: 0});
  assert.deepEqual(read("黑色卫衣 港湾"), {gloss: ["harbour works", "sweatshirt", "black"], traits: ["black"]});
  assert.deepEqual(read("HW hoodie"), {gloss: ["harbour works"], traits: []});
  // A code is a whole Latin word: not inside another word or joined by a hyphen.
  for (const title of ["HWY hoodie", "SHW tee", "HW-01 tee", "https://shop.example.com/hw/1"]) {
    assert.deepEqual(read(title).gloss, [], title);
  }
  assert.equal(reason("harbour works", "港湾卫衣"), "Title");
  assert.equal(reason("harbour works hoodie", "港湾卫衣"), "Title");
  assert.equal(reason("hw", "港湾卫衣"), "Title", "a typed code finds the Chinese name");
  assert.equal(reason("northpeak", "北峰 jacket"), "Title", "a form finds the Chinese name");
  assert.equal(reason("north peak", "HW tee"), null);
  assert.deepEqual(wordRanges("hw 港湾", new Set(["harbour works"])).sort((a, b) => a[0] - b[0]), [[0, 2], [3, 5]]);
  assert.deepEqual(matchRanges("港湾 tee", "harbour works"), [[0, 2]]);
});

test("a file of the wrong shape is refused whole and leaves the aliases as they were", () => {
  setOperatorAliases(FILE);
  for (const file of [null, [], "x", {aliases: []}, {version: 2, aliases: []}, {version: "1", aliases: []},
    {version: 1}, {version: 1, aliases: {}}]) {
    assert.throws(() => setOperatorAliases(file), TypeError, JSON.stringify(file));
  }
  assert.deepEqual(read("港湾").gloss, ["harbour works"]);
});

test("each malformed or repeated record is skipped and counted, and the rest load", () => {
  const result = setOperatorAliases({version: 1, aliases: [
    {en: "North Peak", tokens: ["北峰"]},
    "north peak", null, [], {tokens: ["港湾"]}, {en: " ", tokens: ["港湾"]}, {en: "x", tokens: []},
    {en: "x", tokens: "港湾"}, {en: "x", tokens: ["港湾", 7]}, {en: "x", tokens: ["港湾", " "]},
    {en: "x", tokens: ["港湾"], forms: "hw"}, {en: "x", tokens: ["港湾"], forms: [null]},
    {en: "NORTH  peak", tokens: ["港湾"]},
  ]});
  assert.deepEqual(result, {loaded: 1, skipped: 12});
  assert.deepEqual(read("北峰 港湾").gloss, ["north peak"]);
});

const source = readFileSync(join(__dirname, "../../viewer/app.js"), "utf8");
function lift(name) {
  const found = source.match(new RegExp("\\n(?:async )?function " + name + "\\([\\s\\S]*?\\n\\}\\n"));
  assert.ok(found, "app.js no longer declares " + name);
  return found[0];
}

/** Runs app.js's loader against a stub fetch; returns the note and what was set. */
async function load(respond, waitMs = 1000) {
  const state = {aliasNote: ""};
  const set = [];
  const requests = [];
  const loader = new Function("state", "fetch", "setOperatorAliases", "console", "ALIAS_WAIT_MS",
    lift("loadOperatorAliases") + "\nreturn loadOperatorAliases;")(state, async (url, init) => {
    requests.push(url);
    return respond(init.signal);
  }, file => { set.push(file); return setOperatorAliases(file); }, {error() {}}, waitMs);
  await loader();
  return {note: state.aliasNote, set, requests};
}
const json = body => ({status: 200, ok: true, json: async () => body});

test("the viewer reads data/aliases.json once, and a missing file is no failure", async () => {
  assert.deepEqual(await load(() => ({status: 404, ok: false})), {note: "", set: [], requests: ["data/aliases.json"]});
  const loaded = await load(() => json(FILE));
  assert.equal(loaded.note, "");
  assert.deepEqual(loaded.set, [FILE]);
});

test("every other failure is named in the load note and loading goes on", async () => {
  assert.equal((await load(() => ({status: 500, ok: false}))).note, "data/aliases.json was not used: HTTP 500.");
  assert.match((await load(() => ({status: 200, ok: true, json: async () => JSON.parse("{")}))).note,
    /^data\/aliases\.json was not used: .*JSON/);
  assert.equal((await load(() => json({version: 2, aliases: []}))).note,
    'data/aliases.json was not used: not {"version": 1, "aliases": [...]}.');
  assert.equal((await load(() => json({version: 1, aliases: [FILE.aliases[0], {en: "x"}]}))).note,
    "data/aliases.json: 1 entry was skipped for a missing or malformed field or a repeated name.");
  // A server that never answers is given up on, not waited for.
  const stalled = await load(signal => new Promise((_, reject) =>
    signal.addEventListener("abort", () => reject(signal.reason))), 20);
  assert.match(stalled.note, /^data\/aliases\.json was not used: /);
});

test("the load note carries an alias problem alone or after failed sellers", () => {
  const note = (state, loading = false) => {
    const els = {note: {hidden: true}, noteText: {textContent: ""}};
    new Function("state", "els", "noteDismissed", lift("setLoadNote") + "\nreturn setLoadNote;")(state, els, false)(loading);
    return els.note.hidden ? null : els.noteText.textContent;
  };
  assert.equal(note({failedShards: [], aliasNote: ""}), null);
  assert.equal(note({failedShards: [], aliasNote: "A."}), "A.");
  assert.equal(note({failedShards: [], aliasNote: "A."}, true), null);
  assert.equal(note({failedShards: ["one"], aliasNote: "A."}),
    "1 seller's data is missing or unreadable and was left out: one. A.");
  assert.equal(note({failedShards: ["one", "two"], aliasNote: ""}),
    "2 sellers' data is missing or unreadable and were left out: one, two.");
});
