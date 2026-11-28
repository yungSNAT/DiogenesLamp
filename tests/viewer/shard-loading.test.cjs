// Copyright 2026 yungSNAT
// SPDX-License-Identifier: Apache-2.0

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const {readFileSync} = require("node:fs");
const {join} = require("node:path");
const {dupKey, registerTitleKeys, newestListings} = require("../../viewer/dedup.js");
const {markNew} = require("../../viewer/catalogue.js");
const {kindOf} = require("../../viewer/kinds.js");
const {prepareAlbumSearch, admittedCatalogueShard, inheritableAliases, unselectedShardAlbums, listingCopies} =
  require("../../viewer/discovery.js");
const {publishedMs} = require("../../viewer/age.js");
const {createViewState, selectCatalogue} = require("../../viewer/view-state.js");
const {defaultFilters} = require("../../viewer/filters.js");

const source = readFileSync(join(__dirname, "../../viewer/app.js"), "utf8");
function lift(name) {
  const found = source.match(new RegExp("\\n(?:async )?function " + name + "\\([\\s\\S]*?\\n\\}\\n"));
  assert.ok(found, "app.js no longer declares " + name);
  return found[0];
}

/** rebuildAlbums from app.js, over `state`. */
function rebuilder(state) {
  return new Function("state", "newestListings", "inheritableAliases", "listingCopies",
    lift("rebuildAlbums") + "\nreturn rebuildAlbums;")(state, newestListings, inheritableAliases, listingCopies);
}

test("newest selection precedes seller and favourite filters in a legacy catalogue", () => {
  const old = {id: "100", seller: "one", title: "Old posting", title_lc: "old posting",
    date_published: "2026-01-01", links: [{url: "https://weidian.com/item.html?itemID=7"}]};
  const latest = {...old, id: "200", seller: "two", date_published: "2026-02-01"};
  for (const a of [old, latest]) { a.dup_key = dupKey(a); a.published_ms = publishedMs(a); }
  const state = {index: {sellers: [{id: "one"}, {id: "two"}]},
    loadedSellers: 2, failedShards: [],
    shardAlbums: new Map([["one", [old]], ["two", [latest]]]), unselectedAlbums: new Map()};
  const rebuild = rebuilder(state);
  rebuild();
  assert.deepEqual(state.albums, [latest]);
  assert.deepEqual(state.albumAliases, {100: "200"});
  assert.deepEqual(state.listingCopies.get("200"), [old]);
  for (const filters of [{seller: "one"}, {favsOnly: true}]) {
    const view = createViewState({...defaultFilters(), ...filters});
    assert.deepEqual(selectCatalogue(view, {albums: state.albums,
      favorites: new Set(["100"]), bannedKeys: new Set()}).filtered, []);
  }
  // With a seller unread, saved work is not moved between listings, but the
  // grid still hides the older listing, so the preview still lists it.
  for (const partial of [{loadedSellers: 3, failedShards: ["three"]}, {loadedSellers: 2}]) {
    Object.assign(state, {index: {sellers: [{id: "one"}, {id: "two"}, {id: "three"}]}, failedShards: []}, partial);
    rebuild();
    assert.deepEqual(state.albums, [latest]);
    assert.deepEqual(state.albumAliases, {});
    assert.deepEqual(state.listingCopies.get("200"), [old], JSON.stringify(partial));
  }
});

test("indexed shards publish only selected records and report missing selections", async () => {
  const sellers = [{id: "one", shard: "catalog/one.json", album_ids: ["200"]},
    {id: "missing", shard: "catalog/missing.json", album_ids: ["300"]}];
  const record = id => ({id, seller: "one", title: "Blue shirt", links: [], thumb: "",
    album_url: `https://one.x.yupoo.com/albums/${id}`});
  const state = {pendingShards: [...sellers], generatedAt: "2026-10-01",
    titleKeyCounts: new Map(), bannedKeys: new Set(), categories: new Set(),
    shardAlbums: new Map(), unselectedAlbums: new Map(), failedShards: [], loadedSellers: 0};
  const prepared = [];
  const worker = new Function("state", "els", "fetch", "admittedCatalogueShard", "unselectedShardAlbums", "dupKey", "kindOf",
    "prepareAlbumSearch", "publishedMs", "markNew", "registerTitleKeys", "scheduleRefresh", "console",
    lift("nextShard") + lift("shardWorker") + "\nreturn shardWorker;"
  )(state, {seller: {value: ""}}, async url => ({ok: true,
    json: async () => ({albums: url.includes("missing") ? [] : [record("100"), record("200")]})}),
    admittedCatalogueShard, unselectedShardAlbums, dupKey, kindOf, a => {prepared.push(a.id); prepareAlbumSearch(a);},
    publishedMs, markNew, registerTitleKeys, () => {}, {warn() {}});
  await worker();
  assert.deepEqual(prepared, ["200"]);
  assert.deepEqual(state.shardAlbums.get("one").map(a => a.id), ["200"]);
  assert.equal(state.shardAlbums.has("missing"), false);
  assert.deepEqual(state.failedShards, ["missing"]);
  assert.deepEqual(state.unselectedAlbums.get("one").map(a => a.id), ["100"]);
});

