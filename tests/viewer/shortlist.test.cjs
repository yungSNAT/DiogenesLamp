// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const vm = require("node:vm");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

const {
  SHORTLIST_STORAGE_KEY,
  SHORTLIST_LIMITS,
  safeHttpUrl,
  normalizeShortlistItem,
  normalizeShortlist,
  readShortlist,
  writeShortlist,
  toggleShortlistItem,
  updateShortlistItem,
  removeShortlistItem,
  primaryAgentUrl,
  shortlistToJson,
  escapeMarkdownCell,
  shortlistToMarkdown,
} = require("../../viewer/shortlist.js");
const { defaultFilters, anyFilterActive, serializeFilters, filtersFromURL, filtersToURL } =
  require("../../viewer/filters.js");
const { createViewState, selectCatalogue } = require("../../viewer/view-state.js");

function fakeHost(initial = {}, fail = false) {
  const data = { ...initial };
  return {
    data,
    localStorage: {
      getItem(k) {
        if (fail) throw new Error("storage disabled");
        return Object.prototype.hasOwnProperty.call(data, k) ? data[k] : null;
      },
      setItem(k, v) {
        if (fail) throw new Error("quota exceeded");
        data[k] = String(v);
      },
    },
  };
}

function blockedHost() {
  const host = {};
  Object.defineProperty(host, "localStorage", {
    get() {
      const err = new Error("Access is denied");
      err.name = "SecurityError";
      throw err;
    },
  });
  return host;
}

test("shortlist storage round-trips items with size, colour and note", () => {
  const host = fakeHost();
  let list = readShortlist(host);
  assert.equal(list.size, 0);

  const first = toggleShortlistItem(list, "100", {
    size: "  L  ",
    colour: "Navy",
    note: "Check sleeve length\nprefer heavy batch",
  }, "2026-11-24T08:00:00.000Z");
  assert.equal(first.added, true);
  list = first.shortlist;

  list = updateShortlistItem(list, "100", { size: "XL" }, "2026-11-24T09:00:00.000Z");
  assert.equal(list.get("100").size, "XL");
  assert.equal(list.get("100").colour, "Navy");
  assert.equal(list.get("100").added_at, "2026-11-24T08:00:00.000Z");

  assert.equal(writeShortlist(host, list), true);
  const reloaded = readShortlist(host);
  assert.deepEqual([...reloaded.values()], [{
    id: "100",
    size: "XL",
    colour: "Navy",
    note: "Check sleeve length\nprefer heavy batch",
    added_at: "2026-11-24T08:00:00.000Z",
  }]);

  const toggledOff = toggleShortlistItem(reloaded, "100");
  assert.equal(toggledOff.added, false);
  assert.equal(toggledOff.shortlist.size, 0);
});

test("shortlist storage handles blocked, failing and corrupt storage safely", () => {
  assert.equal(readShortlist(blockedHost()).size, 0);
  assert.equal(writeShortlist(blockedHost(), new Map()), false);
  assert.equal(readShortlist(fakeHost({}, true)).size, 0);
  assert.equal(writeShortlist(fakeHost({}, true), new Map()), false);
  assert.equal(readShortlist(fakeHost({ [SHORTLIST_STORAGE_KEY]: "{not-json" })).size, 0);
  assert.equal(readShortlist(fakeHost({ [SHORTLIST_STORAGE_KEY]: "42" })).size, 0);
});

