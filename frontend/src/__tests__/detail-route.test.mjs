import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(__dirname, "../App.vue"), "utf8");
const stylesSource = readFileSync(resolve(__dirname, "../styles.css"), "utf8");
const functionStart = appSource.indexOf("const DETAIL_MEDIA_TYPES");
const functionEnd = appSource.indexOf("\n\nconst page = computed", functionStart);

assert.notEqual(functionStart, -1, "detail route helpers should start at DETAIL_MEDIA_TYPES");
assert.notEqual(functionEnd, -1, "detail route helpers should end before page computed state");

const helpers = vm.runInNewContext(
  `${appSource.slice(functionStart, functionEnd)}
({
  normalizeDetailRoute,
  detailRouteLocationFromMediaCard,
  detailRouteLocationFromSubscriptionRecord,
  detailBackRouteLocation,
});`,
);

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

assert.deepEqual(
  plain(
    helpers.normalizeDetailRoute({
      name: "media-detail",
      params: { mediaType: "movie", id: "123" },
    }),
  ),
  {
    kind: "media",
    mediaType: "movie",
    id: "123",
  },
);

assert.deepEqual(
  plain(
    helpers.normalizeDetailRoute({
      name: "media-detail",
      params: { mediaType: ["tv"], id: [456] },
    }),
  ),
  {
    kind: "media",
    mediaType: "tv",
    id: "456",
  },
);

assert.deepEqual(
  plain(
    helpers.normalizeDetailRoute({
      name: "subscription-detail",
      params: { id: "douban-7" },
    }),
  ),
  {
    kind: "subscription",
    id: "douban-7",
  },
);

assert.equal(
  helpers.normalizeDetailRoute({ name: "media-detail", params: { mediaType: "movie" } }),
  null,
);
assert.equal(
  helpers.normalizeDetailRoute({ name: "media-detail", params: { mediaType: "bad", id: "123" } }),
  null,
);
assert.equal(helpers.normalizeDetailRoute({ name: "main", params: {} }), null);

assert.deepEqual(
  plain(
    helpers.detailRouteLocationFromMediaCard(
      { id: "subject-9", subject_id: "fallback", source: "douban", tags: "tag" },
      "movie",
    ),
  ),
  {
    name: "media-detail",
    params: { mediaType: "douban", id: "subject-9" },
    query: { doubanTags: "tag" },
  },
);

assert.deepEqual(
  plain(helpers.detailRouteLocationFromMediaCard({ id: 42, media_type: "tv" }, "movie")),
  {
    name: "media-detail",
    params: { mediaType: "tv", id: "42" },
    query: {},
  },
);

assert.deepEqual(plain(helpers.detailRouteLocationFromSubscriptionRecord({ subject_id: 88 })), {
  name: "subscription-detail",
  params: { id: "88" },
  query: {},
});

assert.deepEqual(
  plain(helpers.detailBackRouteLocation({ kind: "media", mediaType: "movie", id: "1" })),
  { name: "main" },
);

assert.deepEqual(plain(helpers.detailBackRouteLocation({ kind: "subscription", id: "1" })), {
  name: "subscriptions",
});

assert.match(
  appSource,
  /function openCardDetail\(item, fallbackType\) \{\s+const detailLocation = detailRouteLocationFromMediaCard\(item, fallbackType\);[\s\S]+pushDetailRoute/,
  "media card clicks should navigate to the standalone detail route",
);

assert.match(
  appSource,
  /function openSubscriptionDetail\(record\) \{\s+const detailLocation = detailRouteLocationFromSubscriptionRecord\(record\);[\s\S]+pushDetailRoute/,
  "subscription card clicks should navigate to the standalone subscription detail route",
);

assert.match(
  appSource,
  /const navigation = alreadyInDetail \? router\.replace\(target\) : router\.push\(target\);/,
  "detail route writer should use router history for first opens and replace existing detail pages",
);

assert.match(
  appSource,
  /function syncDetailFromRoute\(\)/,
  "route watcher should delegate detail page state to syncDetailFromRoute",
);

assert.match(
  appSource,
  /route\.name,\s*route\.params\.mediaType,\s*route\.params\.id,\s*route\.query\.doubanTags/,
  "route watcher should observe detail route fields",
);

assert.match(appSource, /page === ['"]detail['"]/, "detail should render as an app page");
assert.doesNotMatch(
  stylesSource,
  /#detail\.detail-drawer/,
  "detail page should not use fixed drawer styles",
);
