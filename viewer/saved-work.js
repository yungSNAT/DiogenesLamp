// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";

// Local browser work is separate from fetched catalogue metadata.
/** @template {keyof import("./controller-types.js").ViewerElements} K
 * @param {K} id @returns {import("./controller-types.js").ViewerElements[K]} */
const workEl = id => document.getElementById(id);
const preview = workEl("album-preview");
/** @type {string[]} */
let previewIds = [];
let previewAt = 0;
let previewReturnId = "";
/** @type {Partial<Record<"collection" | "tags" | "notes", string>>} */
let previewInitial = {};
/** @type {import('./controller-types.js').ViewerAlbum | null} */
let previewSnapshot = null;
let previewUnresolved = false;

/** @param {string} text */
function actionStatus(text) {
  const inPreview = preview.open;
  const pageStatus = workEl("action-status");
  pageStatus.textContent = inPreview ? "" : text;
  pageStatus.hidden = inPreview || !text;
  const previewStatus = workEl("preview-action-status");
  previewStatus.textContent = inPreview ? text : "";
  previewStatus.hidden = !inPreview || !text;
}

function saveFilterURL() {
  const query = filtersToURL(state.view.filters);
  if (location.search !== query) history.pushState(null, "", query);
}

/** @param {HTMLSelectElement} select @param {string[]} values */
function selectOptions(select, values, wanted = select.value) {
  const first = select.options[0].cloneNode(true);
  select.replaceChildren(first);
  for (const value of [...new Set(values)].filter(Boolean).sort((a, b) => a.localeCompare(b))) {
    select.add(new Option(value, value));
  }
  // Keep a missing filter explicit: an imported collection may refer to an
  // unavailable shard. Silently clearing it would display unrelated albums.
  if (wanted && ![...select.options].some(o => o.value === wanted)) {
    select.add(new Option(wanted + " (unavailable)", wanted));
  }
  select.value = wanted;
}

function applyInheritedShortlist() {
  if (!state.shortlist || typeof inheritShortlistAliases !== "function") return false;
  const inherited = inheritShortlistAliases(state.shortlist, state.albumAliases);
  if (!inherited.changed) return false;
  state.shortlist = inherited.shortlist;
  writeShortlist(window, state.shortlist);
  return true;
}

function refreshSavedState(render = true) {
  inheritDuplicateWork(userStore, state.albumAliases);
  if (applyInheritedShortlist()) {
    if (typeof updateShortlistCount === "function") updateShortlistCount();
    if (typeof syncAllShortlistCards === "function") syncAllShortlistCards();
    if (workEl("shortlist-dialog").open) renderShortlistDialog();
  }
  const entries = userStore.load();
  /** @template {import("./user-store.js").UserKind} K @param {K} kind
   * @returns {Map<string, import("./user-store.js").UserValues[K]>} */
  const section = kind => new Map(entries.filter(e => e.kind === kind).map(e => [e.id, /** @type {import("./user-store.js").UserValues[K]} */ (e.value)]));
  state.favorites = new Set(entries.filter(e => e.kind === "favorite" && e.value).map(e => e.id));
  state.collections = section("collection");
  state.tags = section("tags");
  state.reviews = section("review");
  selectOptions(els.collection, [...state.collections.values()], state.view.filters.collection);
  selectOptions(els.tag, [...state.tags.values()].flat(), state.view.filters.tag);
  selectOptions(workEl("saved-search"), entries.filter(e => e.kind === "search" && e.value).map(e => e.id));
  if (render) applyFilters();
  if (preview.open) {
    reconcilePreview(); updatePreviewFavourite(); updatePreviewShortlist();
    if (previewSnapshot) showListingCopies(previewSnapshot);
  }
}

/** @param {unknown} filters */
function useFilters(filters) {
  const saved = normalizeFilters(filters);
  refreshSavedState(false);
  for (const [key, value] of /** @type {[keyof import("./filters.js").Filters, string | boolean][]} */ (Object.entries(saved))) {
    if (!els[key]) continue;
    if (typeof value === "boolean") /** @type {HTMLInputElement} */ (els[key]).checked = value;
    else {
      if (els[key].tagName === "SELECT" && value &&
          ![.../** @type {HTMLSelectElement} */ (els[key]).options].some(o => o.value === value)) {
        /** @type {HTMLSelectElement} */ (els[key]).add(new Option(value + " (unavailable)", value));
      }
      els[key].value = value;
    }
  }
  state.view = nextViewState(state.view, {type: "filter", filters: saved});
  writeFilters(window, saved);
  window.scrollTo(0, 0);
  applyFilters();
}
window.addEventListener("popstate", () => useFilters(filtersFromURL(location.search) || defaultFilters()));
window.addEventListener("storage", event => {
  const { key: storageKey } = event;
  if (storageKey === null || storageKey.startsWith(USER_PREFIX) ||
      storageKey.startsWith(INHERITED_PREFIX)) refreshSavedState();
  if (storageKey === null || storageKey === PREFS_STORAGE_KEY) {
    state.prefs = readPrefs(window);
    showPrefs();
    applyPrefs();
  }
  if (storageKey === null || storageKey === SHORTLIST_STORAGE_KEY) {
    state.shortlist = readShortlist(window);
    applyInheritedShortlist();
    if (typeof updateShortlistCount === "function") updateShortlistCount();
    if (typeof syncAllShortlistCards === "function") syncAllShortlistCards();
    if (state.view.filters.shortlistOnly) applyFilters();
    syncShortlistUI();
  }
});

