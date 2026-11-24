// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";

const { describe, it } = require("node:test");
const assert = require("node:assert/strict");

const { WORDS } = require("../../viewer/kinds.js");
const {
  materialChipList,
  countMaterials,
  toggleMaterialFilter,
  preferredSellerSet,
  sortSellersForFilter,
  sellerOptionLabel,
} = require("../../viewer/chips.js");
const { defaultFilters, normalizeFilters, sortAlbums, filtersFromURL, filtersToURL } = require("../../viewer/filters.js");
const { collapseDups, newestListings } = require("../../viewer/dedup.js");
const { createViewState, selectCatalogue } = require("../../viewer/view-state.js");
const { validCatalogueIndex } = require("../../viewer/discovery.js");

describe("material filter chips", () => {
  it("derives chips directly from kinds.js material dictionary", () => {
    const chips = materialChipList(WORDS);
    const ids = chips.map((c) => c.id);
    for (const expected of ["cotton", "wool", "cashmere", "leather", "denim", "linen", "silk"]) {
      assert.ok(ids.includes(expected), `missing material chip: ${expected}`);
    }
    assert.equal(chips.find((c) => c.id === "cashmere")?.label, "Cashmere");
    assert.equal(chips.find((c) => c.id === "wool")?.label, "Wool");
  });

  it("counts active albums per material and ignores gone listings", () => {
    const albums = [
      { traits: ["wool", "black"] },
      { traits: ["wool", "cashmere"] },
      { traits: ["cashmere"], gone_since: "2026-11-01" },
      { traits: [] },
      {},
    ];
    const counts = countMaterials(albums, ["wool", "cashmere", "silk"]);
    assert.equal(counts.get("wool"), 2);
    assert.equal(counts.get("cashmere"), 1);
    assert.equal(counts.get("silk"), 0);
  });

  it("toggles material filter on and off cleanly", () => {
    assert.equal(toggleMaterialFilter("", "cashmere"), "cashmere");
    assert.equal(toggleMaterialFilter("cashmere", "wool"), "wool");
    assert.equal(toggleMaterialFilter("wool", "wool"), "");
  });
});