test("an indexed shard keeps the albums it did not select, each checked as a selected one is", () => {
  const record = (id, extra = {}) => ({id, seller: "one", title: "Blue shirt", links: [], thumb: "",
    album_url: `https://one.x.yupoo.com/albums/${id}`, ...extra});
  const seller = {id: "one", shard: "catalog/one.json", album_ids: ["200"]};
  const shard = {albums: [record("100", {price: 90}), record("200"),
    record("300", {album_url: "http://one.x.yupoo.com/albums/300"}), record("400", {seller: "two"}),
    record("500", {price: -1}), null]};
  assert.deepEqual(unselectedShardAlbums(shard, seller).map(a => a.id), ["100"]);
  // Only what the preview's list shows is kept, not descriptions or history.
  const full = record("600", {price: 80, date_published: "2026-10-01", first_seen: "2026-10-02T09:00:00+11:00",
    gone_since: "2026-11-01T09:00:00+11:00", description: "long", history: [{at: "x"}]});
  assert.deepEqual(unselectedShardAlbums({albums: [full]}, seller), [{id: "600", seller: "one", price: 80,
    date_published: "2026-10-01", first_seen: "2026-10-02T09:00:00+11:00", gone_since: "2026-11-01T09:00:00+11:00",
    album_url: "https://one.x.yupoo.com/albums/600"}]);
  assert.deepEqual(unselectedShardAlbums(shard, {id: "one", shard: "catalog/one.json"}), []);
  assert.deepEqual(unselectedShardAlbums(null, seller), []);
});

test("a selected listing's other listings come from the index's aliases", () => {
  const record = (id, seller) => ({id, seller, title: "Blue shirt", links: [], thumb: "",
    album_url: `https://${seller}.x.yupoo.com/albums/${id}`});
  const shown = record("300", "three"), older = record("100", "one"), other = record("200", "two");
  const state = {index: {selection_version: 1, album_aliases: {100: "300", 200: "300", 999: "300"},
    sellers: [{id: "one"}, {id: "two"}, {id: "three"}]}, loadedSellers: 3, failedShards: [],
    shardAlbums: new Map([["one", []], ["two", []], ["three", [shown]]]),
    unselectedAlbums: new Map([["one", [older]], ["two", [other]]])};
  rebuilder(state)();
  assert.deepEqual(state.albums, [shown]);
  assert.deepEqual([...state.listingCopies], [["300", [older, other]]]);
});

// Every index shape `lamp scrape` writes (src/storage.rs write_index): a
// complete one (selection_version 1, album_ids and album_aliases), one written
// whilst a seller's shard was missing (album_ids and album_aliases for the
// sellers it read, none for that seller, no selection_version), and one from
// before album_ids. With every shard loaded, or with one failing here, each
// hidden listing is either listed under the one shown or named, and so
// counted, by state.listingAliases.
test("the preview lists or counts the hidden listings of every index shape", () => {
  const record = (id, seller, date) => {
    const album = {id, seller, title: "Blue shirt", title_lc: "blue shirt", thumb: "", price: 90,
      links: [{url: "https://weidian.com/item.html?itemID=7"}], date_published: date,
      album_url: `https://${seller}.x.yupoo.com/albums/${id}`};
    album.dup_key = dupKey(album); album.published_ms = publishedMs(album);
    return album;
  };
  const older = record("100", "one", "2026-01-01"), shown = record("200", "two", "2026-02-01");
  const three = {id: "three", shard: "catalog/three.json"};
  const shapes = {
    complete: {selection_version: 1, album_aliases: {100: "200"},
      sellers: [{id: "one", album_ids: []}, {id: "two", album_ids: ["200"]}, {...three, album_ids: []}]},
    "seller missing": {album_aliases: {100: "200"},
      sellers: [{id: "one", album_ids: []}, {id: "two", album_ids: ["200"]}, three]},
    "before album_ids": {sellers: [{id: "one"}, {id: "two"}, three]},
  };
  const cases = [];
  for (const [shape, index] of Object.entries(shapes)) {
    for (const oneFails of [false, true]) cases.push({shape, index, oneFails});
  }
  // Every case's list first, then the names that count the rest.
  for (const check of ["listed", "named"]) {
    for (const {shape, index, oneFails} of cases) {
      const indexed = index.sellers[0].album_ids !== undefined;
      const shardAlbums = new Map([["two", [shown]], ["three", []]]);
      const unselectedAlbums = new Map();
      if (!oneFails) {
        if (indexed) unselectedAlbums.set("one", [older]);
        shardAlbums.set("one", indexed ? [] : [older]);
      }
      const state = {index, loadedSellers: 3, failedShards: oneFails ? ["one"] : [], shardAlbums, unselectedAlbums};
      rebuilder(state)();
      const label = `${shape}, seller one ${oneFails ? "failed" : "loaded"}`;
      assert.deepEqual(state.albums, [shown], label);
      // Nothing names a listing in a seller never read; the heading then
      // says the count covers the sellers read (preview.test.cjs).
      const known = !(oneFails && !indexed);
      if (check === "listed") {
        assert.deepEqual((state.listingCopies.get("200") || []).map(a => a.id), known && !oneFails ? ["100"] : [], label);
      } else {
        const named = Object.keys(state.listingAliases).filter(id => state.listingAliases[id] === "200");
        assert.deepEqual(named, known ? ["100"] : [], label);
      }
    }
  }
});