/** @param {string} text @param {string} name @param {string} type */
function downloadWork(text, name, type) {
  const url = URL.createObjectURL(new Blob([text], {type}));
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function catalogueAvailability() {
  const loading = !state.index || state.loadedSellers < state.index.sellers.length;
  return loading ? "partial: still loading" : state.failedShards.length
    ? "partial: unavailable sellers " + state.failedShards.join(", ") : "complete";
}

els.exportFavs.addEventListener("click", () => {
  const entries = userStore.load();
  const ids = new Set(entries.filter(e => ["favorite", "note", "tags", "collection"].includes(e.kind))
    .map(e => e.id));
  for (const id of state.shortlist?.keys() || []) ids.add(id);
  const canonicalId = /** @param {string} id */ id =>
    typeof resolveShortlistAlias === "function" ? resolveShortlistAlias(id, state.albumAliases).target
      : (state.albumAliases[id] || id);
  const resolvedIds = new Set([...ids].map(canonicalId));
  const albums = state.albums.filter(a => ids.has(a.id) || resolvedIds.has(a.id)).map(a => ({
    id: a.id, seller: a.seller, title: a.title, price: a.price, album_url: a.album_url,
    links: a.links, category: a.category, updated_at: a.updated_at,
  }));
  const present = new Set(albums.map(a => a.id));
  const missingIds = [...ids].filter(id => !present.has(id) && !present.has(canonicalId(id)));
  const shortlist = state.shortlist?.size ? serializeShortlist(state.shortlist) : undefined;
  const backup = {format: "diogeneslamp", version: 1, exported_at: new Date().toISOString(),
    catalogue_status: catalogueAvailability(), entries, albums, unresolved_ids: missingIds,
    ...(shortlist ? {shortlist} : {})};
  downloadWork(JSON.stringify(backup, null, 2), "diogenes-saved-work.json", "application/json");
  const shortlistNote = shortlist ? ` and ${shortlist.items.length} shortlist items` : "";
  actionStatus(`Backed up ${entries.length} saved fields${shortlistNote}. ${missingIds.length} album IDs have no loaded metadata; their saved work is included.`);
});

workEl("import-backup").addEventListener("change", async event => {
  const file = /** @type {FileList} */ (/** @type {HTMLInputElement} */ (event.target).files)[0];
  if (!file) return;
  try {
    if (file.size > 20 * 1024 * 1024) throw new Error("Backup exceeds 20 MB.");
    const parsed = JSON.parse(await file.text());
    const count = importUserBackup(userStore, parsed);
    let shortlistAdded = 0;
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed) && parsed.shortlist !== undefined) {
      const merged = mergeShortlistBackup(state.shortlist || new Map(), parsed.shortlist, state.albumAliases);
      if (merged.added > 0) {
        if (!writeShortlist(window, merged.shortlist)) throw new Error("Storage write failed.");
        state.shortlist = merged.shortlist;
        shortlistAdded = merged.added;
        if (typeof updateShortlistCount === "function") updateShortlistCount();
        if (typeof syncAllShortlistCards === "function") syncAllShortlistCards();
      }
    }
    const shortlistNote = shortlistAdded ? ` and ${shortlistAdded} shortlist items` : "";
    actionStatus(`Imported ${count} saved fields${shortlistNote}. Existing values were kept.`);
  } catch (error) { savedDataError(/** @type {Error} */ (error).message); }
  finally { refreshSavedState(); /** @type {HTMLInputElement | HTMLSelectElement} */ (event.target).value = ""; }
});

workEl("export-csv").addEventListener("click", () => {
  const availability = catalogueAvailability();
  const scope = `${availability}; current results; ${filtersToURL(state.view.filters)}`;
  downloadWork(catalogueCsv(state.filtered, scope, state.prefs.agent), "diogenes-results.csv", "text/csv;charset=utf-8");
  actionStatus(`Exported ${state.filtered.length} current results. Catalogue: ${availability}.`);
});
workEl("save-search").addEventListener("click", () => {
  const name = workEl("search-name").value.trim();
  if (!name) { workEl("search-name").focus(); actionStatus("Enter a name for this search."); return; }
  if (userStore.set("search", name, currentFilters())) {
    refreshSavedState(false);
    workEl("saved-search").value = name;
    actionStatus("Search saved.");
  }
});
workEl("saved-search").addEventListener("change", event => {
  const name = /** @type {HTMLInputElement | HTMLSelectElement} */ (event.target).value;
  const filters = userStore.get("search", name);
  if (filters) { useFilters(filters); saveFilterURL(); workEl("search-name").value = name; }
});
workEl("remove-search").addEventListener("click", () => {
  const name = workEl("saved-search").value;
  if (name && userStore.set("search", name, null)) {
    workEl("saved-search").value = "";
    refreshSavedState(false);
    actionStatus("Saved search removed.");
  }
});
workEl("mark-reviewed").addEventListener("click", () => {
  const seller = els.seller.value;
  if (!seller || !state.shardAlbums.has(seller) || state.failedShards.includes(seller)) {
    actionStatus("Choose a seller whose catalogue has finished loading.");
    return;
  }
  // Use the snapshot timestamp, not the wall clock: a later downloaded
  // catalogue can contain changes already older than today's browsing time.
  const cutoff = /** @type {import("./controller-types.js").ViewerAlbum[]} */ (state.shardAlbums.get(seller)).reduce((latest, album) => {
    const date = Date.parse(album.last_seen || album.updated_at || "");
    return Number.isFinite(date) ? Math.max(latest, date) : latest;
  }, 0) || Date.parse(state.generatedAt);
  if (!Number.isFinite(cutoff)) { actionStatus("This catalogue has no valid review date."); return; }
  if (userStore.set("review", seller, cutoff)) {
    refreshSavedState();
    actionStatus(`${seller} marked reviewed through ${new Date(cutoff).toISOString()}.`);
  }
});