describe("preferred sellers in viewer", () => {
  it("extracts preferred seller set, sorts preferred first and formats option labels", () => {
    const sellers = [
      { id: "zeta", name: "Zeta", album_count: 10 },
      { id: "beta", name: "Beta", album_count: 5, preferred: true },
      { id: "alpha", name: "Alpha", album_count: 20, preferred: false },
      { id: "gamma", name: "Gamma", album_count: 8, preferred: true },
    ];
    const pref = preferredSellerSet(sellers);
    assert.deepEqual([...pref], ["beta", "gamma"]);
    assert.equal(preferredSellerSet(null).size, 0);

    const sorted = sortSellersForFilter(sellers);
    assert.deepEqual(sorted.map((s) => s.id), ["beta", "gamma", "zeta", "alpha"]);
    assert.equal(sellerOptionLabel(sellers[0]), "Zeta (10)");
    assert.equal(sellerOptionLabel(sellers[1]), "Beta [preferred] (5)");
  });

  it("boosts preferred sellers in default sort and breaks ties in explicit sorts", () => {
    const pref = new Set(["trusted"]);
    const albums = [
      { id: "300", seller: "other", published_ms: 3000, price: 200, photo_count: 10 },
      { id: "100", seller: "trusted", published_ms: 1000, price: 200, photo_count: 10 },
      { id: "200", seller: "trusted", published_ms: 2000, price: 300, photo_count: 5 },
    ];
    assert.deepEqual(
      sortAlbums(albums, "default", pref).map((a) => a.id),
      ["200", "100", "300"],
    );
    assert.deepEqual(
      sortAlbums(albums, "price-asc", pref).map((a) => a.id),
      ["100", "300", "200"],
    );
    assert.deepEqual(
      sortAlbums(albums, "price-desc", pref).map((a) => a.id),
      ["200", "100", "300"],
    );
    assert.deepEqual(
      sortAlbums(albums, "photos-desc", pref).map((a) => a.id),
      ["100", "300", "200"],
    );
  });

  it("favours preferred sellers in duplicate clustering (newestListings and collapseDups)", () => {
    const pref = new Set(["trusted"]);
    const olderPref = {
      id: "100",
      seller: "trusted",
      dup_key: "i:weidian:7",
      date_published: "2026-01-01",
      updated_at: "2026-01-02T00:00:00Z",
      links: [{ platform: "weidian", url: "https://weidian.com/item.html?itemID=7" }],
    };
    const newerOther = {
      id: "200",
      seller: "other",
      dup_key: "i:weidian:7",
      date_published: "2026-03-01",
      updated_at: "2026-03-02T00:00:00Z",
      links: [
        { platform: "weidian", url: "https://weidian.com/item.html?itemID=7" },
        { platform: "taobao", url: "https://item.taobao.com/item.htm?id=9" },
      ],
    };
    const newerPref = {
      id: "300",
      seller: "trusted",
      dup_key: "i:weidian:7",
      date_published: "2026-02-01",
      updated_at: "2026-02-02T00:00:00Z",
      links: [{ platform: "weidian", url: "https://weidian.com/item.html?itemID=7" }],
    };

    const selected = newestListings([newerOther, olderPref], pref);
    assert.deepEqual(selected.albums.map((a) => a.id), ["100"]);
    assert.deepEqual(selected.aliases, { "200": "100" });

    // Among two preferred listings, newest posting still wins.
    const bothPref = newestListings([olderPref, newerOther, newerPref], pref);
    assert.deepEqual(bothPref.albums.map((a) => a.id), ["300"]);
    assert.deepEqual(bothPref.aliases, { "100": "300", "200": "300" });

    // collapseDups favours preferred over more links / newer updated_at.
    const collapsed = collapseDups([newerOther, olderPref], new Set(), pref);
    assert.deepEqual(collapsed.out.map((a) => a.id), ["100"]);
    assert.equal(collapsed.counts.get("i:weidian:7"), 2);
  });

  it("filters preferredOnly in selectCatalogue and round-trips through URL/storage", () => {
    const filters = normalizeFilters({ preferredOnly: true, material: "wool" });
    assert.equal(filters.preferredOnly, true);
    assert.equal(defaultFilters().preferredOnly, false);

    const qs = filtersToURL(filters);
    assert.deepEqual(filtersFromURL(qs), filters);

    const albums = [
      { id: "1", seller: "trusted", category: "", first_seen: "2026-01-01", published_ms: 1000, title_lc: "wool coat", traits: ["wool"], links: [] },
      { id: "2", seller: "other", category: "", first_seen: "2026-01-01", published_ms: 2000, title_lc: "wool coat", traits: ["wool"], links: [] },
      { id: "3", seller: "trusted", category: "", first_seen: "2026-01-01", published_ms: 3000, title_lc: "cotton tee", traits: ["cotton"], links: [] },
    ];
    const view = createViewState(filters);
    const result = selectCatalogue(view, {
      albums,
      favorites: new Set(),
      preferredSellers: new Set(["trusted"]),
      bannedKeys: new Set(),
    });
    assert.deepEqual(result.filtered.map((a) => a.id), ["1"]);
  });

  it("validates optional preferred boolean on index.json sellers", () => {
    const base = {
      schema_version: 1,
      generated_at: "2026-11-24T00:00:00Z",
      sellers: [{ id: "trusted", name: "Trusted", album_count: 1, shard: "catalog/trusted.json", preferred: true }],
      shops: [],
    };
    assert.equal(validCatalogueIndex(base), true);
    const bad = {
      ...base,
      sellers: [{ id: "trusted", name: "Trusted", album_count: 1, shard: "catalog/trusted.json", preferred: "yes" }],
    };
    assert.equal(validCatalogueIndex(bad), false);
  });
});
