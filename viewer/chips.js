// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";

/**
 * @typedef {{id: string, label: string}} MaterialChip
 * @typedef {{id: string, name: string, album_count: number, preferred?: boolean}} ChipSeller
 * @typedef {{traits?: readonly string[], gone_since?: string}} MaterialAlbum
 */

// Pure helpers for material filter chips and preferred-seller presentation.
// No DOM access so the same module runs in the browser and under node tests.

/**
 * Derive ordered 1-click material chips from kinds.js WORDS without
 * duplicating the material gloss table.
 * @param {readonly {facet: string, en: string}[]} words
 * @returns {MaterialChip[]}
 */
function materialChipList(words) {
  /** @type {MaterialChip[]} */
  const chips = [];
  for (const word of words) {
    if (word.facet !== "material" || !word.en) continue;
    chips.push({
      id: word.en,
      label: word.en[0].toUpperCase() + word.en.slice(1),
    });
  }
  return chips;
}

/**
 * Count active (non-gone) albums matching each material trait.
 * @param {readonly MaterialAlbum[]} albums
 * @param {readonly string[]} materialIds
 * @returns {Map<string, number>}
 */
function countMaterials(albums, materialIds) {
  /** @type {Map<string, number>} */
  const counts = new Map();
  for (const id of materialIds) counts.set(id, 0);
  for (const album of albums) {
    if (album.gone_since || !album.traits?.length) continue;
    for (const trait of album.traits) {
      const current = counts.get(trait);
      if (current !== undefined) counts.set(trait, current + 1);
    }
  }
  return counts;
}

/**
 * 1-click material chip toggle: clicking the active material clears it.
 * @param {string} currentMaterial
 * @param {string} clickedMaterial
 * @returns {string}
 */
function toggleMaterialFilter(currentMaterial, clickedMaterial) {
  return currentMaterial === clickedMaterial ? "" : clickedMaterial;
}

/**
 * Build the set of preferred seller IDs from the catalogue index.
 * @param {readonly {id?: string, preferred?: boolean}[] | null | undefined} sellers
 * @returns {Set<string>}
 */
function preferredSellerSet(sellers) {
  /** @type {Set<string>} */
  const out = new Set();
  if (!Array.isArray(sellers)) return out;
  for (const seller of sellers) {
    if (seller && seller.preferred && typeof seller.id === "string" && seller.id) {
      out.add(seller.id);
    }
  }
  return out;
}

/**
 * Order preferred sellers first in the seller dropdown while preserving
 * relative order within each group.
 * @template {ChipSeller} T
 * @param {readonly T[]} sellers
 * @returns {T[]}
 */
function sortSellersForFilter(sellers) {
  const preferred = [];
  const rest = [];
  for (const seller of sellers) {
    if (seller.preferred) preferred.push(seller);
    else rest.push(seller);
  }
  return [...preferred, ...rest];
}

/**
 * Label for a seller entry in the seller dropdown.
 * @param {ChipSeller} seller
 * @returns {string}
 */
function sellerOptionLabel(seller) {
  const tag = seller.preferred ? " [preferred]" : "";
  return `${seller.name}${tag} (${seller.album_count})`;
}

if (typeof module !== "undefined") {
  module.exports = {
    materialChipList,
    countMaterials,
    toggleMaterialFilter,
    preferredSellerSet,
    sortSellersForFilter,
    sellerOptionLabel,
  };
}