function previewAlbum() { return previewSnapshot; }
/** @param {string} id */
function resolvePreview(id) {
  const target = typeof resolveShortlistAlias === "function"
    ? resolveShortlistAlias(id, state.albumAliases).target
    : state.albumAliases[id];
  return state.albums.find(a => a.id === id) || state.albums.find(a => a.id === target);
}
/** @param {boolean} unresolved */
function previewAvailability(unresolved) {
  previewUnresolved = unresolved;
  workEl("preview-save").disabled = unresolved;
  workEl("preview-favourite").disabled = unresolved;
  workEl("preview-shortlist").disabled = unresolved;
  workEl("preview-shortlist-save").disabled = unresolved;
  workEl("preview-prev").disabled = unresolved || previewAt === 0;
  workEl("preview-next").disabled = unresolved || previewAt + 1 === previewIds.length;
  workEl("preview-recovery").hidden = !unresolved;
  const latest = previewSnapshot && resolvePreview(previewSnapshot.id);
  workEl("preview-transfer").disabled = !latest || latest.id === previewSnapshot?.id;
  workEl("preview-discard").disabled = !latest;
  if (unresolved) {
    workEl("preview-similar").open = false;
    workEl("similar-results").replaceChildren();
    workEl("preview-save-status").textContent = latest
      ? `Album ${previewSnapshot?.id} was replaced by ${latest.id}. Your draft is retained. Transfer stages edited fields only where the latest listing has no saved value; review before saving.`
      : "This listing is unavailable and has no confirmed loaded replacement. Your draft is retained; saving, favourites and navigation are paused.";
  }
}
function reconcilePreview() {
  if (!previewSnapshot) return;
  const latest = resolvePreview(previewSnapshot.id);
  if (latest?.id === previewSnapshot.id) { previewAvailability(false); return; }
  if (!latest || previewDirty()) { previewAvailability(true); return; }
  previewIds = previewIds.map(id => id === previewSnapshot?.id ? latest.id : id);
  previewReturnId = latest.id;
  showPreview(latest);
}
function previewReady() {
  reconcilePreview();
  return Boolean(previewSnapshot && !previewUnresolved && state.albums.some(a => a.id === previewSnapshot?.id));
}
function activePreviewAlbum() {
  return preview.open && previewReady() ? previewSnapshot : null;
}

function updatePreviewFavourite() {
  const fav = workEl("preview-favourite");
  const chosen = state.favorites.has(previewSnapshot?.id || "");
  fav.setAttribute("aria-pressed", String(chosen));
  fav.textContent = favLabel(chosen);
}

/** @param {boolean} [resetInputs] */
function updatePreviewShortlist(resetInputs = false) {
  const btn = workEl("preview-shortlist");
  const fields = workEl("preview-shortlist-fields");
  const id = previewSnapshot?.id || "";
  const entry = id ? state.shortlist?.get(id) : undefined;
  const active = Boolean(entry);
  btn.setAttribute("aria-pressed", String(active));
  btn.textContent = active ? "Remove from shortlist" : "Add to shortlist";
  fields.hidden = !active;
  const focusedInside = typeof fields.contains === "function" && fields.contains(document.activeElement);
  if (active && entry && (resetInputs || !focusedInside)) {
    workEl("preview-shortlist-size").value = entry.size;
    workEl("preview-shortlist-colour").value = entry.colour;
    workEl("preview-shortlist-note").value = entry.note;
  }
  if (!active || resetInputs) workEl("preview-shortlist-status").textContent = "";
}
/** @param {unknown} value */
function localDate(value) { return value ? String(value).replace("T", " ") : "Not recorded"; }

// A link that opens in a new tab, added only for an http(s) address.
/** @param {HTMLElement} parent @param {string} text @param {string} href */
function addLink(parent, text, href) {
  const url = new URL(href, location.href);
  if (!["https:", "http:"].includes(url.protocol)) return;
  const a = document.createElement("a");
  a.href = url.href; a.target = "_blank"; a.rel = "noreferrer"; a.textContent = text;
  parent.appendChild(a);
}

// The read-only part of the preview that preferences change: price, stored
// text and links. Kept apart from showPreview so a changed agent or rate
// never repopulates the notes form and discards an edit in progress.
/** @param {import("./controller-types.js").ViewerAlbum | null} album */
function showPreviewLinks(album) {
  if (!album) return;
  const evidence = album.price_evidence;
  const converted = convertedPrice(album.price, state.prefs);
  workEl("preview-price").textContent = album.price ? `CNY ${album.price}` +
    (evidence?.token && evidence?.rule ? ` · ${evidence.field || "album text"}: ${evidence.token} (${evidence.rule})` +
      (/** @type {number} */ (evidence.candidates?.length) > 1 ? ` · Other candidates: ${/** @type {unknown[]} */ (evidence.candidates).join(", ")}` : "") : " · Extraction evidence not recorded") +
    (converted ? ` · ${converted} at 1 CNY = ${state.prefs.rate} ${state.prefs.currency}, your rate, before fees and shipping` : "")
    : "Price unknown";
  workEl("preview-description").textContent = !("description" in album) ? "Description not recorded: it is kept from the next detail fetch (refresh budget or full scrape)."
    : !album.description ? "No description extracted at the last detail check."
    : album.description + (album.description_truncated ? "\n(Saved excerpt; the description was longer.)" : "");
  const links = workEl("preview-links");
  links.replaceChildren();
  addLink(links, "Original album", album.album_url);
  const agent = ACTIVE_AGENTS.find(candidate => candidate.id === state.prefs.agent);
  for (const link of album.links) {
    addLink(links, link.platform, link.url);
    const via = agent ? agentLink(agent.id, marketplaceItem(link.url)) : "";
    if (via && agent) addLink(links, `${link.platform} on ${agent.name}`, via);
  }
  showListingCopies(album);
  const external = workEl("preview-external");
  external.replaceChildren();
  if (state.prefs.lookups) {
    const items = /** @type {import("./agents.js").MarketplaceItem[]} */ (album.links.map(link => marketplaceItem(link.url)).filter(Boolean));
    for (const item of items) {
      addLink(external, `QC photos on rep.tools for ${item.platform} ${item.id}`,
        "https://rep.tools/qc-finder?link=" + encodeURIComponent(marketplaceUrl(item)));
    }
    if (!items.length) external.textContent = "No marketplace item to look up.";
  }
}

