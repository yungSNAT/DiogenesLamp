// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { KINDS, KIND_IDS, UNSORTED, kindOf } = require("../../viewer/kinds.js");

const CASES = [
  ["Nike Dunk Low Panda", "shoes"], ["Jordan 4 Black Cat", "shoes"], ["Yeezy Slide Onyx", "shoes"],
  ["Balenciaga Track LED", UNSORTED], ["Moncler Maya down jacket", "jackets"], ["Stone Island hoodie", "hoodies"],
  ["Essentials hoodie and sweatpants set", "sets"], ["Louis Vuitton belt", "accessories"],
  ["Watch strap replacement", "accessories"], ["Shoelaces 3 pairs", "accessories"], ["Rolex Submariner watch", "watches"],
  ["Cuban chain necklace", "jewellery"], ["Jewellery storage box", "accessories"], ["Short-sleeve shirt", "tees"],
  ["Nike shorts", "shorts"], ["Denim jeans", "pants"], ["Goyard tote bag", "bags"], ["Top quality version", UNSORTED],
  ["短袖T恤", "tees"], ["羽绒服", "jackets"], ["运动鞋", "shoes"], ["表带", "accessories"], ["首饰盒", "accessories"],
  ["短裤", "shorts"], ["泳裤", "shorts"], ["牛仔裤", "pants"], ["Swim shorts", "shorts"],
  ["", UNSORTED], [null, UNSORTED], ["https://weidian.com/shoes-shop", UNSORTED],
  ["ＡＦ１ white", "shoes"],
];

test("kindOf reads a garment kind from the title and abstains otherwise", () => {
  for (const [title, kind] of CASES) assert.equal(kindOf(title), kind, String(title));
});

test("the table is well formed: unique ids, lowercase tokens, unsorted last", () => {
  assert.equal(new Set(KIND_IDS).size, KIND_IDS.length);
  assert.equal(KIND_IDS.at(-1), UNSORTED);
  for (const kind of KINDS) {
    assert.ok(kind.latin.length && kind.label, kind.id);
    assert.ok(kind.latin.every(token => token === token.toLowerCase()), kind.id);
  }
  for (const [title] of CASES) assert.ok(KIND_IDS.includes(kindOf(title)));
});

test("link removal preserves adjacent title words and handles long domain candidates", () => {
  for (const title of ["watchhttp://example.com/shoes", "watchhttps://example.com/shoes", "watch example.com/shoes"]) {
    assert.equal(kindOf(title), "watches", title);
  }
  assert.equal(kindOf("a".repeat(50000) + "! watch"), "watches");
  assert.equal(kindOf("https://" + "a".repeat(50000) + ".com/watch"), UNSORTED);
});

test("T恤 reads as a tee: tokens are matched against the lowercased title", () => {
  assert.equal(kindOf("T恤"), "tees");
  for (const kind of KINDS) assert.ok(kind.cjk.every(token => token === token.toLowerCase()), kind.id);
});

const { WORDS, readWords } = require("../../viewer/kinds.js");
const read = title => readWords(title.normalize("NFKC").toLowerCase());

test("the word table is well formed: unique English, lowercase forms, no single-character tokens", () => {
  assert.equal(new Set(WORDS.map(word => word.en)).size, WORDS.length);
  // A colour, fit or material form names one word, or a Latin title could read as either.
  const traitForms = WORDS.filter(word => word.facet !== "garment").flatMap(word => [word.en, ...word.latin]);
  assert.equal(new Set(traitForms).size, traitForms.length);
  for (const word of WORDS) {
    for (const form of [word.en, ...word.latin, ...word.cjk]) assert.equal(form, form.toLowerCase(), form);
    // One character is too loose: 包 (bag) is also in 包邮, free postage.
    assert.ok(word.cjk.length && word.cjk.every(token => [...token].length > 1), word.en);
  }
});


test("titles are read longest token first, links ignored, Latin traits as whole words", () => {
  assert.deepEqual(read("粉红色T恤"), {gloss: ["t-shirt", "pink"], traits: ["pink"]});
  assert.deepEqual(read("深蓝色 米白色"), {gloss: ["navy", "beige"], traits: ["navy", "beige"]});
  assert.deepEqual(read("牛仔裤"), {gloss: ["jeans", "denim"], traits: ["denim"]});
  assert.deepEqual(read("Off-White hoodie"), {gloss: [], traits: []});
  assert.deepEqual(read("Grey oversized hoodie, gray cuffs"), {gloss: [], traits: ["grey", "loose"]});
  // "down" is read only in Chinese: in English it is as often a direction.
  assert.deepEqual(read("Upside Down tee, button down shirt"), {gloss: [], traits: []});
  assert.deepEqual(read("羽绒服"), {gloss: ["puffer", "down"], traits: ["down"]});
  assert.deepEqual(read("卫衣 https://shop.example.com/black-黑色"), {gloss: ["sweatshirt"], traits: []});
  assert.equal(read("Plain title").gloss, read("").traits, "titles with no words share one empty list");
});
