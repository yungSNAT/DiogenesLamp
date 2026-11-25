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

  it("caches material chip counts per state.albums reference across filter changes", () => {
    const fs = require("node:fs");
    const path = require("node:path");
    const vm = require("node:vm");
    const appSource = fs.readFileSync(path.join(__dirname, "../../viewer/app.js"), "utf8");
    const block = appSource.match(/let cachedMaterialAlbums = null;[\s\S]*?\nfunction syncMaterialChips\(\) \{[\s\S]*?\n\}/);
    assert.ok(block, "app.js no longer declares cachedMaterialAlbums and syncMaterialChips");
    let scans = 0;
    const chipButtons = new Map([
      ["wool", {
        attrs: {},
        countEl: { textContent: "", hidden: true },
        setAttribute(k, v) { this.attrs[k] = v; },
        querySelector() { return this.countEl; },
      }],
    ]);
    const firstAlbums = [{ traits: ["wool"] }, { traits: ["wool"] }];
    const context = {
      MATERIAL_CHIPS: [{ id: "wool", label: "Wool" }],
      MATERIAL_CHIP_IDS: ["wool"],
      materialChipButtons: chipButtons,
      state: { albums: firstAlbums, view: { filters: { material: "" } } },
      countMaterials(albums, ids) {
        scans += 1;
        return countMaterials(albums, ids);
      },
    };
    vm.createContext(context);
    vm.runInContext(block[0], context);
    context.syncMaterialChips();
    assert.equal(scans, 1);
    assert.equal(chipButtons.get("wool").countEl.textContent, "2");
    assert.equal(chipButtons.get("wool").attrs["aria-pressed"], "false");

    // Repeated filter changes reuse cached counts without rescanning state.albums.
    context.state.view.filters.material = "wool";
    context.syncMaterialChips();
    context.state.view.filters.material = "";
    context.syncMaterialChips();
    assert.equal(scans, 1);

    // Replacing state.albums (shard rebuild) recomputes counts once.
    context.state.albums = [{ traits: ["wool"] }];
    context.syncMaterialChips();
    assert.equal(scans, 2);
    assert.equal(chipButtons.get("wool").countEl.textContent, "1");
  });

  it("isolates seller name truncation from the PREF badge in card markup and CSS", () => {
    const fs = require("node:fs");
    const path = require("node:path");
    const appSource = fs.readFileSync(path.join(__dirname, "../../viewer/app.js"), "utf8");
    const cssSource = fs.readFileSync(path.join(__dirname, "../../viewer/style.css"), "utf8");
    assert.match(appSource, /<span class="seller-tag"><span class="seller-name">\$\{escapeHtml\(album\.seller\)\}<\/span>/);
    assert.match(cssSource, /\.seller-name\s*\{[^}]*text-overflow:\s*ellipsis/);
    assert.match(cssSource, /\.seller-tag\s+\.badge-preferred\s*\{[^}]*flex:\s*none/);
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

  it("favours live listings before preferred sellers in duplicate clustering", () => {
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

    // Among two live preferred listings, newest posting wins.
    const bothPref = newestListings([olderPref, newerOther, newerPref], pref);
    assert.deepEqual(bothPref.albums.map((a) => a.id), ["300"]);
    assert.deepEqual(bothPref.aliases, { "100": "300", "200": "300" });

    // A gone preferred listing must not hide a live non-preferred duplicate (in either order).
    const gonePref = {
      ...newerPref,
      id: "400",
      date_published: "2026-04-01",
      gone_since: "2026-11-01",
      traits: ["cashmere"],
    };
    const liveOther = {
      ...newerOther,
      id: "200",
      date_published: "2026-01-15",
      traits: ["cashmere"],
    };
    assert.deepEqual(newestListings([gonePref, liveOther], pref).albums.map((a) => a.id), ["200"]);
    assert.deepEqual(newestListings([liveOther, gonePref], pref).albums.map((a) => a.id), ["200"]);
    assert.equal(
      countMaterials(newestListings([gonePref, liveOther], pref).albums, ["cashmere"]).get("cashmere"),
      1,
    );

    // When every listing in the cluster is gone, preferred still wins.
    const goneOther = { ...liveOther, gone_since: "2026-10-01" };
    assert.deepEqual(newestListings([goneOther, gonePref], pref).albums.map((a) => a.id), ["400"]);

    // collapseDups ranks live before preferred, then preferred before links/updated_at.
    const collapsed = collapseDups([newerOther, olderPref], new Set(), pref);
    assert.deepEqual(collapsed.out.map((a) => a.id), ["100"]);
    assert.equal(collapsed.counts.get("i:weidian:7"), 2);
    const collapsedLive = collapseDups([gonePref, liveOther], new Set(), pref);
    assert.deepEqual(collapsedLive.out.map((a) => a.id), ["200"]);
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