// Every listing of the item, this one included, cheapest first, so 2
// sellers' prices compare whilst the grid still shows the newest only. The
// aliases name every other listing; one not loaded yet, or not readable, is
// counted rather than left out silently.
/** @param {import('./controller-types.js').ViewerAlbum} album */
function showListingCopies(album) {
  const copies = state.listingCopies.get(album.id) || [];
  const named = Object.values(state.listingAliases).filter(id => id === album.id).length;
  const missing = Math.max(named - copies.length, 0);
  const listings = workEl("preview-copies");
  listings.replaceChildren();
  listings.hidden = workEl("preview-copies-note").hidden = !copies.length && !missing;
  const loading = state.loadedSellers < (state.index?.sellers.length ?? 0);
  const failed = state.failedShards.length > 0;
  const why = loading && failed ? "still loading or could not be read" : loading ? "still loading" : "could not be read";
  // Only a complete index names every seller's listings; otherwise, whilst
  // a seller is unread, the count covers the sellers read.
  const scope = state.index?.selection_version !== 1 && (loading || failed) ? " in the sellers read" : "";
  workEl("preview-copies-note").textContent = listings.hidden ? ""
    : `Listed ${copies.length + missing + 1} times${scope}` + (missing ? `; ${missing} ${why}. Cheapest first:` : ", cheapest first:");
  /** @param {{price?: number | null}} listing */
  const priced = listing => typeof listing.price === "number" && listing.price > 0;
  const ordered = [album, ...copies].sort((a, b) => priced(a) && priced(b) ? /** @type {number} */ (a.price) - /** @type {number} */ (b.price)
    : Number(priced(b)) - Number(priced(a)));
  for (const listing of listings.hidden ? [] : ordered) {
    const item = document.createElement("li");
    const prefCopy = state.preferredSellers?.has(listing.seller) ? " (preferred)" : "";
    item.textContent = `${listing.seller}${prefCopy} · ${priced(listing) ? `CNY ${listing.price}` : "Price unknown"} · ` +
      (listing.date_published ? `posted ${localDate(listing.date_published)}` : `first seen ${localDate(listing.first_seen)}`) +
      (listing.gone_since ? " · no longer listed" : "") + (listing === album ? " · shown here " : " ");
    addLink(item, "Original album", listing.album_url);
    listings.appendChild(item);
  }
}

/** @param {import('./controller-types.js').ViewerAlbum | undefined} album */
function showPreview(album = resolvePreview(previewIds[previewAt])) {
  if (!album) return false;
  previewSnapshot = album;
  // The heading names the dialog, so an untitled album keeps the default name.
  workEl("preview-title").textContent = album.title.trim() || "Album preview";
  workEl("preview-position").textContent = `${previewAt + 1} of ${previewIds.length}`;
  workEl("preview-prev").disabled = previewAt === 0;
  workEl("preview-next").disabled = previewAt + 1 === previewIds.length;
  const prefNote = state.preferredSellers?.has(album.seller) ? " (preferred)" : "";
  workEl("preview-facts").textContent = `${album.seller}${prefNote} · Album ${album.id} · ${album.category || "No category"}` +
    ` · Kind: ${album.kind || UNSORTED} (read from the title, experimental)` +
    (album.gloss?.length ? `\nTitle words (experimental): ${album.gloss.join(", ")}` : "") +
    `\nFirst seen: ${localDate(album.first_seen)}\nListed: ${localDate(album.last_seen)}\nDetails checked: ${localDate(album.updated_at)}` +
    (album.gone_since ? `\nNo longer listed since ${localDate(album.gone_since)}` : "");
  const reason = compileCatalogueSearch(state.view.filters.search).reason(album);
  workEl("preview-match").textContent = reason ? "Search match: " + reason : "";
  const run = state.index?.sellers.find(s => s.id === album.seller)?.run;
  workEl("preview-run").textContent = run ? `Seller last successful run: ${localDate(run.last_success)}` +
    (run.failed_ids?.length ? ` · ${run.failed_ids.length} detail failures awaiting retry` : "") : "Seller run status not recorded";
  const cover = workEl("preview-cover");
  cover.hidden = !album.thumb;
  workEl("preview-no-cover").hidden = Boolean(album.thumb);
  cover.onerror = () => { cover.hidden = true; workEl("preview-no-cover").hidden = false; };
  if (album.thumb) cover.src = `data/${album.thumb}?v=${thumbVersion(album)}`;
  else cover.removeAttribute("src");
  showPreviewLinks(album);
  const changes = [...(album.history || [])].reverse();
  workEl("preview-history").replaceChildren();
  for (const change of changes) {
    const li = document.createElement("li");
    li.textContent = `${localDate(change.at)}: ${change.field}: ${JSON.stringify(change.before)} → ${JSON.stringify(change.after)}`;
    workEl("preview-history").appendChild(li);
  }
  if (!changes.length) workEl("preview-history").textContent = "No changes recorded.";
  previewInitial = {
    collection: userStore.get("collection", album.id) || "",
    tags: (userStore.get("tags", album.id) || []).join(", "),
    notes: userStore.get("note", album.id) || "",
  };
  for (const [field, value] of /** @type {[keyof typeof previewInitial, string][]} */ (Object.entries(previewInitial))) workEl(/** @type {`preview-${keyof typeof previewInitial}`} */ ("preview-" + field)).value = value;
  workEl("preview-save-status").textContent = "";
  workEl("preview-action-status").textContent = "";
  workEl("preview-action-status").hidden = true;
  workEl("preview-similar").open = false;
  workEl("similar-results").replaceChildren();
  updatePreviewFavourite();
  updatePreviewShortlist(true);
  previewAvailability(false);
  return true;
}

