// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";

// Local haul shortlist persisted in browser storage with zero external requests.
// Nothing here touches the DOM, so the whole module runs directly under node.

/**
 * @typedef {{
 *   id: string,
 *   size: string,
 *   colour: string,
 *   note: string,
 *   added_at: string,
 * }} ShortlistItem
 * @typedef {{
 *   size?: unknown,
 *   colour?: unknown,
 *   note?: unknown,
 * }} ShortlistFields
 * @typedef {Map<string, ShortlistItem>} ShortlistMap
 * @typedef {{
 *   id: string,
 *   seller?: string,
 *   title?: string,
 *   price?: number | null,
 *   album_url?: string,
 *   links?: readonly {platform: string, url: string}[],
 * }} ShortlistAlbum
 * @typedef {{
 *   id: string,
 *   title: string,
 *   seller: string,
 *   price_cny: number | null,
 *   estimated_price: string,
 *   size: string,
 *   colour: string,
 *   note: string,
 *   album_url: string,
 *   purchase_url: string,
 *   agent_url: string,
 *   added_at: string,
 *   loaded: boolean,
 * }} ShortlistExportRecord
 * @typedef {{
 *   count: number,
 *   pricedCount: number,
 *   totalCny: number,
 *   estimatedTotal: string,
 * }} ShortlistTotals
 */

const SHORTLIST_HELPERS = typeof module !== "undefined"
  ? {
    ...require("./filters.js"),
    ...require("./dedup.js"),
    ...require("./agents.js"),
    ...require("./prefs.js"),
  }
  : {
    readStored,
    writeStored,
    formatPrice,
    marketplaceItem,
    agentLink,
    agentFor,
    convertedPrice,
  };

const SHORTLIST_STORAGE_KEY = "lamp.shortlist.v1";

const SHORTLIST_LIMITS = {
  maxItems: 5000,
  maxId: 200,
  maxSize: 40,
  maxColour: 60,
  maxNote: 500,
  maxDate: 40,
};

/**
 * Strip ASCII control characters (except spaces) and cap length.
 * @param {unknown} value
 * @param {number} maxLen
 * @param {boolean} [allowNewlines]
 * @returns {string}
 */
function cleanShortlistText(value, maxLen, allowNewlines = false) {
  if (typeof value !== "string") return "";
  const cleaned = allowNewlines
    ? value.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "")
    : value.replace(/[\u0000-\u001f\u007f]/g, " ");
  return cleaned.trim().slice(0, maxLen);
}

/**
 * Allow only explicit http: and https: URLs.
 * @param {unknown} value
 * @returns {string}
 */
function safeHttpUrl(value) {
  if (typeof value !== "string") return "";
  const trimmed = value.trim();
  if (!trimmed) return "";
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol === "http:" || parsed.protocol === "https:") return parsed.href;
  } catch {
    return "";
  }
  return "";
}

/**
 * @param {unknown} raw
 * @returns {ShortlistItem | null}
 */
function normalizeShortlistItem(raw) {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const obj = /** @type {Record<string, unknown>} */ (raw);
  const id = cleanShortlistText(obj.id, SHORTLIST_LIMITS.maxId);
  if (!id) return null;
  return {
    id,
    size: cleanShortlistText(obj.size, SHORTLIST_LIMITS.maxSize),
    colour: cleanShortlistText(obj.colour, SHORTLIST_LIMITS.maxColour),
    note: cleanShortlistText(obj.note, SHORTLIST_LIMITS.maxNote, true),
    added_at: cleanShortlistText(obj.added_at, SHORTLIST_LIMITS.maxDate),
  };
}

/**
 * @param {unknown} raw
 * @returns {ShortlistMap}
 */
function normalizeShortlist(raw) {
  /** @type {ShortlistMap} */
  const out = new Map();
  if (!raw || typeof raw !== "object") return out;
  const list = Array.isArray(raw)
    ? raw
    : Array.isArray(/** @type {Record<string, unknown>} */ (raw).items)
      ? /** @type {unknown[]} */ (/** @type {Record<string, unknown>} */ (raw).items)
      : [];
  for (const entry of list) {
    if (out.size >= SHORTLIST_LIMITS.maxItems) break;
    const item = normalizeShortlistItem(entry);
    if (item && !out.has(item.id)) out.set(item.id, item);
  }
  return out;
}

