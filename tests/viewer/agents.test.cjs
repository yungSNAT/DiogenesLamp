// Copyright 2026 yungSNAT
// SPDX-License-Identifier: Apache-2.0

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { AGENTS, ACTIVE_AGENTS, AGENT_HOSTS, unwrapAgentLink, agentLink } = require("../../viewer/agents.js");
const { marketplaceItem, marketplaceUrl } = require("../../viewer/dedup.js");

const IDS = { weidian: "7652831688", taobao: "560198996624", "1688": "691234567890" };
const WD = "https%3A%2F%2Fweidian.com%2Fitem.html%3FitemID%3D7652831688";
const TB = "https%3A%2F%2Fitem.taobao.com%2Fitem.htm%3Fid%3D560198996624";
const ALI = "https%3A%2F%2Fdetail.1688.com%2Foffer%2F691234567890.html";

// Read from rep.tools' public converter on 27 September 2026, minus its
// KakoBuy referral code. OrientDig, PonyBuy and Fishgoo were opened on
// 16 November 2026; PonyBuy's taobao code did not open, so it offers none.
// LoloBuy was opened on 17 November 2026; its taobao link opened a manual
// order form instead of the item, so it offers none for taobao either.
const OBSERVED = {
  weidian: {
    kakobuy: `https://www.kakobuy.com/item/details?url=${WD}`,
    hoobuy: "https://hoobuy.com/product/2/7652831688",
    oopbuy: "https://oopbuy.com/goods/details?id=7652831688&channel=weidian",
    mulebuy: "https://mulebuy.com/product/?shop_type=weidian&id=7652831688",
    joyagoo: "https://joyagoo.com/product/?shop_type=weidian&id=7652831688",
    cssbuy: "https://www.cssbuy.com/item-micro-7652831688.html",
    sugargoo: `https://www.sugargoo.com/productDetail?productLink=${WD}`,
    superbuy: `https://www.superbuy.com/en/page/buy/?url=${WD}`,
    loongbuy: `https://www.loongbuy.com/product-details?url=${WD}`,
    pikobuy: `https://www.pikobuy.com/product/detail?productUrl=${WD}`,
    itaobuy: `https://www.itaobuy.com/product-detail?url=${WD}`,
    bbdbuy: `https://bbdbuy.com/index/item/index.html?tp=taobao&tid=&searchlang=en&url=${WD}`,
    ootdbuy: "https://www.ootdbuy.com/goods/details?id=7652831688&channel=weidian",
    lovegobuy: "https://www.lovegobuy.com/product?id=7652831688&shop_type=weidian",
    cnshopper: "https://cnshopper.com/goods/detail?keyword=7652831688&platform=weidian",
    alleurobuy: "https://alleurobuy.com/index/detail-public?id=7652831688&type=weidian",
    rizzitgo: "https://rizzitgo.com/detail-page?goodsId=7652831688&source=3",
    vigorbuy: "https://vigorbuy.com/product/2/7652831688",
    usfans: "https://usfans.com/product/3/7652831688",
    litbuy: "https://www.litbuy.com/product/weidian/7652831688",
    hipobuy: "https://hipobuy.com/product/weidian/7652831688",
    gtbuy: "https://www.gtbuy.com/product/weidian/7652831688",
    orientdig: "https://orientdig.com/product/?shop_type=weidian&id=7652831688",
    ponybuy: "https://www.ponybuy.com/products/3/7652831688",
    fishgoo: `https://www.fishgoo.com/#/product?productLink=${WD}`,
    lolobuy: `https://www.lolobuy.com/productDetail/0?url=${WD}`,
  },
  taobao: {
    kakobuy: `https://www.kakobuy.com/item/details?url=${TB}`,
    hoobuy: "https://hoobuy.com/product/1/560198996624",
    oopbuy: "https://oopbuy.com/goods/details?id=560198996624&channel=taobao",
    mulebuy: "https://mulebuy.com/product/?shop_type=taobao&id=560198996624",
    joyagoo: "https://joyagoo.com/product/?shop_type=taobao&id=560198996624",
    cssbuy: "https://www.cssbuy.com/item-560198996624.html",
    sugargoo: `https://www.sugargoo.com/productDetail?productLink=${TB}`,
    superbuy: `https://www.superbuy.com/en/page/buy/?url=${TB}`,
    loongbuy: `https://www.loongbuy.com/product-details?url=${TB}`,
    pikobuy: `https://www.pikobuy.com/product/detail?productUrl=${TB}`,
    itaobuy: `https://www.itaobuy.com/product-detail?url=${TB}`,
    bbdbuy: `https://bbdbuy.com/index/item/index.html?tp=taobao&tid=&searchlang=en&url=${TB}`,
    ootdbuy: "https://www.ootdbuy.com/goods/details?id=560198996624&channel=TAOBAO",
    lovegobuy: "https://www.lovegobuy.com/product?id=560198996624&shop_type=taobao",
    cnshopper: "https://cnshopper.com/goods/detail?keyword=560198996624&platform=taobao",
    alleurobuy: "https://alleurobuy.com/index/detail-public?id=560198996624&type=tao",
    rizzitgo: "https://rizzitgo.com/detail-page?goodsId=560198996624&source=1",
    vigorbuy: "https://vigorbuy.com/product/1/560198996624",
    usfans: "https://usfans.com/product/2/560198996624",
    litbuy: "https://www.litbuy.com/product/1/560198996624",
    hipobuy: "https://hipobuy.com/product/1/560198996624",
    gtbuy: "https://www.gtbuy.com/product/1/560198996624",
    orientdig: "https://orientdig.com/product/?shop_type=taobao&id=560198996624",
    ponybuy: "",
    fishgoo: `https://www.fishgoo.com/#/product?productLink=${TB}`,
    lolobuy: "",
  },
  "1688": {
    kakobuy: `https://www.kakobuy.com/item/details?url=${ALI}`,
    hoobuy: "https://hoobuy.com/product/0/691234567890",
    oopbuy: "https://oopbuy.com/goods/details?id=691234567890&channel=1688",
    mulebuy: "https://mulebuy.com/product/?shop_type=ali_1688&id=691234567890",
    joyagoo: "https://joyagoo.com/product/?shop_type=ali_1688&id=691234567890",
    cssbuy: "https://www.cssbuy.com/item-1688-691234567890.html",
    sugargoo: `https://www.sugargoo.com/productDetail?productLink=${ALI}`,
    superbuy: `https://www.superbuy.com/en/page/buy/?url=${ALI}`,
    loongbuy: `https://www.loongbuy.com/product-details?url=${ALI}`,
    pikobuy: `https://www.pikobuy.com/product/detail?productUrl=${ALI}`,
    itaobuy: `https://www.itaobuy.com/product-detail?url=${ALI}`,
    bbdbuy: `https://bbdbuy.com/index/item/index1688.html?tp=taobao&tid=&searchlang=en&url=${ALI}`,
    ootdbuy: "https://www.ootdbuy.com/goods/details?id=691234567890&channel=1688",
    lovegobuy: "https://www.lovegobuy.com/product?id=691234567890&shop_type=ali_1688",
    cnshopper: "https://cnshopper.com/goods/detail?keyword=691234567890&platform=1688",
    alleurobuy: "https://alleurobuy.com/index/detail-public?id=691234567890&type=ali",
    rizzitgo: "https://rizzitgo.com/detail-page?goodsId=691234567890&source=2",
    vigorbuy: "https://vigorbuy.com/product/0/691234567890",
    usfans: "https://usfans.com/product/1/691234567890",
    litbuy: "https://www.litbuy.com/product/0/691234567890",
    hipobuy: "https://hipobuy.com/product/0/691234567890.html",
    gtbuy: "https://www.gtbuy.com/product/0/691234567890",
    orientdig: "https://orientdig.com/product/?shop_type=ali_1688&id=691234567890",
    ponybuy: "https://www.ponybuy.com/products/2/691234567890",
    fishgoo: `https://www.fishgoo.com/#/product?productLink=${ALI}`,
    lolobuy: `https://www.lolobuy.com/productDetail/0?url=${ALI}`,
  },
};

