// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";

// Experimental: a garment kind read from the album title, so the corpus can
// be browsed by kind across sellers whose own Yupoo categories differ or
// are empty. A keyword table, nothing cleverer: the first kind in table
// order with a token in the title wins, and a title with none is "unsorted"
// rather than guessed. Latin tokens match on word boundaries, CJK tokens
// anywhere. ponytail: precision is unmeasured on real catalogues; the
// filter is labelled experimental and an album's own category filter is
// untouched. Measure precision on a real catalogue before dropping the label.

/** @typedef {{id: string, label: string, latin: string[], cjk: string[]}} Kind */

/** @type {Kind[]} */
const KINDS = [
  { id: "sets", label: "Sets", latin: ["set", "sets", "tracksuit", "tracksuits", "suit", "suits", "outfit"],
    cjk: ["套装", "两件套", "三件套"] },
  { id: "shoes", label: "Shoes", latin: ["shoe", "shoes", "sneaker", "sneakers", "boot", "boots", "slide",
    "slides", "sandal", "sandals", "loafer", "loafers", "runner", "runners", "trainer", "trainers", "mule",
    "mules", "jordan", "dunk", "dunks", "yeezy", "af1", "sb", "gel", "kayano", "samba", "gazelle", "clog", "clogs"],
    cjk: ["鞋", "靴", "拖鞋", "凉鞋"] },
  { id: "jackets", label: "Jackets and coats", latin: ["jacket", "jackets", "coat", "coats", "parka", "puffer",
    "puffers", "windbreaker", "blazer", "vest", "gilet", "bomber", "trench", "anorak", "down", "fleece", "overshirt"],
    cjk: ["外套", "夹克", "羽绒", "大衣", "风衣", "马甲", "棉服", "冲锋衣"] },
  { id: "hoodies", label: "Hoodies and sweaters", latin: ["hoodie", "hoodies", "hoody", "sweatshirt", "sweatshirts",
    "crewneck", "sweater", "sweaters", "knit", "knitwear", "cardigan", "pullover", "jumper", "zip-up", "zipup"],
    cjk: ["卫衣", "毛衣", "针织", "开衫", "连帽"] },
  { id: "tees", label: "Tees and shirts", latin: ["tee", "tees", "t-shirt", "tshirt", "t-shirts", "shirt", "shirts",
    "polo", "polos", "tank", "jersey", "jerseys", "longsleeve", "long-sleeve"],
    cjk: ["短袖", "t恤", "衬衫", "长袖", "球衣", "背心"] },
  // Shorts before pants: 短裤 and 泳裤 contain 裤, and the specific token must win.
  { id: "shorts", label: "Shorts", latin: ["shorts", "swim", "trunks"], cjk: ["短裤", "泳裤"] },
  { id: "pants", label: "Pants", latin: ["pants", "jeans", "denim", "trousers", "joggers", "sweatpants", "cargo",
    "cargos", "chinos", "leggings", "tights"],
    cjk: ["裤", "牛仔"] },
  { id: "bags", label: "Bags and wallets", latin: ["bag", "bags", "backpack", "backpacks", "tote", "totes", "wallet",
    "wallets", "cardholder", "card holder", "purse", "pouch", "crossbody", "handbag", "duffle", "duffel", "clutch",
    "briefcase", "luggage", "suitcase"],
    cjk: ["包", "钱包", "卡包", "背包", "箱"] },
  { id: "watches", label: "Watches", latin: ["watch", "watches", "chronograph"], cjk: ["手表", "腕表"] },
  { id: "jewellery", label: "Jewellery", latin: ["ring", "rings", "necklace", "necklaces", "bracelet", "bracelets",
    "earring", "earrings", "chain", "chains", "pendant", "brooch", "cufflinks", "jewelry", "jewellery"],
    cjk: ["项链", "手链", "戒指", "耳环", "耳钉", "首饰", "吊坠"] },
  { id: "accessories", label: "Accessories", latin: ["hat", "hats", "cap", "caps", "beanie", "beanies", "belt", "belts",
    "scarf", "scarves", "socks", "glasses", "sunglasses", "eyewear", "gloves", "tie", "ties", "keychain", "bandana",
    "balaclava", "umbrella", "perfume", "airpods", "headphones", "case", "cover"],
    cjk: ["帽", "皮带", "腰带", "围巾", "袜", "眼镜", "墨镜", "手套", "领带", "香水", "耳机"] },
];
const UNSORTED = "unsorted";
const KIND_IDS = [...KINDS.map(kind => kind.id), UNSORTED];

// A part or container named after the thing it serves ("watch strap",
// "shoelaces", "jewellery box") is an accessory, whatever else the title
// says, so these are checked before the table.
/** @type {Kind} */
const PARTS = { id: "accessories", label: "Accessories", latin: ["strap", "straps", "laces", "shoelaces", "insole",
  "insoles", "box", "boxes", "organizer", "organiser", "cleaner", "protector"],
  cjk: ["表带", "鞋带", "鞋垫", "盒", "收纳"] };

