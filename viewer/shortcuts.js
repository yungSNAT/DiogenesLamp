// Copyright 2026 yungSNAT
// SPDX-License-Identifier: Apache-2.0

"use strict";

// Viewer keyboard shortcut resolution and reference list.
// Pure logic without DOM access so it runs directly under node.

/**
 * @typedef {{
 *   key: string,
 *   ctrlKey?: boolean,
 *   metaKey?: boolean,
 *   altKey?: boolean,
 * }} ShortcutEvent
 * @typedef {{
 *   tagName?: string | null,
 *   isContentEditable?: boolean,
 * }} ShortcutTarget
 * @typedef {{
 *   target?: ShortcutTarget | null,
 *   previewOpen?: boolean,
 *   shortlistOpen?: boolean,
 *   shortcutsOpen?: boolean,
 *   shopsOpen?: boolean,
 *   hasTargetAlbum?: boolean,
 * }} ShortcutContext
 * @typedef {"focus-search" | "toggle-shortcuts" | "toggle-shortlist" | "open-album" | "open-agent" | null} ShortcutAction
 */

/** @type {readonly {keys: string, description: string}[]} */
const SHORTCUT_HINTS = Object.freeze([
  { keys: "/", description: "Focus search" },
  { keys: "s", description: "Toggle shortlist on focused or open album" },
  { keys: "o", description: "Open Yupoo album link in a new tab" },
  { keys: "a", description: "Open shopping-agent link in a new tab (if present)" },
  { keys: "f", description: "Toggle favourite on focused card" },
  { keys: "p", description: "Open preview for focused card" },
  { keys: "Enter", description: "Open focused card's Yupoo album" },
  { keys: "Arrows", description: "Move across cards, or step through preview" },
  { keys: "?", description: "Toggle this keyboard shortcut hint" },
  { keys: "Esc", description: "Close open dialog, panel or group view" },
]);

const EDITABLE_TAGS = new Set(["input", "textarea", "select"]);

/** @type {ShortcutContext} */
const EMPTY_CONTEXT = Object.freeze({});

/**
 * True when typing should stay in a form field or editable surface.
 * @param {ShortcutTarget | null | undefined} target
 * @returns {boolean}
 */
function isEditableTarget(target) {
  if (!target) return false;
  if (target.isContentEditable === true) return true;
  const tag = (target.tagName || "").toLowerCase();
  return EDITABLE_TAGS.has(tag);
}

/**
 * Decide which global viewer shortcut, if any, a keydown event triggers.
 * Browser chords (Ctrl, Meta, Alt) and editable controls always keep priority.
 * @param {ShortcutEvent} event
 * @param {ShortcutContext} [context]
 * @returns {ShortcutAction}
 */
function resolveShortcutAction(event, context = EMPTY_CONTEXT) {
  if (!event || event.ctrlKey || event.metaKey || event.altKey) return null;
  if (isEditableTarget(context.target)) return null;
  const key = event.key;
  if (key === "?") return "toggle-shortcuts";
  if (context.shortcutsOpen || context.shortlistOpen) return null;
  if (key === "/") {
    return context.previewOpen ? null : "focus-search";
  }
  if (context.shopsOpen || !context.hasTargetAlbum) return null;
  const lower = key.toLowerCase();
  if (lower === "s") return "toggle-shortlist";
  if (lower === "o") return "open-album";
  if (lower === "a") return "open-agent";
  return null;
}

if (typeof module !== "undefined") {
  module.exports = { SHORTCUT_HINTS, isEditableTarget, resolveShortcutAction };
}