/** @param {string} id */
function openPreview(id) {
  const targetId = resolvePreview(id)?.id || id;
  previewIds = state.filtered.map(a => a.id);
  previewAt = previewIds.indexOf(targetId);
  if (previewAt < 0) return;
  previewReturnId = targetId;
  if (!showPreview()) return;
  document.body.classList.add("preview-open");
  preview.showModal();
}
function previewDirty() {
  return /** @type {[keyof typeof previewInitial, string][]} */ (Object.entries(previewInitial)).some(([key, value]) => workEl(/** @type {`preview-${keyof typeof previewInitial}`} */ ("preview-" + key)).value !== value);
}
function leavePreviewAllowed() {
  return !previewDirty() || window.confirm("Discard the unsaved notes, tags or collection for this album?");
}
/** @param {number} delta */
function stepPreview(delta) {
  if (!previewReady()) return;
  const next = previewAt + delta;
  if (next < 0 || next >= previewIds.length || !leavePreviewAllowed()) return;
  const album = resolvePreview(previewIds[next]);
  if (!album) { workEl("preview-save-status").textContent = "That listing is unavailable. The displayed album remains selected."; return; }
  previewAt = next;
  previewIds[next] = album.id;
  previewReturnId = album.id;
  showPreview(album);
}
workEl("preview-prev").addEventListener("click", () => stepPreview(-1));
workEl("preview-next").addEventListener("click", () => stepPreview(1));
workEl("preview-close").addEventListener("click", () => { if (leavePreviewAllowed()) preview.close(); });
preview.addEventListener("cancel", event => { if (!leavePreviewAllowed()) event.preventDefault(); });
preview.addEventListener("close", () => {
  document.body.classList.remove("preview-open");
  workEl("preview-action-status").textContent = "";
  workEl("preview-action-status").hidden = true;
  const at = state.filtered.findIndex(a => a.id === previewReturnId);
  if (at >= 0) moveCursor(at);
  else els.grid.focus({preventScroll: true});
});
preview.addEventListener("keydown", event => {
  if (event.ctrlKey || event.metaKey || event.altKey) return;
  if (/** @type {Element} */ (event.target).closest("input, textarea, select")) return;
  if (["ArrowLeft", "ArrowRight"].includes(event.key)) {
    event.preventDefault(); stepPreview(event.key === "ArrowLeft" ? -1 : 1);
  }
});
workEl("preview-favourite").addEventListener("click", event => {
  if (previewReady() && previewSnapshot) toggleFavorite(previewSnapshot.id, /** @type {Element} */ (event.currentTarget));
});
workEl("preview-shortlist").addEventListener("click", event => {
  if (previewReady() && previewSnapshot) toggleShortlist(previewSnapshot.id, /** @type {Element} */ (event.currentTarget));
});
workEl("preview-shortlist-save").addEventListener("click", () => {
  if (!previewReady() || !previewSnapshot) return;
  const saved = saveShortlistFields(previewSnapshot.id, {
    size: workEl("preview-shortlist-size").value,
    colour: workEl("preview-shortlist-colour").value,
    note: workEl("preview-shortlist-note").value,
  });
  workEl("preview-shortlist-status").textContent = saved
    ? "Shortlist details saved."
    : "Shortlist details could not be saved.";
});
workEl("preview-retain").addEventListener("click", () => {
  workEl("preview-save-status").textContent = "Draft retained in this open preview. Keep this page open until you transfer or discard it; the saved-work backup includes persisted fields only.";
});
/** @param {boolean} transfer */
function resolvePreviewDraft(transfer) {
  if (!previewSnapshot) return;
  const latest = resolvePreview(previewSnapshot.id);
  if (!latest || (transfer && latest.id === previewSnapshot.id)) return;
  if (!transfer && !leavePreviewAllowed()) return;
  const draft = /** @type {Partial<Record<'collection' | 'tags' | 'notes', string>>} */ ({});
  for (const [field, kind] of /** @type {const} */ ([["collection", "collection"], ["tags", "tags"], ["notes", "note"]])) {
    const value = workEl(/** @type {`preview-${keyof typeof previewInitial}`} */ ("preview-" + field)).value;
    if (transfer && value !== previewInitial[field] && !userStore.has(kind, latest.id)) draft[field] = value;
  }
  previewIds = previewIds.map(id => id === previewSnapshot?.id ? latest.id : id);
  previewReturnId = latest.id;
  showPreview(latest);
  for (const [field, value] of /** @type {[keyof typeof previewInitial, string][]} */ (Object.entries(draft)))
    workEl(/** @type {`preview-${keyof typeof previewInitial}`} */ ("preview-" + field)).value = value;
  workEl("preview-save-status").textContent = transfer ? "Draft staged on the latest listing. Existing saved fields were kept. Review the fields before saving." : "Draft discarded; the confirmed latest listing is displayed.";
  workEl("preview-notes").focus();
}
workEl("preview-transfer").addEventListener("click", () => resolvePreviewDraft(true));
workEl("preview-discard").addEventListener("click", () => resolvePreviewDraft(false));
workEl("preview-form").addEventListener("submit", event => {
  event.preventDefault();
  if (!previewReady() || !previewSnapshot) return;
  const id = previewSnapshot.id;
  const tags = workEl("preview-tags").value.split(",").map(s => s.trim()).filter(Boolean);
  if (tags.length > 20 || tags.some(tag => tag.length > 50)) {
    workEl("preview-save-status").textContent = "Use at most 20 tags, each no longer than 50 characters.";
    return;
  }
  // Only write edited fields. An unchanged open form must not revert a
  // different field saved by another tab after this preview opened.
  for (const [field, kind] of /** @type {const} */ ([["collection", "collection"], ["tags", "tags"], ["notes", "note"]])) {
    const value = workEl(/** @type {`preview-${keyof typeof previewInitial}`} */ ("preview-" + field)).value;
    if (value === previewInitial[field]) continue;
    if (!userStore.set(kind, id, field === "tags" ? [...new Set(tags)] : value)) {
      workEl("preview-save-status").textContent = "Save incomplete. Keep this form open and export a backup.";
      return;
    }
    previewInitial[field] = value;
  }
  refreshSavedState();
  workEl("preview-save-status").textContent = "Saved.";
});