/** @param {string} token */
const escapeToken = token => token.replace(/[.*+?^${}()|[\]\\-]/g, "\\$&");

/** @param {Kind} kind */
const latinPattern = kind => new RegExp(
  "(?:^|[^a-z0-9])(?:" + kind.latin.map(escapeToken).join("|") +
  ")(?![a-z0-9])", "i");
const LATIN_PATTERNS = KINDS.map(latinPattern);
const PARTS_PATTERN = latinPattern(PARTS);

/** @param {Kind} kind @param {RegExp} pattern @param {string} text */
const mentions = (kind, pattern, text) => pattern.test(text) || kind.cjk.some(token => text.includes(token));

// A link needs a slash, so a title with none skips the pattern. Blanking
// keeps every other character where it was, for wordRanges.
/** @param {string} lowered an NFKC-normalised, lowercased title */
const withoutLinks = lowered => lowered.includes("/")
  ? lowered.replace(/(?:https?:\/\/|(?<![a-z0-9.-]))[a-z0-9.-]+\.[a-z]{2,}\/\S*/g, link => " ".repeat(link.length))
  : lowered;

/**
 * The kind id for a title, "unsorted" when no token matches. Links in the
 * title are ignored, so a shop path never names a kind.
 * @param {string | null | undefined} title
 */
function kindOf(title) {
  const text = withoutLinks((title || "").normalize("NFKC").toLowerCase());
  if (!text.trim()) return UNSORTED;
  if (mentions(PARTS, PARTS_PATTERN, text)) return PARTS.id;
  for (let i = 0; i < KINDS.length; i++) {
    if (mentions(KINDS[i], LATIN_PATTERNS[i], text)) return KINDS[i].id;
  }
  return UNSORTED;
}

// Experimental, like the kinds: the English a buyer types, each word with
// the Chinese a seller writes for it. A word's English and Latin forms in a
// query also find its Chinese tokens, never the reverse, because loose
// Latin title words such as "down" or "sb" must not stand for Chinese ones.
// The Chinese tokens in a title print as an English gloss beside it.
// Colour, fit and material words are filters; for those, the English and
// Latin forms are also read from Latin titles as whole words, where a
// hyphen joins rather than separates ("off-white" names no colour).
// Single Chinese characters are too loose to list: 包 is in 包邮, free
// postage. No brand or model names: aliases for those belong in the
// operator's own notes, not in the repository.

/** @typedef {"garment" | "colour" | "fit" | "material"} Facet */
/** @typedef {{facet: Facet, en: string, latin: string[], cjk: string[]}} Word */

/** @param {Facet} facet @param {[string, string[], string[]][]} rows @returns {Word[]} */
const wordRows = (facet, rows) => rows.map(([en, latin, cjk]) => ({ facet, en, latin, cjk }));