test("every active agent rebuilds the observed link for each marketplace", () => {
  for (const [platform, expected] of Object.entries(OBSERVED)) {
    assert.deepEqual(Object.keys(expected).sort(), ACTIVE_AGENTS.map(a => a.id).sort(), platform);
    for (const [agentId, url] of Object.entries(expected)) {
      assert.equal(agentLink(agentId, { platform, id: IDS[platform] }), url, `${agentId} ${platform}`);
    }
  }
});

test("built links unwrap back to the item they were built from", () => {
  for (const agent of ACTIVE_AGENTS) {
    for (const [platform, id] of Object.entries(IDS)) {
      const item = { platform, id };
      if (OBSERVED[platform][agent.id] === "") continue;
      assert.deepEqual(marketplaceItem(unwrapAgentLink(agentLink(agent.id, item))), item, `${agent.id} ${platform}`);
    }
    // A tmall link is offered only where the format keeps the tmall
    // identity; agents that name a taobao channel offer none.
    const link = agentLink(agent.id, { platform: "tmall", id: "77" });
    if (link) assert.deepEqual(marketplaceItem(unwrapAgentLink(link)), { platform: "tmall", id: "77" }, agent.id);
    else assert.ok(["hoobuy", "oopbuy", "mulebuy", "joyagoo", "cssbuy", "ootdbuy", "lovegobuy", "cnshopper",
      "alleurobuy", "rizzitgo", "vigorbuy", "usfans", "litbuy", "hipobuy", "gtbuy", "orientdig", "ponybuy",
      "lolobuy"].includes(agent.id), agent.id);
  }
  assert.equal(agentLink("hoobuy", { platform: "tmall", id: "77" }), "");
  assert.equal(agentLink("kakobuy", { platform: "tmall", id: "77" }),
    "https://www.kakobuy.com/item/details?url=https%3A%2F%2Fdetail.tmall.com%2Fitem.htm%3Fid%3D77");
});