/**
 * @param {ShortlistMap} shortlist
 */
function serializeShortlist(shortlist) {
  return {
    version: 1,
    items: [...shortlist.values()].slice(0, SHORTLIST_LIMITS.maxItems),
  };
}

/**
 * @param {import("./filters.js").StorageHost} host
 * @returns {ShortlistMap}
 */
function readShortlist(host) {
  const raw = SHORTLIST_HELPERS.readStored(host, SHORTLIST_STORAGE_KEY);
  if (!raw) return new Map();
  try {
    return normalizeShortlist(JSON.parse(raw));
  } catch {
    return new Map();
  }
}

/**
 * @param {import("./filters.js").StorageHost} host
 * @param {ShortlistMap} shortlist
 * @returns {boolean}
 */
function writeShortlist(host, shortlist) {
  return SHORTLIST_HELPERS.writeStored(
    host,
    SHORTLIST_STORAGE_KEY,
    JSON.stringify(serializeShortlist(shortlist)),
  );
}

/**
 * @param {ShortlistMap} shortlist
 * @param {string} albumId
 * @param {ShortlistFields} [fields]
 * @param {string} [nowIso]
 * @returns {{shortlist: ShortlistMap, added: boolean}}
 */
function toggleShortlistItem(shortlist, albumId, fields = {}, nowIso = new Date().toISOString()) {
  const id = cleanShortlistText(albumId, SHORTLIST_LIMITS.maxId);
  const next = new Map(shortlist);
  if (!id) return { shortlist: next, added: false };
  if (next.has(id)) {
    next.delete(id);
    return { shortlist: next, added: false };
  }
  if (next.size >= SHORTLIST_LIMITS.maxItems) {
    return { shortlist: next, added: false };
  }
  next.set(id, {
    id,
    size: cleanShortlistText(fields.size, SHORTLIST_LIMITS.maxSize),
    colour: cleanShortlistText(fields.colour, SHORTLIST_LIMITS.maxColour),
    note: cleanShortlistText(fields.note, SHORTLIST_LIMITS.maxNote, true),
    added_at: cleanShortlistText(nowIso, SHORTLIST_LIMITS.maxDate),
  });
  return { shortlist: next, added: true };
}

/**
 * Add or update an album's optional shortlist fields without disturbing order.
 * @param {ShortlistMap} shortlist
 * @param {string} albumId
 * @param {ShortlistFields} fields
 * @param {string} [nowIso]
 * @returns {ShortlistMap}
 */
function updateShortlistItem(shortlist, albumId, fields, nowIso = new Date().toISOString()) {
  const id = cleanShortlistText(albumId, SHORTLIST_LIMITS.maxId);
  const next = new Map(shortlist);
  if (!id) return next;
  const existing = next.get(id);
  if (!existing && next.size >= SHORTLIST_LIMITS.maxItems) return next;
  next.set(id, {
    id,
    size: fields.size !== undefined
      ? cleanShortlistText(fields.size, SHORTLIST_LIMITS.maxSize)
      : (existing ? existing.size : ""),
    colour: fields.colour !== undefined
      ? cleanShortlistText(fields.colour, SHORTLIST_LIMITS.maxColour)
      : (existing ? existing.colour : ""),
    note: fields.note !== undefined
      ? cleanShortlistText(fields.note, SHORTLIST_LIMITS.maxNote, true)
      : (existing ? existing.note : ""),
    added_at: existing?.added_at || cleanShortlistText(nowIso, SHORTLIST_LIMITS.maxDate),
  });
  return next;
}

/**
 * @param {ShortlistMap} shortlist
 * @param {string} albumId
 * @returns {ShortlistMap}
 */
function removeShortlistItem(shortlist, albumId) {
  const next = new Map(shortlist);
  next.delete(albumId);
  return next;
}