/** @type {Word[]} */
const WORDS = [
  ...wordRows("garment", [
    ["sweatshirt", ["sweatshirts", "hoodie", "hoodies", "hoody", "crewneck"], ["卫衣"]],
    ["sweater", ["sweaters", "jumper", "jumpers", "pullover"], ["毛衣"]],
    ["knit", ["knits", "knitwear", "knitted"], ["针织"]],
    ["cardigan", ["cardigans"], ["开衫"]],
    ["jacket", ["jackets"], ["外套", "夹克"]],
    ["coat", ["coats", "overcoat"], ["大衣"]],
    ["puffer", ["puffers"], ["羽绒服", "面包服"]],
    ["padded jacket", ["padded"], ["棉服", "棉衣"]],
    ["trench coat", ["trench"], ["风衣"]],
    ["windbreaker", ["windbreakers"], ["冲锋衣"]],
    ["vest", ["vests", "gilet"], ["马甲"]],
    ["t-shirt", ["tee", "tees", "t-shirts", "tshirt"], ["t恤"]],
    ["shirt", ["shirts"], ["衬衫", "衬衣"]],
    ["tank top", ["tank", "singlet"], ["背心"]],
    ["jersey", ["jerseys"], ["球衣"]],
    ["dress", ["dresses"], ["连衣裙"]],
    ["skirt", ["skirts"], ["半身裙", "短裙"]],
    ["shorts", [], ["短裤"]],
    ["swim shorts", ["swim", "trunks"], ["泳裤"]],
    ["pants", ["trousers"], ["裤子", "长裤", "休闲裤", "西裤"]],
    ["jeans", ["pants"], ["牛仔裤"]],
    ["sweatpants", ["joggers", "pants"], ["卫裤", "运动裤"]],
    ["cargo pants", ["cargo", "cargos", "pants"], ["工装裤"]],
    ["set", ["sets", "tracksuit", "tracksuits"], ["套装"]],
    ["sneakers", ["sneaker", "trainers", "shoes"], ["运动鞋", "板鞋", "球鞋", "跑鞋"]],
    ["boots", ["boot", "shoes"], ["靴子", "短靴", "马丁靴"]],
    ["slides", ["slide", "slipper", "slippers"], ["拖鞋"]],
    ["sandals", ["sandal"], ["凉鞋"]],
    ["bag", ["bags", "handbag", "handbags"], ["包包", "手提包", "斜挎包", "单肩包"]],
    ["backpack", ["backpacks", "bag", "bags"], ["双肩包", "背包"]],
    ["wallet", ["wallets", "cardholder"], ["钱包", "卡包"]],
    ["cap", ["caps", "hat", "hats"], ["棒球帽", "鸭舌帽"]],
    ["beanie", ["beanies", "hat", "hats"], ["毛线帽", "针织帽"]],
    ["hat", ["hats"], ["帽子"]],
    ["belt", ["belts"], ["皮带", "腰带"]],
    ["scarf", ["scarves"], ["围巾"]],
    ["socks", ["sock"], ["袜子"]],
    ["sunglasses", [], ["墨镜", "太阳镜"]],
    ["watch", ["watches"], ["手表", "腕表"]],
    ["necklace", ["necklaces"], ["项链"]],
    ["bracelet", ["bracelets"], ["手链", "手镯"]],
    ["ring", ["rings"], ["戒指"]],
    ["earrings", ["earring"], ["耳环", "耳钉"]],
    ["hooded", ["hoodie", "hoodies", "hoody"], ["连帽"]],
    ["short sleeve", ["tee", "tees", "t-shirt", "t-shirts", "tshirt", "short-sleeve"], ["短袖"]],
    ["long sleeve", ["longsleeve", "long-sleeve"], ["长袖"]],
  ]),
  ...wordRows("colour", [
    ["black", [], ["黑色", "纯黑", "全黑"]],
    ["white", [], ["白色", "纯白", "全白"]],
    ["grey", ["gray"], ["灰色"]],
    ["red", [], ["红色"]],
    ["blue", [], ["蓝色"]],
    ["navy", [], ["藏青", "深蓝", "海军蓝"]],
    ["green", [], ["绿色"]],
    ["yellow", [], ["黄色"]],
    ["orange", [], ["橙色", "橘色"]],
    ["pink", [], ["粉色", "粉红"]],
    ["purple", [], ["紫色"]],
    ["brown", [], ["棕色", "咖啡色", "褐色"]],
    ["beige", ["cream"], ["米色", "米白", "杏色"]],
    ["khaki", [], ["卡其"]],
    ["silver", [], ["银色"]],
    ["gold", [], ["金色"]],
  ]),
  ...wordRows("fit", [
    ["loose", ["oversized", "oversize", "baggy", "relaxed"], ["宽松"]],
    ["slim", ["slim-fit", "skinny", "fitted"], ["修身", "紧身"]],
    ["straight", ["straight-leg"], ["直筒"]],
    ["wide leg", ["wide-leg"], ["阔腿"]],
    ["cropped", ["crop"], ["短款"]],
    ["drop shoulder", ["drop-shoulder"], ["落肩"]],
  ]),
  ...wordRows("material", [
    ["cotton", [], ["纯棉", "全棉", "棉质"]],
    ["wool", ["woollen", "woolen", "merino"], ["羊毛", "毛呢"]],
    ["cashmere", [], ["羊绒"]],
    ["leather", [], ["真皮", "皮革", "牛皮", "羊皮"]],
    ["suede", [], ["麂皮"]],
    ["denim", [], ["牛仔", "牛仔裤"]],
    ["linen", [], ["亚麻"]],
    ["silk", [], ["真丝", "丝绸", "桑蚕丝"]],
    ["nylon", [], ["尼龙"]],
    ["polyester", [], ["涤纶", "聚酯"]],
    ["down", [], ["羽绒", "羽绒服"]],
    ["fleece", [], ["抓绒", "摇粒绒"]],
  ]),
];

/** @param {Word} word */
const wordForms = word => [word.en, ...word.latin];
/** @type {Map<string, string[]>} */
const WORDS_BY_FORM = new Map();
for (const word of WORDS) {
  for (const form of wordForms(word)) WORDS_BY_FORM.set(form, [...WORDS_BY_FORM.get(form) || [], word.en]);
}
const LONGEST_FORM = Math.max(...[...WORDS_BY_FORM.keys()].map(form => form.split(" ").length));

/** @typedef {{literals: string[], words: readonly string[]}} SearchTerm */

