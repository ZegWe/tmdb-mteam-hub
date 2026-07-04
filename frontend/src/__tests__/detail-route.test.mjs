import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(__dirname, "../App.vue"), "utf8");
const functionStart = appSource.indexOf("const DETAIL_QUERY_KEYS");
const functionEnd = appSource.indexOf("\n\nconst page = computed", functionStart);

assert.notEqual(functionStart, -1, "detail route helpers should start at DETAIL_QUERY_KEYS");
assert.notEqual(functionEnd, -1, "detail route helpers should end before page computed state");

const helpers = vm.runInNewContext(
  `${appSource.slice(functionStart, functionEnd)}
({
  normalizeDetailRouteQuery,
  detailRouteQueryFromMediaCard,
  detailRouteQueryFromSubscriptionRecord,
  withoutDetailRouteQuery,
});`,
);

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

assert.deepEqual(plain(helpers.normalizeDetailRouteQuery({ detail: "movie", id: "123" })), {
  kind: "media",
  mediaType: "movie",
  id: "123",
});

assert.deepEqual(plain(helpers.normalizeDetailRouteQuery({ detail: ["tv"], id: [456] })), {
  kind: "media",
  mediaType: "tv",
  id: "456",
});

assert.deepEqual(
  plain(helpers.normalizeDetailRouteQuery({ detail: "subscription", id: "douban-7" })),
  {
    kind: "subscription",
    id: "douban-7",
  },
);

assert.equal(helpers.normalizeDetailRouteQuery({ detail: "movie" }), null);
assert.equal(helpers.normalizeDetailRouteQuery({ detail: "bad", id: "123" }), null);

assert.deepEqual(
  plain(
    helpers.detailRouteQueryFromMediaCard(
      { id: "subject-9", subject_id: "fallback", source: "douban", tags: "tag" },
      "movie",
    ),
  ),
  { detail: "douban", id: "subject-9", doubanTags: "tag" },
);

assert.deepEqual(
  plain(helpers.detailRouteQueryFromMediaCard({ id: 42, media_type: "tv" }, "movie")),
  {
    detail: "tv",
    id: "42",
  },
);

assert.deepEqual(plain(helpers.detailRouteQueryFromSubscriptionRecord({ subject_id: 88 })), {
  detail: "subscription",
  id: "88",
});

assert.deepEqual(
  plain(helpers.withoutDetailRouteQuery({ detail: "movie", id: "1", doubanTags: "x", q: "keep" })),
  { q: "keep" },
);

assert.match(
  appSource,
  /function openCardDetail\(item, fallbackType\) \{\s+const detailQuery = detailRouteQueryFromMediaCard\(item, fallbackType\);[\s\S]+pushDetailRoute/,
  "media card clicks should write detail query through the router",
);

assert.match(
  appSource,
  /function openSubscriptionDetail\(record\) \{\s+const detailQuery = detailRouteQueryFromSubscriptionRecord\(record\);[\s\S]+pushDetailRoute/,
  "subscription card clicks should write detail query through the router",
);

assert.match(
  appSource,
  /const navigation = alreadyInDetail \? router\.replace\(target\) : router\.push\(target\);/,
  "detail route writer should use router history for first opens and replace existing detail URLs",
);

assert.match(
  appSource,
  /function syncDetailFromRoute\(\)/,
  "route watcher should delegate detail drawer state to syncDetailFromRoute",
);

assert.match(
  appSource,
  /route\.query\.detail,\s*route\.query\.id,\s*route\.query\.doubanTags/,
  "route watcher should observe detail query fields",
);