/**
 * First safe HTTP(S) purchase link on an album.
 * @param {ShortlistAlbum | null | undefined} album
 * @returns {string}
 */
function primaryPurchaseUrl(album) {
  if (!album || !Array.isArray(album.links)) return "";
  for (const link of album.links) {
    const safe = safeHttpUrl(link?.url);
    if (safe) return safe;
  }
  return "";
}

/**
 * Resolve the primary shopping-agent URL for an album, if one exists.
 * Rebuilds from the configured agent preference first; falls back to an
 * explicit shopping-agent link already recorded on the album.
 * @param {ShortlistAlbum | null | undefined} album
 * @param {string} [agentId]
 * @returns {string}
 */
function primaryAgentUrl(album, agentId = "") {
  if (!album || !Array.isArray(album.links)) return "";
  if (agentId) {
    for (const link of album.links) {
      if (!link?.url) continue;
      const rebuilt = SHORTLIST_HELPERS.agentLink(agentId, SHORTLIST_HELPERS.marketplaceItem(link.url));
      const safe = safeHttpUrl(rebuilt);
      if (safe) return safe;
    }
  }
  for (const link of album.links) {
    if (!link?.url) continue;
    const safe = safeHttpUrl(link.url);
    if (!safe) continue;
    if (SHORTLIST_HELPERS.agentFor(new URL(safe).hostname.toLowerCase())) return safe;
  }
  return "";
}

/**
 * @param {ShortlistMap} shortlist
 * @param {Map<string, ShortlistAlbum>} albumsById
 * @param {import("./prefs.js").Prefs} [prefs]
 * @returns {ShortlistExportRecord[]}
 */
function buildShortlistExportRecords(shortlist, albumsById, prefs = { agent: "", lookups: false, currency: "", rate: "" }) {
  /** @type {ShortlistExportRecord[]} */
  const records = [];
  for (const item of shortlist.values()) {
    const album = albumsById.get(item.id) || null;
    const price = album && typeof album.price === "number" && Number.isFinite(album.price) && album.price > 0
      ? album.price
      : null;
    records.push({
      id: item.id,
      title: album?.title || "",
      seller: album?.seller || "",
      price_cny: price,
      estimated_price: price ? SHORTLIST_HELPERS.convertedPrice(price, prefs) : "",
      size: item.size,
      colour: item.colour,
      note: item.note,
      album_url: safeHttpUrl(album?.album_url),
      purchase_url: primaryPurchaseUrl(album),
      agent_url: primaryAgentUrl(album, prefs.agent),
      added_at: item.added_at,
      loaded: Boolean(album),
    });
  }
  return records;
}

/**
 * @param {readonly ShortlistExportRecord[]} records
 * @param {import("./prefs.js").Prefs} [prefs]
 * @returns {ShortlistTotals}
 */
function shortlistTotals(records, prefs = { agent: "", lookups: false, currency: "", rate: "" }) {
  let pricedCount = 0;
  let totalCny = 0;
  for (const record of records) {
    if (record.price_cny !== null) {
      pricedCount += 1;
      totalCny += record.price_cny;
    }
  }
  totalCny = Math.round(totalCny * 100) / 100;
  return {
    count: records.length,
    pricedCount,
    totalCny,
    estimatedTotal: totalCny > 0 ? SHORTLIST_HELPERS.convertedPrice(totalCny, prefs) : "",
  };
}

/**
 * @param {ShortlistMap} shortlist
 * @param {Map<string, ShortlistAlbum>} albumsById
 * @param {import("./prefs.js").Prefs} [prefs]
 * @param {string} [exportedAt]
 * @returns {string}
 */
function shortlistToJson(
  shortlist,
  albumsById,
  prefs = { agent: "", lookups: false, currency: "", rate: "" },
  exportedAt = new Date().toISOString(),
) {
  const items = buildShortlistExportRecords(shortlist, albumsById, prefs);
  const totals = shortlistTotals(items, prefs);
  const payload = {
    format: "diogeneslamp-shortlist",
    version: 1,
    exported_at: exportedAt,
    agent: prefs.agent || "",
    count: totals.count,
    priced_count: totals.pricedCount,
    total_cny: totals.totalCny,
    estimated_total: totals.estimatedTotal,
    items,
  };
  return JSON.stringify(payload, null, 2) + "\n";
}