test("shortlist normalisation trims control characters, caps lengths and deduplicates IDs", () => {
  const item = normalizeShortlistItem({
    id: " 200\u0000 ",
    size: "X".repeat(80),
    colour: "Blue\u0007",
    note: "Line 1\u0000\nLine 2",
    added_at: "2026-11-24",
  });
  assert.equal(item.id, "200");
  assert.equal(item.size.length, SHORTLIST_LIMITS.maxSize);
  assert.equal(item.colour, "Blue");
  assert.equal(item.note, "Line 1\nLine 2");

  const map = normalizeShortlist({
    version: 1,
    items: [
      { id: "100", size: "M", colour: "Black", note: "first" },
      { id: "100", size: "L", colour: "Grey", note: "duplicate" },
      { id: "" },
      null,
      { id: "200", size: 42 },
    ],
  });
  assert.deepEqual([...map.keys()], ["100", "200"]);
  assert.equal(map.get("100").size, "M");
  assert.equal(map.get("200").size, "");

  const removed = removeShortlistItem(map, "100");
  assert.deepEqual([...removed.keys()], ["200"]);
});

test("shortlistOnly filter integrates with filters.js and selectCatalogue including delisted albums", () => {
  const active = { ...defaultFilters(), shortlistOnly: true };
  assert.equal(anyFilterActive(active), true);
  assert.deepEqual(serializeFilters(active), { shortlistOnly: true });
  assert.deepEqual(filtersFromURL(filtersToURL(active)), active);

  const shortlist = new Map([["gone-1", { id: "gone-1", size: "L", colour: "", note: "", added_at: "" }]]);
  const albums = [
    { id: "live-1", seller: "one", title: "Live jacket", gone_since: "", published_ms: 2 },
    { id: "gone-1", seller: "one", title: "Gone hoodie", gone_since: "2026-11-20", published_ms: 1 },
  ];
  const catalogue = { albums, favorites: new Set(), shortlist, bannedKeys: new Set() };
  assert.deepEqual(selectCatalogue(createViewState(), catalogue).filtered.map(a => a.id), ["live-1"]);
  assert.deepEqual(
    selectCatalogue(createViewState({ shortlistOnly: true }), catalogue).filtered.map(a => a.id),
    ["gone-1"],
  );
});