refreshSavedState(false);
if (!filtersFromURL(location.search)) {
  history.replaceState(null, "", filtersToURL(state.view.storedFilters));
}

workEl("preview-similar").addEventListener("toggle", () => {
  if (!workEl("preview-similar").open) return;
  if (!previewReady() || !previewSnapshot) { workEl("preview-similar").open = false; return; }
  const album = previewSnapshot;
  const dismissed = new Set(userStore.load().filter(e => e.kind === "dismiss" && e.value === true &&
    e.id.startsWith(album.id + ":")).map(e => e.id.slice(album.id.length + 1)));
  const matches = similarCovers(state.albums, album, dismissed);
  const list = workEl("similar-results");
  list.replaceChildren();
  if (!matches.length) list.textContent = album.cover_hash ? "No suggestions within the similarity threshold." :
    "No cover fingerprint. Run lamp prepare-covers for your cached catalogue to try this experiment.";
  for (const match of matches) {
    const li = document.createElement("li");
    const view = document.createElement("button");
    view.type = "button";
    view.textContent = `${match.album.title} (${match.distance}/64 differences)`;
    view.addEventListener("click", () => {
      if (!previewReady()) return;
      if (!leavePreviewAllowed()) return;
      const target = resolvePreview(match.album.id);
      if (!target) { workEl("preview-save-status").textContent = "That suggestion is unavailable."; return; }
      if (!previewIds.includes(target.id)) previewIds.push(target.id);
      previewAt = previewIds.indexOf(target.id);
      previewReturnId = target.id;
      showPreview(target);
      workEl("preview-close").focus();
    });
    const dismiss = document.createElement("button");
    dismiss.type = "button";
    dismiss.textContent = "Dismiss";
    dismiss.setAttribute("aria-label", "Dismiss similar cover: " + match.album.title);
    dismiss.addEventListener("click", () => {
      if (!previewReady() || previewSnapshot?.id !== album.id) return;
      if (userStore.set("dismiss", album.id + ":" + match.album.id, true)) {
        li.remove(); /** @type {HTMLElement} */ (workEl("preview-similar").querySelector("summary")).focus();
      }
    });
    li.append(view, dismiss); list.appendChild(li);
  }
});
window.addEventListener("beforeunload", event => {
  if (preview.open && previewDirty()) { event.preventDefault(); event.returnValue = ""; }
});

// Preferences: the agent purchase links open on, external lookups and the
// operator's price rate. Validated by prefs.js; the typed values stay in the
// fields while a half-entered currency and rate convert nothing.
const prefEls = { agent: workEl("pref-agent"), lookups: workEl("pref-lookups"),
  currency: workEl("pref-currency"), rate: workEl("pref-rate") };
for (const agent of ACTIVE_AGENTS) prefEls.agent.add(new Option(agent.name, agent.id));
function showPrefs() {
  prefEls.agent.value = state.prefs.agent;
  prefEls.lookups.checked = state.prefs.lookups;
  prefEls.currency.value = state.prefs.currency;
  prefEls.rate.value = state.prefs.rate;
}
// Cards are cached across renders and would keep their old destinations
// and figures; the forced pass rebuilds the window.
function applyPrefs() {
  renderGrid(true);
  if (preview.open) showPreviewLinks(previewAlbum());
  if (shortlistDialog.open) renderShortlistDialog();
}
showPrefs();
{
  const saved = /** @type {Record<string, unknown> | null} */ (storedPrefs(window));
  if (saved?.agent && saved.agent !== state.prefs.agent) {
    actionStatus(`The saved agent "${saved.agent}" is no longer offered; purchase links open on the marketplace until you choose another.`);
  }
}
for (const el of Object.values(prefEls)) el.addEventListener("change", () => {
  const wanted = { agent: prefEls.agent.value, lookups: prefEls.lookups.checked,
    currency: prefEls.currency.value.trim().toUpperCase(), rate: prefEls.rate.value.trim() };
  state.prefs = normalizePrefs(wanted);
  const saved = writePrefs(window, state.prefs);
  actionStatus(!saved ? "Applied for this session only: the browser refused to save the preference."
    : (wanted.currency || wanted.rate) && !state.prefs.currency
    ? "Prices stay in CNY until both a 3-letter currency code and a rate above 0 (up to 6 decimals) are entered." : "");
  applyPrefs();
});

// A community spreadsheet names items by marketplace or agent link. Match
// them to albums by item and tag the matches, so the Tag filter shows which
// of the followed sellers carry them. Tags are additive, unlike a collection,
// so nothing already saved is overwritten.
workEl("import-sheet").addEventListener("change", async event => {
  const file = /** @type {FileList} */ (/** @type {HTMLInputElement} */ (event.target).files)[0];
  if (!file) return;
  try {
    if (file.size > 20 * 1024 * 1024) throw new Error("Spreadsheet exceeds 20 MB.");
    const sheet = sheetItemKeys(await file.text());
    const tag = "sheet:" + file.name.replace(/\.[^.]*$/, "").slice(0, 44);
    const { matched, found } = matchSheet(state.albums, sheet.keys);
    let full = 0;
    for (const album of matched) {
      const tags = userStore.get("tags", album.id) || [];
      if (tags.includes(tag)) continue;
      if (tags.length >= 20) { full++; continue; }
      if (!userStore.set("tags", album.id, [...tags, tag])) throw new Error("Storage write failed.");
    }
    refreshSavedState(false);
    actionStatus(`${sheet.rows} rows, ${sheet.urls} links, ${sheet.keys.size} distinct items. ` +
      `${found.size} items matched ${matched.length} albums, tagged ${tag}; ${sheet.keys.size - found.size} items are not in the loaded catalogue` +
      (full ? `; ${full} matched albums already carry 20 tags` : "") + `. Catalogue: ${catalogueAvailability()}.`);
    if (matched.length) useFilters({...state.view.filters, tag});
    else applyFilters();
  } catch (error) { savedDataError(/** @type {Error} */ (error).message); }
  finally { /** @type {HTMLInputElement | HTMLSelectElement} */ (event.target).value = ""; }
});