test("retired agents still unwrap but never build, and stay out of the menu", () => {
  assert.equal(unwrapAgentLink("https://cnfans.com/product/?shop_type=weidian&id=7652831688"),
    "https://weidian.com/item.html?itemID=7652831688");
  assert.equal(unwrapAgentLink("https://www.pandabuy.com/product?url=" + TB), marketplaceUrl({ platform: "taobao", id: IDS.taobao }));
  assert.equal(agentLink("cnfans", { platform: "weidian", id: "1" }), "");
  assert.equal(agentLink("nobody", { platform: "weidian", id: "1" }), "");
  assert.equal(agentLink("hoobuy", null), "");
  assert.ok(AGENTS.filter(a => a.retired).every(a => !ACTIVE_AGENTS.includes(a)));
  assert.equal(new Set(AGENT_HOSTS).size, AGENTS.length);
  assert.ok(AGENTS.every(a => /^\d{4}-\d{2}-\d{2}$/.test(a.checked)));
});

test("unwrapping refuses ambiguity, other sites and deep nesting", () => {
  const evil = "https%3A%2F%2Fevil.example%2Fitem.html%3FitemID%3D1";
  for (const url of [
    `https://www.kakobuy.com/item/details?url=${evil}`,
    // an unrelated site carrying a marketplace URL in its own query string
    "https://www.kakobuy.com/item/details?url=" + encodeURIComponent("https://evil.example/?next=https://weidian.com/item.html?itemID=123"),
    "https://oopbuy.com/goods/details?id=5&id=6&channel=weidian",
    "https://mulebuy.com/product/?shop_type=weidian&channel=taobao&id=5",
    "https://oopbuy.com/goods/details?id=5&channel=unknown",
    "https://hoobuy.com/product/9/123",
    "https://hoobuy.com/product/3/123",
    "https://usfans.com/product/0/123",
    "https://rizzitgo.com/detail-page?goodsId=5&source=weidian",
    "https://www.litbuy.com/product/tmall/5",
    "https://usfans.com/product/0/691234567890?keyword=691234567890&platform=1688",
    "https://oopbuy.com/goods/details?keyword=7652831688&platform=weidian",
    "https://cnshopper.com/goods/detail?id=7652831688&channel=weidian",
    "https://www.kakobuy.com/item/details?id=7652831688&channel=weidian",
    "https://oopbuy.com/goods/details?url=" + WD,
    "https://oopbuy.com/goods/details?id=7652831688&keyword=6&channel=weidian",
    "https://www.cssbuy.com/item-abc.html",
    // PonyBuy's taobao code is unconfirmed; the plural path is PonyBuy's alone.
    "https://www.ponybuy.com/products/1/560198996624",
    "https://www.ponybuy.com/product/3/7652831688",
    "https://hoobuy.com/products/2/7652831688",
    // Fishgoo names its item only in the fragment route #/product.
    "https://www.fishgoo.com/?productLink=" + WD,
    "https://www.fishgoo.com/#/home?productLink=" + WD,
    "https://www.fishgoo.com/?x=1#/product?productLink=" + WD,
    "https://www.fishgoo.com/#/product?productLink=" + WD + "&productLink=" + encodeURIComponent("https://weidian.com/item.html?itemID=5"),
    // Two different wrapped URLs under different names name no single item.
    "https://www.fishgoo.com/#/product?productLink=" + WD + "&url=" + TB,
    "https://www.kakobuy.com/item/details?url=" + WD + "&productLink=" + TB,
    "https://notkakobuy.com/item/details?url=" + WD,
    "https://weidian.com/item.html?itemID=7652831688",
    "not a url",
  ]) assert.equal(unwrapAgentLink(url), null, url);
  // One agent quoting another is one more unwrap; a third layer is not followed.
  // Query hints on a path or wrapped agent do not make its link ambiguous.
  assert.equal(unwrapAgentLink("https://hoobuy.com/product/2/7652831688?id=5&keyword=6"),
    "https://weidian.com/item.html?itemID=7652831688");
  assert.equal(unwrapAgentLink("https://www.kakobuy.com/item/details?url=" + WD + "&id=1&id=2"),
    "https://weidian.com/item.html?itemID=7652831688");
  const nested = "https://www.kakobuy.com/item/details?url=" + encodeURIComponent("https://hoobuy.com/product/2/7652831688");
  assert.equal(unwrapAgentLink(nested), "https://weidian.com/item.html?itemID=7652831688");
  const deeper = "https://www.kakobuy.com/item/details?url=" + encodeURIComponent(nested);
  assert.equal(unwrapAgentLink(deeper), null);
  const routed = "https://www.kakobuy.com/item/details?url=" + encodeURIComponent("https://www.fishgoo.com/#/product?productLink=" + WD);
  assert.equal(unwrapAgentLink(routed), "https://weidian.com/item.html?itemID=7652831688");
  // The fragment route ends at a second '#'; an encoded '#' stays in the
  // wrapped URL; a missing slash before the route is the root path.
  for (const url of [
    "https://www.fishgoo.com/#/product?productLink=" + WD + "#&url=" + TB,
    "https://www.fishgoo.com/#/product?productLink=" + WD + "%23x",
    "https://www.fishgoo.com#/product?productLink=" + WD,
    "https://www.kakobuy.com/item/details?url=" + WD + "&link=" + WD,
    // An empty parameter names nothing, so it is no second wrapped URL.
    "https://www.kakobuy.com/item/details?url=&link=" + WD,
  ]) assert.equal(unwrapAgentLink(url), "https://weidian.com/item.html?itemID=7652831688", url);
});
