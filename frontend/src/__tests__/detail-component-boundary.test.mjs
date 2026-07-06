import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const srcDir = resolve(__dirname, "..");
const appSource = readFileSync(resolve(srcDir, "App.vue"), "utf8");
const mediaComponentPath = resolve(srcDir, "components/MediaDetailView.vue");
const subscriptionComponentPath = resolve(srcDir, "components/SubscriptionDetailView.vue");

assert.ok(existsSync(mediaComponentPath), "media detail view component should exist");
assert.ok(existsSync(subscriptionComponentPath), "subscription detail view component should exist");

const mediaSource = readFileSync(mediaComponentPath, "utf8");
const subscriptionSource = readFileSync(subscriptionComponentPath, "utf8");

assert.match(
  appSource,
  /import MediaDetailView from "\.\/components\/MediaDetailView\.vue";/,
  "App.vue should import the media detail component",
);
assert.match(
  appSource,
  /import SubscriptionDetailView from "\.\/components\/SubscriptionDetailView\.vue";/,
  "App.vue should import the subscription detail component",
);
assert.match(appSource, /<MediaDetailView\b/, "App.vue should render MediaDetailView");
assert.match(
  appSource,
  /<SubscriptionDetailView\b/,
  "App.vue should render SubscriptionDetailView",
);

const detailPageSource = appSource.slice(
  appSource.indexOf('id="page-detail"'),
  appSource.indexOf('<dialog class="modal"', appSource.indexOf('id="page-detail"')),
);
assert.doesNotMatch(
  detailPageSource,
  /class="d-head"|class="subscription-detail"/,
  "App.vue detail page should delegate article templates to components",
);

assert.match(mediaSource, /class="d-head"/, "MediaDetailView should own the media detail article");
assert.match(
  subscriptionSource,
  /class="subscription-detail"/,
  "SubscriptionDetailView should own the subscription detail article",
);