test("rebuilding a large seller keeps index order without an argument limit", () => {
  const first = Array.from({length: 200000}, (_, index) => ({id: String(index)}));
  const last = [{id: "last"}];
  const state = {albums: [{id: "old"}], index: {sellers: [
    {id: "first"}, {id: "missing"}, {id: "last"},
  ]}, shardAlbums: new Map([["last", last], ["first", first]]), unselectedAlbums: new Map()};
  const rebuild = rebuilder(state);
  rebuild();
  assert.equal(state.albums.length, 200001);
  assert.deepEqual(state.albums.slice(0, first.length), first);
  assert.equal(state.albums[0], first[0]);
  assert.equal(state.albums[first.length - 1], first.at(-1));
  assert.equal(state.albums.at(-1), last[0]);
  assert.equal(first.length, 200000);
  assert.deepEqual(last, [{id: "last"}]);
  state.shardAlbums.delete("first");
  rebuild();
  assert.deepEqual(state.albums, last);
});

test("large shards yield to input before publishing, then honour seller priority", async () => {
  const sellers = ["first", "second", "third"].map(id => ({id, shard: `catalog/${id}.json`}));
  const albums = seller => Array.from({length: seller === "first" ? 1200 : 1}, (_, index) => ({
    id: String(index + 1), seller, title: `blue shirt ${index + 1}`, category: "shirts",
    links: [], album_url: `https://${seller}.x.yupoo.com/albums/${index + 1}`, thumb: "",
    date_published: "2026-10-01", first_seen: "2026-10-01T00:00:00Z",
  }));
  const shards = new Map(sellers.map(seller => [seller.id, {albums: albums(seller.id)}]));
  const state = {pendingShards: [...sellers], generatedAt: "2026-10-01T00:00:00Z",
    titleKeyCounts: new Map(), bannedKeys: new Set(), categories: new Set(),
    shardAlbums: new Map(), unselectedAlbums: new Map(), failedShards: [], loadedSellers: 0};
  const els = {seller: {value: ""}};
  const requests = [];
  let prepared = 0, checkpoint, publishedAtInput, refreshes = 0;
  const worker = new Function("state", "els", "fetch", "admittedCatalogueShard", "unselectedShardAlbums", "dupKey", "kindOf",
    "prepareAlbumSearch", "publishedMs", "markNew", "registerTitleKeys", "scheduleRefresh",
    lift("nextShard") + lift("shardWorker") + "\nreturn shardWorker;"
  )(state, els, async url => {
    const id = /catalog\/(.+)\.json$/.exec(url)[1];
    requests.push(id);
    return {ok: true, json: async () => shards.get(id)};
  }, admittedCatalogueShard, unselectedShardAlbums, dupKey, kindOf, album => {
    prepared += 1;
    prepareAlbumSearch(album);
  }, publishedMs, markNew, registerTitleKeys, () => { refreshes += 1; });
  const input = new Promise(resolve => setTimeout(() => {
    checkpoint = prepared;
    publishedAtInput = state.shardAlbums.has("first");
    els.seller.value = "third";
    resolve();
  }, 0));
  await worker();
  await input;
  assert.equal(publishedAtInput, false);
  assert.ok(checkpoint > 0 && checkpoint < 1200, `input ran after ${checkpoint} albums`);
  assert.deepEqual(requests, ["first", "third", "second"]);
  assert.equal(prepared, 1202);
  assert.equal(state.loadedSellers, 3);
  assert.equal(refreshes, 3);
  assert.deepEqual(state.failedShards, []);
  for (const seller of sellers) {
    assert.equal(state.shardAlbums.get(seller.id), shards.get(seller.id).albums);
    assert.ok(state.shardAlbums.get(seller.id).every(album =>
      album.search_keys && album.published_ms && album.kind));
  }
});