/**
 * A query as terms. A term matches text holding every one of its literals,
 * or a title read as one of its words. Query words that together make a
 * form, such as "wide leg", are one term.
 * @param {string} query
 * @returns {SearchTerm[]}
 */
function searchTerms(query) {
  const parts = query.toLowerCase().split(/\s+/).filter(Boolean);
  /** @type {SearchTerm[]} */
  const terms = [];
  for (let at = 0; at < parts.length;) {
    let size = Math.min(LONGEST_FORM, parts.length - at);
    while (size > 1 && !WORDS_BY_FORM.has(parts.slice(at, at + size).join(" "))) size--;
    const literals = parts.slice(at, at + size);
    terms.push({ literals, words: WORDS_BY_FORM.get(literals.join(" ")) || NO_WORDS });
    at += size;
  }
  return terms;
}

// "down" in a Latin title is as often a direction ("upside down") as the
// filling, so only its Chinese reads as the material.
const TRAIT_BY_FORM = new Map(WORDS.filter(word => word.facet !== "garment")
  .flatMap(word => wordForms(word).filter(form => form !== "down").map(form => [form, word.en])));
const LATIN_TRAIT = new RegExp("(?<![a-z0-9-])(" + [...TRAIT_BY_FORM.keys()]
  .sort((a, b) => b.length - a.length).map(escapeToken).join("|") + ")(?![a-z0-9-])", "g");
// A title is read left to right, taking the longest token at each place,
// so 粉红色 is pink and not red as well. A token that means 2 words, such
// as 牛仔裤 (jeans, denim), is listed under both.
/** @type {Map<string, Word[]>} */
const WORDS_BY_TOKEN = new Map();
/** @type {Map<string, [string, Word[]][]>} */
const TOKENS_BY_FIRST = new Map();
for (const word of WORDS) {
  for (const token of word.cjk) WORDS_BY_TOKEN.set(token, [...WORDS_BY_TOKEN.get(token) || [], word]);
}
for (const entry of [...WORDS_BY_TOKEN].sort((a, b) => b[0].length - a[0].length)) {
  TOKENS_BY_FIRST.set(entry[0][0], [...TOKENS_BY_FIRST.get(entry[0][0]) || [], entry]);
}
const HAN_CHARACTER = /\p{Script=Han}/u;
/** @type {readonly string[]} */
const NO_WORDS = Object.freeze([]);
/** @type {readonly [string, Word[]][]} */
const NO_TOKENS = Object.freeze([]);

/**
 * Calls `visit` for each table token in a title, with its words and where
 * it starts.
 * @param {string} text a lowercased title with its links blanked
 * @param {(words: Word[], at: number, length: number) => void} visit
 */
function eachToken(text, visit) {
  if (!HAN_CHARACTER.test(text)) return;
  for (let at = 0; at < text.length;) {
    let length = 1;
    for (const [token, words] of TOKENS_BY_FIRST.get(text[at]) || NO_TOKENS) {
      if (text.startsWith(token, at)) { length = token.length; visit(words, at, length); break; }
    }
    at += length;
  }
}

/**
 * Where a lowercased title holds a token read as one of `words`, as
 * [start, end) pairs, so a mark falls on the token the gloss came from.
 * @param {string} lowered @param {ReadonlySet<string>} words
 * @returns {[number, number][]}
 */
function wordRanges(lowered, words) {
  /** @type {[number, number][]} */
  const ranges = [];
  eachToken(withoutLinks(lowered), (found, at, length) => {
    if (found.some(word => words.has(word.en))) ranges.push([at, at + length]);
  });
  return ranges;
}

/**
 * The English words a title carries: `gloss` for its Chinese tokens, in
 * table order, and `traits`, the colour, fit and material words read from
 * its Chinese tokens or its Latin words. Shared and frozen when empty, as
 * most titles in a Latin catalogue are.
 * @param {string} lowered an NFKC-normalised, lowercased title
 * @returns {{gloss: readonly string[], traits: readonly string[]}}
 */
function readWords(lowered) {
  const text = withoutLinks(lowered);
  /** @type {Set<Word>} */
  const found = new Set();
  eachToken(text, words => { for (const word of words) found.add(word); });
  const gloss = found.size ? WORDS.filter(word => found.has(word)) : [];
  const traits = new Set(gloss.filter(word => word.facet !== "garment").map(word => word.en));
  for (const match of text.matchAll(LATIN_TRAIT)) traits.add(/** @type {string} */ (TRAIT_BY_FORM.get(match[1])));
  return { gloss: gloss.length ? gloss.map(word => word.en) : NO_WORDS, traits: traits.size ? [...traits] : NO_WORDS };
}

if (typeof module !== "undefined") {
  module.exports = { KINDS, KIND_IDS, UNSORTED, kindOf, WORDS, searchTerms, readWords, wordRanges };
}