// Local haul shortlist dialog and keyboard shortcut reference.
const shortlistDialog = workEl("shortlist-dialog");
const shortcutsDialog = workEl("shortcuts-dialog");

/** @returns {Map<string, import("./controller-types.js").ViewerAlbum>} */
function albumsByIdForShortlist() {
  /** @type {Map<string, import("./controller-types.js").ViewerAlbum>} */
  const byId = new Map();
  for (const album of state.albums) byId.set(album.id, album);
  for (const [oldId, targetId] of Object.entries(state.albumAliases || {})) {
    const target = byId.get(targetId);
    if (target && !byId.has(oldId)) byId.set(oldId, target);
  }
  return byId;
}

/**
 * @param {string} albumId
 * @param {import("./shortlist.js").ShortlistFields} fields
 */
function saveShortlistFields(albumId, fields) {
  const next = updateShortlistItem(state.shortlist, albumId, fields);
  if (!writeShortlist(window, next)) {
    savedDataError("Shortlist could not be saved. Browser storage may be full or blocked.");
    return false;
  }
  state.shortlist = next;
  if (typeof updateShortlistCount === "function") updateShortlistCount();
  if (typeof syncAllShortlistCards === "function") syncAllShortlistCards();
  syncShortlistUI();
  return true;
}

/** @param {string} text @param {string} formatLabel */
async function copyShortlistText(text, formatLabel) {
  const statusEl = workEl("shortlist-status");
  if (!navigator.clipboard || typeof navigator.clipboard.writeText !== "function") {
    statusEl.textContent = `Clipboard copy is unavailable in this browser; use Export ${formatLabel} instead.`;
    return;
  }
  try {
    await navigator.clipboard.writeText(text);
    statusEl.textContent = `Copied shortlist as ${formatLabel}.`;
  } catch {
    statusEl.textContent = `Could not copy shortlist to clipboard; use Export ${formatLabel} instead.`;
  }
}

/**
 * @param {import("./shortlist.js").ShortlistExportRecord} record
 * @param {string} labelText
 * @param {"size" | "colour" | "note"} fieldName
 * @param {string} value
 * @param {number} maxLength
 * @param {string} placeholder
 */
function buildShortlistField(record, labelText, fieldName, value, maxLength, placeholder) {
  const label = document.createElement("label");
  label.textContent = labelText;
  const input = document.createElement("input");
  input.type = "text";
  input.value = value;
  input.maxLength = maxLength;
  input.placeholder = placeholder;
  input.autocomplete = "off";
  input.addEventListener("change", () => {
    const ok = saveShortlistFields(record.id, { [fieldName]: input.value });
    workEl("shortlist-status").textContent = ok
      ? `Saved ${labelText.toLowerCase()} for ${record.title || `Album ${record.id}`}.`
      : "Shortlist could not be saved.";
  });
  label.appendChild(input);
  return label;
}

/** @param {import("./shortlist.js").ShortlistExportRecord} record */
function buildShortlistRowHead(record) {
  const head = document.createElement("div");
  head.className = "shortlist-item-head";
  if (!record.loaded) {
    const span = document.createElement("span");
    span.textContent = `Album ${record.id} (not loaded)`;
    head.appendChild(span);
    return head;
  }
  const openBtn = document.createElement("button");
  openBtn.type = "button";
  openBtn.className = "shortlist-item-title";
  openBtn.textContent = record.title || `Album ${record.id}`;
  openBtn.addEventListener("click", () => {
    shortlistDialog.close();
    const targetId = resolvePreview(record.id)?.id || record.id;
    if (!state.filtered.some(a => a.id === targetId)) {
      useFilters({ ...defaultFilters(), shortlistOnly: true });
      saveFilterURL();
    }
    openPreview(targetId);
  });
  head.appendChild(openBtn);
  return head;
}

/** @param {import("./shortlist.js").ShortlistExportRecord} record */
function buildShortlistRowMeta(record) {
  const meta = document.createElement("div");
  meta.className = "shortlist-item-meta fig";
  const priceText = record.price_cny !== null
    ? (record.estimated_price
      ? `${formatPrice(record.price_cny)} (${record.estimated_price})`
      : formatPrice(record.price_cny))
    : "Price unknown";
  const metaText = document.createElement("span");
  const prefShortlist = record.seller && state.preferredSellers?.has(record.seller) ? " (preferred)" : "";
  const sellerLabel = record.seller ? `${record.seller}${prefShortlist}` : "Seller not loaded";
  metaText.textContent = `${sellerLabel} · ${priceText}`;
  meta.appendChild(metaText);
  if (record.album_url) addLink(meta, "Yupoo album", record.album_url);
  if (record.agent_url) addLink(meta, "Shopping agent", record.agent_url);
  else if (record.purchase_url) addLink(meta, "Purchase link", record.purchase_url);
  return meta;
}

/** @param {import("./shortlist.js").ShortlistExportRecord} record */
function buildShortlistRow(record) {
  const li = document.createElement("li");
  li.className = "shortlist-item";
  const fields = document.createElement("div");
  fields.className = "shortlist-item-fields";
  fields.append(
    buildShortlistField(record, "Size", "size", record.size, SHORTLIST_LIMITS.maxSize, "L, 43..."),
    buildShortlistField(record, "Colour", "colour", record.colour, SHORTLIST_LIMITS.maxColour, "Black..."),
    buildShortlistField(record, "Note", "note", record.note, SHORTLIST_LIMITS.maxNote, "Optional note"),
  );
  const removeBtn = document.createElement("button");
  removeBtn.type = "button";
  removeBtn.textContent = "Remove";
  removeBtn.setAttribute("aria-label", `Remove ${record.title || `Album ${record.id}`} from shortlist`);
  removeBtn.addEventListener("click", () => {
    const next = removeShortlistItem(state.shortlist, record.id);
    if (!writeShortlist(window, next)) {
      savedDataError("Shortlist could not be saved. Browser storage may be full or blocked.");
      return;
    }
    state.shortlist = next;
    if (typeof updateShortlistCount === "function") updateShortlistCount();
    if (typeof syncAllShortlistCards === "function") syncAllShortlistCards();
    if (state.view.filters.shortlistOnly) applyFilters();
    workEl("shortlist-status").textContent = `Removed ${record.title || `Album ${record.id}`}.`;
    syncShortlistUI();
  });
  fields.appendChild(removeBtn);
  li.append(buildShortlistRowHead(record), buildShortlistRowMeta(record), fields);
  return li;
}

