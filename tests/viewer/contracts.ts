// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

// Compile only. Runtime behaviour is covered by the adjacent .test.cjs files.
import { defaultFilters, normalizeFilters } from "../../viewer/filters.js";
import { createViewState, nextViewState, selectCatalogue } from "../../viewer/view-state.js";
import { createUserStore } from "../../viewer/user-store.js";
import type { ViewerAlbum } from "../../viewer/controller-types.js";

const view = createViewState(normalizeFilters({ hideDups: "invalid stored value" }));
nextViewState(view, { type: "filter", filters: defaultFilters() });
nextViewState(view, { type: "cursor", index: 1, count: 4 });
// @ts-expect-error Cursor positions must be numbers.
nextViewState(view, { type: "cursor", index: "1", count: 4 });
// @ts-expect-error Opening a group needs its identity and scroll position.
nextViewState(view, { type: "open-group" });
// @ts-expect-error Typed filter updates cannot store strings as booleans.
nextViewState(view, { type: "filter", filters: { ...defaultFilters(), hideDups: "yes" } });
// @ts-expect-error The reducer event name must match an implemented transition.
nextViewState(view, { type: "clear-filter" });

const selected = selectCatalogue(view, {
  albums: [], favorites: new Set<string>(), bannedKeys: new Set<string>(),
});
// @ts-expect-error Duplicate counts are numeric.
selected.dupCounts.set("group", "2");

const store = createUserStore(window);
const tags: string[] | undefined = store.get("tags", "123");
const review: number | undefined = store.get("review", "seller");
// @ts-expect-error Saved tags are a list, not a single string.
const tagText: string = store.get("tags", "123");
// @ts-expect-error Review timestamps are numeric.
store.section("review").set("seller", "yesterday");
// @ts-expect-error Saved kinds must match the storage schema.
store.get("favourite", "123");
document.getElementById("preview-cover").src = "data/thumbs/fixture/123.jpg";
// @ts-expect-error The preview cover is an image, not an input control.
document.getElementById("preview-cover").value = "123";
// @ts-expect-error A dialog's open state is boolean.
document.getElementById("album-preview").open = "yes";
// @ts-expect-error Unknown IDs retain the nullable DOM lookup contract.
document.getElementById("misspelt-control").focus();
declare const album: ViewerAlbum;
// @ts-expect-error Marketplace links must contain textual URLs.
album.links.push({platform: "fixture", url: 123});
void tags;
void review;
void tagText;

import {
  readShortlist,
  toggleShortlistItem,
  updateShortlistItem,
  shortlistToJson,
  shortlistToMarkdown,
  primaryAgentUrl,
} from "../../viewer/shortlist.js";
import { resolveShortcutAction, SHORTCUT_HINTS } from "../../viewer/shortcuts.js";
import {
  materialChipList,
  countMaterials,
  toggleMaterialFilter,
  preferredSellerSet,
  sortSellersForFilter,
  sellerOptionLabel,
} from "../../viewer/chips.js";

const emptyShortlist = readShortlist({ localStorage: { getItem: () => null, setItem: () => {} } });
const toggled = toggleShortlistItem(emptyShortlist, "100", { size: "L", colour: "Black", note: "Check cuffs" });
const updated = updateShortlistItem(toggled.shortlist, "100", { note: "Updated note" });
const exportMap = new Map([["100", { id: "100", seller: "one", title: "Jacket", price: 320, album_url: "https://one.x.yupoo.com/albums/100", links: [] }]]);
const exportedJson: string = shortlistToJson(updated, exportMap);
const exportedMd: string = shortlistToMarkdown(updated, exportMap);
const resolvedAgent: string = primaryAgentUrl(exportMap.get("100"), "superbuy");
const shortcutAction = resolveShortcutAction({ key: "s" }, { hasTargetAlbum: true });
const shortcutCount: number = SHORTCUT_HINTS.length;
const chipList = materialChipList([{ facet: "material", en: "wool" }]);
const matCounts = countMaterials([{ traits: ["wool"] }], ["wool"]);
const toggledMat: string = toggleMaterialFilter("wool", "wool");
const prefSet = preferredSellerSet([{ id: "fav", preferred: true }]);
const sortedSellers = sortSellersForFilter([{ id: "fav", name: "Fav", album_count: 5, preferred: true }]);
const sellerLbl: string = sellerOptionLabel(sortedSellers[0]);
void [exportedJson, exportedMd, resolvedAgent, shortcutAction, shortcutCount,
  chipList, matCounts, toggledMat, prefSet, sellerLbl];
