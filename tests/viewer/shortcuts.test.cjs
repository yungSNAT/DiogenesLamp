// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");

const { SHORTCUT_HINTS, isEditableTarget, resolveShortcutAction } = require("../../viewer/shortcuts.js");

test("isEditableTarget recognises inputs, textareas, selects and contentEditable", () => {
  assert.equal(isEditableTarget({ tagName: "INPUT" }), true);
  assert.equal(isEditableTarget({ tagName: "textarea" }), true);
  assert.equal(isEditableTarget({ tagName: "SELECT" }), true);
  assert.equal(isEditableTarget({ tagName: "DIV", isContentEditable: true }), true);
  assert.equal(isEditableTarget({ tagName: "DIV", isContentEditable: false }), false);
  assert.equal(isEditableTarget({ tagName: "BUTTON" }), false);
  assert.equal(isEditableTarget({ tagName: "BODY" }), false);
  assert.equal(isEditableTarget(null), false);
});

test("resolveShortcutAction maps s, o, a, / and ? when focus is outside form controls", () => {
  const gridCtx = { target: { tagName: "DIV" }, hasTargetAlbum: true };
  assert.equal(resolveShortcutAction({ key: "s" }, gridCtx), "toggle-shortlist");
  assert.equal(resolveShortcutAction({ key: "S" }, gridCtx), "toggle-shortlist");
  assert.equal(resolveShortcutAction({ key: "o" }, gridCtx), "open-album");
  assert.equal(resolveShortcutAction({ key: "O" }, gridCtx), "open-album");
  assert.equal(resolveShortcutAction({ key: "a" }, gridCtx), "open-agent");
  assert.equal(resolveShortcutAction({ key: "A" }, gridCtx), "open-agent");
  assert.equal(resolveShortcutAction({ key: "/" }, gridCtx), "focus-search");
  assert.equal(resolveShortcutAction({ key: "?" }, gridCtx), "toggle-shortcuts");
  assert.equal(resolveShortcutAction({ key: "x" }, gridCtx), null);
});

test("resolveShortcutAction ignores editable targets and browser modifier chords", () => {
  for (const tagName of ["INPUT", "TEXTAREA", "SELECT"]) {
    for (const k of ["s", "o", "a", "/", "?"]) {
      assert.equal(
        resolveShortcutAction({ key: k }, { target: { tagName }, hasTargetAlbum: true }),
        null,
        `${k} must be ignored inside ${tagName}`,
      );
    }
  }
  assert.equal(
    resolveShortcutAction({ key: "s" }, { target: { tagName: "DIV", isContentEditable: true }, hasTargetAlbum: true }),
    null,
  );
  for (const mod of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }]) {
    for (const k of ["s", "o", "a", "/", "?"]) {
      assert.equal(
        resolveShortcutAction({ key: k, ...mod }, { target: { tagName: "DIV" }, hasTargetAlbum: true }),
        null,
      );
    }
  }
});

test("resolveShortcutAction respects open dialogs, shops panel and missing target album", () => {
  // Open preview keeps s, o, a and ? active, but blocks /
  const previewCtx = { target: { tagName: "DIALOG" }, previewOpen: true, hasTargetAlbum: true };
  assert.equal(resolveShortcutAction({ key: "s" }, previewCtx), "toggle-shortlist");
  assert.equal(resolveShortcutAction({ key: "o" }, previewCtx), "open-album");
  assert.equal(resolveShortcutAction({ key: "a" }, previewCtx), "open-agent");
  assert.equal(resolveShortcutAction({ key: "?" }, previewCtx), "toggle-shortcuts");
  assert.equal(resolveShortcutAction({ key: "/" }, previewCtx), null);

  // Open shortcut hint allows ? to toggle closed, blocks everything else
  const hintCtx = { target: { tagName: "DIALOG" }, shortcutsOpen: true, hasTargetAlbum: true };
  assert.equal(resolveShortcutAction({ key: "?" }, hintCtx), "toggle-shortcuts");
  assert.equal(resolveShortcutAction({ key: "s" }, hintCtx), null);
  assert.equal(resolveShortcutAction({ key: "/" }, hintCtx), null);

  // Open shortlist dialog allows ? to open help, blocks grid shortcuts
  const shortlistCtx = { target: { tagName: "DIALOG" }, shortlistOpen: true, hasTargetAlbum: true };
  assert.equal(resolveShortcutAction({ key: "?" }, shortlistCtx), "toggle-shortcuts");
  assert.equal(resolveShortcutAction({ key: "s" }, shortlistCtx), null);
  assert.equal(resolveShortcutAction({ key: "o" }, shortlistCtx), null);

  // Shops panel open blocks album shortcuts but still allows / and ?
  const shopsCtx = { target: { tagName: "ASIDE" }, shopsOpen: true, hasTargetAlbum: true };
  assert.equal(resolveShortcutAction({ key: "s" }, shopsCtx), null);
  assert.equal(resolveShortcutAction({ key: "/" }, shopsCtx), "focus-search");
  assert.equal(resolveShortcutAction({ key: "?" }, shopsCtx), "toggle-shortcuts");

  // Empty catalogue blocks album shortcuts
  const emptyCtx = { target: { tagName: "BODY" }, hasTargetAlbum: false };
  assert.equal(resolveShortcutAction({ key: "s" }, emptyCtx), null);
  assert.equal(resolveShortcutAction({ key: "o" }, emptyCtx), null);
  assert.equal(resolveShortcutAction({ key: "a" }, emptyCtx), null);
  assert.equal(resolveShortcutAction({ key: "/" }, emptyCtx), "focus-search");
});

test("SHORTCUT_HINTS documents every required viewer shortcut", () => {
  const documented = new Set(SHORTCUT_HINTS.map(entry => entry.keys));
  for (const required of ["/", "s", "o", "a", "?"]) {
    assert.equal(documented.has(required), true, `missing hint for ${required}`);
  }
  for (const entry of SHORTCUT_HINTS) {
    assert.equal(entry.description.includes("\u2014"), false, "hints must not contain em dashes");
  }
});