function renderShortlistDialog() {
  const byId = albumsByIdForShortlist();
  const records = buildShortlistExportRecords(state.shortlist, byId, state.prefs);
  const totals = shortlistTotals(records, state.prefs);
  const noun = totals.count === 1 ? "album" : "albums";
  const priceSummary = totals.totalCny > 0
    ? ` · Est. ${formatPrice(totals.totalCny)}${totals.estimatedTotal ? ` (${totals.estimatedTotal})` : ""}`
    : "";
  workEl("shortlist-summary").textContent = `${totals.count.toLocaleString()} ${noun}${priceSummary}`;
  workEl("shortlist-empty").hidden = records.length > 0;
  const empty = records.length === 0;
  for (const id of /** @type {const} */ ([
    "shortlist-export-json", "shortlist-export-md", "shortlist-copy-md", "shortlist-copy-json", "shortlist-clear",
  ])) {
    workEl(id).disabled = empty;
  }
  workEl("shortlist-filter-btn").textContent = state.view.filters.shortlistOnly
    ? "Show all albums"
    : "Filter grid to shortlist";
  const list = workEl("shortlist-list");
  list.replaceChildren(...records.map(buildShortlistRow));
}
function syncShortlistUI() {
  if (preview.open) updatePreviewShortlist();
  if (shortlistDialog.open) renderShortlistDialog();
}

function openShortlistDialog() {
  workEl("shortlist-status").textContent = "";
  renderShortlistDialog();
  document.body.classList.add("preview-open");
  shortlistDialog.showModal();
}

workEl("shortlist-toggle").addEventListener("click", openShortlistDialog);
workEl("shortlist-close").addEventListener("click", () => shortlistDialog.close());
shortlistDialog.addEventListener("close", () => {
  if (!preview.open) document.body.classList.remove("preview-open");
  workEl("shortlist-toggle").focus();
});

workEl("shortlist-filter-btn").addEventListener("click", () => {
  const nextOnly = !state.view.filters.shortlistOnly;
  useFilters({ ...state.view.filters, shortlistOnly: nextOnly });
  saveFilterURL();
  shortlistDialog.close();
  actionStatus(nextOnly ? "Showing shortlisted albums." : "Showing all matching albums.");
});

workEl("shortlist-export-json").addEventListener("click", () => {
  const json = shortlistToJson(state.shortlist, albumsByIdForShortlist(), state.prefs);
  downloadWork(json, "diogenes-shortlist.json", "application/json");
  workEl("shortlist-status").textContent = `Exported ${state.shortlist.size} shortlisted items as JSON.`;
});

workEl("shortlist-export-md").addEventListener("click", () => {
  const md = shortlistToMarkdown(state.shortlist, albumsByIdForShortlist(), state.prefs);
  downloadWork(md, "diogenes-shortlist.md", "text/markdown;charset=utf-8");
  workEl("shortlist-status").textContent = `Exported ${state.shortlist.size} shortlisted items as Markdown.`;
});

workEl("shortlist-copy-md").addEventListener("click", () => {
  const md = shortlistToMarkdown(state.shortlist, albumsByIdForShortlist(), state.prefs);
  void copyShortlistText(md, "Markdown");
});

workEl("shortlist-copy-json").addEventListener("click", () => {
  const json = shortlistToJson(state.shortlist, albumsByIdForShortlist(), state.prefs);
  void copyShortlistText(json, "JSON");
});

workEl("shortlist-clear").addEventListener("click", () => {
  if (!state.shortlist.size) return;
  if (!window.confirm("Remove all items from your shortlist?")) return;
  const empty = new Map();
  if (!writeShortlist(window, empty)) {
    savedDataError("Shortlist could not be saved. Browser storage may be full or blocked.");
    return;
  }
  state.shortlist = empty;
  if (typeof updateShortlistCount === "function") updateShortlistCount();
  if (typeof syncAllShortlistCards === "function") syncAllShortlistCards();
  if (state.view.filters.shortlistOnly) applyFilters();
  workEl("shortlist-status").textContent = "Shortlist cleared.";
  syncShortlistUI();
});

function renderShortcutsDialog() {
  const list = workEl("shortcuts-list");
  list.replaceChildren();
  for (const hint of SHORTCUT_HINTS) {
    const dt = document.createElement("dt");
    const kbd = document.createElement("kbd");
    kbd.textContent = hint.keys;
    dt.appendChild(kbd);
    const dd = document.createElement("dd");
    dd.textContent = hint.description;
    list.append(dt, dd);
  }
}

/** @type {HTMLElement | null} */
let shortcutsReturnFocus = null;

function toggleShortcutsDialog() {
  if (shortcutsDialog.open) {
    shortcutsDialog.close();
    return;
  }
  shortcutsReturnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  renderShortcutsDialog();
  shortcutsDialog.showModal();
}

workEl("shortcuts-toggle").addEventListener("click", toggleShortcutsDialog);
workEl("shortcuts-close").addEventListener("click", () => shortcutsDialog.close());
shortcutsDialog.addEventListener("close", () => {
  if (shortcutsReturnFocus && shortcutsReturnFocus.isConnected) {
    shortcutsReturnFocus.focus({ preventScroll: true });
  }
  shortcutsReturnFocus = null;
});