test("primaryAgentUrl rebuilds for configured agent, falls back to direct agent links and rejects unsafe schemes", () => {
  const album = {
    id: "100",
    links: [
      { platform: "weidian", url: "https://weidian.com/item.html?itemID=7234567890" },
    ],
  };
  const rebuilt = primaryAgentUrl(album, "superbuy");
  assert.match(rebuilt, /^https:\/\/www\.superbuy\.com\//);
  assert.equal(primaryAgentUrl(album, ""), "");

  const directAgent = {
    id: "101",
    links: [
      { platform: "superbuy", url: "https://www.superbuy.com/en/page/buy/?url=https%3A%2F%2Fweidian.com%2Fitem.html%3FitemID%3D7234567890" },
    ],
  };
  assert.equal(primaryAgentUrl(directAgent, ""), directAgent.links[0].url);

  const unsafe = {
    id: "102",
    links: [{ platform: "weidian", url: "javascript:alert(1)" }],
  };
  assert.equal(primaryAgentUrl(unsafe, "superbuy"), "");
  assert.equal(safeHttpUrl("javascript:alert(1)"), "");
  assert.equal(safeHttpUrl("data:text/html,<h1>x</h1>"), "");
  assert.equal(safeHttpUrl("https://example.com/a(1)"), "https://example.com/a(1)");
});

test("shortlistToJson and shortlistToMarkdown export structured data, escape hostile text and omit em dashes", () => {
  let list = new Map();
  list = updateShortlistItem(list, "100", {
    size: "L",
    colour: "Black | <script>alert(1)</script>",
    note: "Check [chest](javascript:alert(1))\nsecond line",
  }, "2026-11-24T08:00:00.000Z");
  list = updateShortlistItem(list, "999", { size: "43", colour: "White", note: "Unloaded" }, "2026-11-24T08:01:00.000Z");

  const albumsById = new Map([
    ["100", {
      id: "100",
      seller: "seller_a",
      title: "Heavy | Hoodie [v2]",
      price: 250,
      album_url: "https://seller_a.x.yupoo.com/albums/100(test)",
      links: [{ platform: "weidian", url: "https://weidian.com/item.html?itemID=7234567890" }],
    }],
  ]);
  const prefs = { agent: "superbuy", lookups: false, currency: "AUD", rate: "0.21" };

  const json = JSON.parse(shortlistToJson(list, albumsById, prefs, "2026-11-24T08:05:00.000Z"));
  assert.equal(json.format, "diogeneslamp-shortlist");
  assert.equal(json.version, 1);
  assert.equal(json.count, 2);
  assert.equal(json.priced_count, 1);
  assert.equal(json.total_cny, 250);
  assert.equal(json.agent, "superbuy");
  assert.equal(json.items[0].loaded, true);
  assert.match(json.items[0].agent_url, /^https:\/\/www\.superbuy\.com\//);
  assert.equal(json.items[1].loaded, false);
  assert.equal(json.items[1].price_cny, null);

  const md = shortlistToMarkdown(list, albumsById, prefs);
  assert.match(md, /^# DiogenesLamp Shortlist \(2 items, est\. ¥250/);
  assert.match(md, /\| Item \| Seller \| Price \| Size \| Colour \| Note \| Links \|/);
  assert.match(md, /Heavy \\\| Hoodie \\\[v2\\\]/);
  assert.match(md, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
  assert.match(md, /Check \\\[chest\\\]\(javascript:alert\(1\)\) second line/);
  assert.match(md, /\[Yupoo\]\(https:\/\/seller_a\.x\.yupoo\.com\/albums\/100%28test%29\)/);
  assert.match(md, /\[Agent\]\(https:\/\/www\.superbuy\.com\//);
  assert.match(md, /Album 999 \(not loaded\)/);
  assert.equal(md.includes("\u2014"), false, "Markdown export must not contain em dashes");
  assert.equal(escapeMarkdownCell("a|b\nc"), "a\\|b c");

  const emptyMd = shortlistToMarkdown(new Map(), albumsById, prefs);
  assert.match(emptyMd, /No items shortlisted\./);
});

test("shortlist controller renders safely with textContent, exports JSON/Markdown and syncs across storage events", async () => {
  class Element {
    constructor(tag = "div") {
      this.tagName = tag.toUpperCase();
      this.value = "";
      this.textContent = "";
      this.hidden = false;
      this.open = false;
      this.disabled = false;
      this.checked = false;
      this.isConnected = true;
      this.children = [];
      this.events = {};
      this.attrs = {};
      this.options = [{ value: "", cloneNode() { return this; } }];
    }
    addEventListener(name, cb) { (this.events[name] ||= []).push(cb); }
    fire(name, extra = {}) {
      for (const cb of this.events[name] || []) {
        cb({ target: this, currentTarget: this, preventDefault() {}, ...extra });
      }
    }
    setAttribute(k, v) { this.attrs[k] = String(v); }
    removeAttribute(k) { delete this.attrs[k]; }
    appendChild(child) { this.children.push(child); return child; }
    append(...children) { this.children.push(...children); }
    replaceChildren(...children) { this.children = children; }
    add(option) { this.options.push(option); }
    querySelector() { return this; }
    closest() { return null; }
    contains() { return false; }
    focus() { this.focused = true; }
    showModal() { this.open = true; }
    close() { this.open = false; this.fire("close"); }
  }

  const elements = new Map();
  const data = new Map();
  const downloads = [];
  const copied = [];
  const windowEvents = {};
  const el = id => {
    if (!elements.has(id)) elements.set(id, new Element());
    return elements.get(id);
  };
  const win = {
    localStorage: {
      get length() { return data.size; },
      key: i => [...data.keys()][i] ?? null,
      getItem: k => data.get(k) ?? null,
      setItem: (k, v) => data.set(k, String(v)),
    },
    addEventListener(name, cb) { (windowEvents[name] ||= []).push(cb); },
    confirm: () => true,
    scrollTo() {},
  };

  const context = {
    window: win,
    navigator: { clipboard: { async writeText(text) { copied.push(text); } } },
    HTMLElement: Element,
    document: {
      activeElement: null,
      getElementById: el,
      createElement: tag => new Element(tag),
      body: { classList: { add() {}, remove() {} } },
    },
    location: { href: "http://localhost/", search: "" },
    history: { pushState() {}, replaceState() {} },
    URL,
    Blob,
    setTimeout,
    console,
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    applyFilters() {},
    renderGrid() {},
    thumbVersion: () => "1",
    favLabel: v => v ? "Remove favourite" : "Add favourite",
    toggleFavorite(id) { context.userStore.set("favorite", id, !context.userStore.get("favorite", id)); },
    toggleShortlist(id) {
      const next = context.toggleShortlistItem(context.state.shortlist, id);
      context.writeShortlist(win, next.shortlist);
      context.state.shortlist = next.shortlist;
      context.syncShortlistUI();
    },
    updateShortlistCount() {},
    syncAllShortlistCards() {},
    moveCursor(at) { context.cursor = at; },
    savedDataError() {},
  };

  for (const name of ["user-store", "discovery", "dedup", "filters", "prefs", "agents", "catalogue", "kinds", "view-state", "shortlist", "shortcuts"]) {
    Object.assign(context, require(`../../viewer/${name}.js`));
  }
  context.userStore = context.createUserStore(win);
  context.state = {
    albums: [{
      id: "100",
      seller: "one",
      title: "<img src=x onerror=alert(1)> Wool Coat",
      price: 420,
      thumb: "",
      album_url: "https://one.x.yupoo.com/albums/100",
      links: [{ platform: "weidian", url: "https://weidian.com/item.html?itemID=7234567890" }],
      history: [],
    }],
    filtered: [],
    albumAliases: {},
    listingAliases: {},
    favorites: new Set(),
    shortlist: new Map(),
    view: context.createViewState(),
    prefs: context.normalizePrefs({ agent: "superbuy" }),
    index: { sellers: [] },
    loadedSellers: 0,
    failedShards: [],
    listingCopies: new Map(),
  };
  context.state.filtered = [...context.state.albums];
  context.els = {
    collection: el("collection-filter"),
    tag: el("tag-filter"),
    seller: el("seller-filter"),
    shortlistOnly: el("shortlist-only"),
    exportFavs: el("export-favs"),
    grid: el("grid"),
  };

  vm.createContext(context);
  vm.runInContext(readFileSync(join(__dirname, "../../viewer/saved-work.js"), "utf8"), context);
  context.downloadWork = (text, name, type) => downloads.push({ text, name, type });

  // Open preview, add to shortlist, and save size/colour/note
  context.openPreview("100");
  assert.equal(el("preview-shortlist-fields").hidden, true);
  el("preview-shortlist").fire("click");
  assert.equal(context.state.shortlist.has("100"), true);
  assert.equal(el("preview-shortlist-fields").hidden, false);

  el("preview-shortlist-size").value = "L";
  el("preview-shortlist-colour").value = "Charcoal";
  el("preview-shortlist-note").value = "<script>alert(1)</script>";
  el("preview-shortlist-save").fire("click");
  assert.equal(context.state.shortlist.get("100").note, "<script>alert(1)</script>");

  // Open shortlist dialog and verify DOM uses textContent (no innerHTML property written)
  el("shortlist-toggle").fire("click");
  assert.equal(el("shortlist-dialog").open, true);
  assert.equal(el("shortlist-list").children.length, 1);
  const row = el("shortlist-list").children[0];
  assert.equal(row.innerHTML, undefined);
  assert.equal(row.children[0].children[0].textContent, "<img src=x onerror=alert(1)> Wool Coat");

  // Export JSON and Markdown + copy to clipboard
  el("shortlist-export-json").fire("click");
  el("shortlist-export-md").fire("click");
  assert.equal(downloads.length, 2);
  assert.equal(downloads[0].name, "diogenes-shortlist.json");
  assert.equal(JSON.parse(downloads[0].text).items[0].colour, "Charcoal");
  assert.equal(downloads[1].name, "diogenes-shortlist.md");
  assert.match(downloads[1].text, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);

  el("shortlist-copy-md").fire("click");
  el("shortlist-copy-json").fire("click");
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(copied.length, 2);
  assert.match(copied[0], /^# DiogenesLamp Shortlist/);
  assert.equal(JSON.parse(copied[1]).count, 1);

  // Filter grid to shortlist
  el("shortlist-filter-btn").fire("click");
  assert.equal(context.state.view.filters.shortlistOnly, true);
  assert.equal(el("shortlist-dialog").open, false);

  // Cross-tab storage sync updates state.shortlist
  const external = context.updateShortlistItem(new Map(), "100", { size: "M", colour: "Tan", note: "From tab 2" });
  context.writeShortlist(win, external);
  for (const cb of windowEvents.storage || []) cb({ key: SHORTLIST_STORAGE_KEY });
  assert.equal(context.state.shortlist.get("100").size, "M");
  assert.equal(context.state.shortlist.get("100").colour, "Tan");
});

test("toggleShortlistItem reports full at capacity without mutating or mimicking removal", () => {
  const fullMap = new Map();
  for (let i = 1; i <= SHORTLIST_LIMITS.maxItems; i += 1) {
    fullMap.set(String(i), { id: String(i), size: "", colour: "", note: "", added_at: "" });
  }
  const blocked = toggleShortlistItem(fullMap, "99999");
  assert.equal(blocked.added, false);
  assert.equal(blocked.full, true);
  assert.equal(blocked.shortlist.size, SHORTLIST_LIMITS.maxItems);
  assert.equal(blocked.shortlist.has("99999"), false);

  // Removing an existing entry when at capacity still succeeds cleanly.
  const removed = toggleShortlistItem(fullMap, "1");
  assert.equal(removed.added, false);
  assert.equal(removed.full, false);
  assert.equal(removed.shortlist.size, SHORTLIST_LIMITS.maxItems - 1);
  assert.equal(removed.shortlist.has("1"), false);
});

test("app.js toggleShortlist reports capacity limit without writing storage or announcing removal", () => {
  const appSource = readFileSync(join(__dirname, "../../viewer/app.js"), "utf8");
  const match = appSource.match(/\nfunction toggleShortlist\([\s\S]*?\n\}/);
  assert.ok(match, "app.js no longer declares toggleShortlist");

  const fullMap = new Map();
  for (let i = 1; i <= SHORTLIST_LIMITS.maxItems; i += 1) {
    fullMap.set(String(i), { id: String(i), size: "", colour: "", note: "", added_at: "" });
  }
  let writes = 0;
  const statuses = [];
  const context = {
    SHORTLIST_LIMITS,
    toggleShortlistItem,
    window: {},
    state: { shortlist: fullMap, albums: [], view: { filters: { shortlistOnly: false } } },
    writeShortlist() { writes += 1; return true; },
    savedDataError() { throw new Error("unexpected savedDataError"); },
    updateShortlistCount() {},
    syncAllShortlistCards() {},
    applyFilters() {},
    syncShortlistUI() {},
    shortlistLabel: v => v ? "Remove from shortlist" : "Add to shortlist",
    actionStatus(msg) { statuses.push(msg); },
  };
  vm.createContext(context);
  vm.runInContext(match[0], context);

  assert.equal(context.toggleShortlist("99999"), false);
  assert.equal(writes, 0);
  assert.equal(context.state.shortlist.size, SHORTLIST_LIMITS.maxItems);
  assert.equal(statuses.length, 1);
  assert.match(statuses[0], /Shortlist is full/);
  assert.doesNotMatch(statuses[0], /Removed from shortlist/);
});