/**
 * Escape text for safe inclusion inside a Markdown table cell.
 * @param {string} value
 * @returns {string}
 */
function escapeMarkdownCell(value) {
  return value
    .replace(/\\/g, "\\\\")
    .replace(/\|/g, "\\|")
    .replace(/\[/g, "\\[")
    .replace(/\]/g, "\\]")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\r?\n/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * @param {string} label
 * @param {string} url
 * @returns {string}
 */
function markdownLink(label, url) {
  const safe = safeHttpUrl(url);
  if (!safe) return "";
  const escapedUrl = safe.replace(/\(/g, "%28").replace(/\)/g, "%29");
  return `[${label}](${escapedUrl})`;
}

/**
 * @param {ShortlistMap} shortlist
 * @param {Map<string, ShortlistAlbum>} albumsById
 * @param {import("./prefs.js").Prefs} [prefs]
 * @returns {string}
 */
function shortlistToMarkdown(
  shortlist,
  albumsById,
  prefs = { agent: "", lookups: false, currency: "", rate: "" },
) {
  const records = buildShortlistExportRecords(shortlist, albumsById, prefs);
  const totals = shortlistTotals(records, prefs);
  const noun = totals.count === 1 ? "item" : "items";
  const summaryParts = [`${totals.count} ${noun}`];
  if (totals.totalCny > 0) {
    const cnyText = SHORTLIST_HELPERS.formatPrice(totals.totalCny);
    summaryParts.push(
      totals.estimatedTotal ? `est. ${cnyText} (${totals.estimatedTotal})` : `est. ${cnyText}`,
    );
  }
  const lines = [`# DiogenesLamp Shortlist (${summaryParts.join(", ")})`, ""];
  if (!records.length) {
    lines.push("No items shortlisted.", "");
    return lines.join("\n");
  }
  lines.push("| Item | Seller | Price | Size | Colour | Note | Links |");
  lines.push("| --- | --- | --- | --- | --- | --- | --- |");
  for (const record of records) {
    const title = record.title
      ? escapeMarkdownCell(record.title)
      : `Album ${escapeMarkdownCell(record.id)} (not loaded)`;
    const seller = record.seller ? escapeMarkdownCell(record.seller) : "-";
    const price = record.price_cny !== null
      ? escapeMarkdownCell(
        record.estimated_price
          ? `${SHORTLIST_HELPERS.formatPrice(record.price_cny)} (${record.estimated_price})`
          : SHORTLIST_HELPERS.formatPrice(record.price_cny),
      )
      : "-";
    const size = record.size ? escapeMarkdownCell(record.size) : "-";
    const colour = record.colour ? escapeMarkdownCell(record.colour) : "-";
    const note = record.note ? escapeMarkdownCell(record.note) : "-";
    const linkParts = [
      markdownLink("Yupoo", record.album_url),
      markdownLink("Agent", record.agent_url),
      !record.agent_url ? markdownLink("Purchase", record.purchase_url) : "",
    ].filter(Boolean);
    const linksCell = linkParts.length ? linkParts.join(" · ") : "-";
    lines.push(`| ${title} | ${seller} | ${price} | ${size} | ${colour} | ${note} | ${linksCell} |`);
  }
  lines.push("");
  return lines.join("\n");
}

if (typeof module !== "undefined") {
  module.exports = {
    SHORTLIST_STORAGE_KEY,
    SHORTLIST_LIMITS,
    safeHttpUrl,
    normalizeShortlistItem,
    normalizeShortlist,
    serializeShortlist,
    readShortlist,
    writeShortlist,
    toggleShortlistItem,
    updateShortlistItem,
    removeShortlistItem,
    primaryPurchaseUrl,
    primaryAgentUrl,
    buildShortlistExportRecords,
    shortlistTotals,
    shortlistToJson,
    escapeMarkdownCell,
    shortlistToMarkdown,
  };
}
