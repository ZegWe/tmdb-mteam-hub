---
---

status: superseded
owner: tmdb-mteam-hub
last_verified: 2026-07-11
archived_at: 2026-07-11
authoritative: false
executable: false
superseded_by: docs/adr/0001-standalone-detail-routes.md
related_adr: docs/adr/0001-standalone-detail-routes.md

---

# Detail Drawer URL State Implementation Plan

> 历史归档：本文档的任务、命令和复选框均不可执行；当前路由决策以 ADR 0001 为准。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make search and subscription detail drawers URL-driven so Back closes the drawer and copied/refreshed detail URLs restore the drawer.

**Architecture:** Keep the existing hash routes and drawer UI. Add small pure helpers for detail query normalization, route writing, and query stripping, then make a route watcher the owner of drawer open/close state. Card clicks write route query; the watcher loads media or subscription detail from query.

**Tech Stack:** Vue 3 `<script setup>`, vue-router hash history, Node-based source extraction tests, existing Vite/Vite Plus build checks.

---

### Task 1: Test Detail Query Helpers

**Files:**

- Create: `frontend/src/__tests__/detail-route.test.mjs`
- Modify: `frontend/src/App.vue`

- [ ] **Step 1: Write the failing test**

Create `frontend/src/__tests__/detail-route.test.mjs`:

```js
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

assert.deepEqual(helpers.normalizeDetailRouteQuery({ detail: "movie", id: "123" }), {
  kind: "media",
  mediaType: "movie",
  id: "123",
});

assert.deepEqual(helpers.normalizeDetailRouteQuery({ detail: ["tv"], id: [456] }), {
  kind: "media",
  mediaType: "tv",
  id: "456",
});

assert.deepEqual(helpers.normalizeDetailRouteQuery({ detail: "subscription", id: "douban-7" }), {
  kind: "subscription",
  id: "douban-7",
});

assert.equal(helpers.normalizeDetailRouteQuery({ detail: "movie" }), null);
assert.equal(helpers.normalizeDetailRouteQuery({ detail: "bad", id: "123" }), null);

assert.deepEqual(
  helpers.detailRouteQueryFromMediaCard(
    { id: "subject-9", subject_id: "fallback", source: "douban", tags: "tag" },
    "movie",
  ),
  { detail: "douban", id: "subject-9", doubanTags: "tag" },
);

assert.deepEqual(helpers.detailRouteQueryFromMediaCard({ id: 42, media_type: "tv" }, "movie"), {
  detail: "tv",
  id: "42",
});

assert.deepEqual(helpers.detailRouteQueryFromSubscriptionRecord({ subject_id: 88 }), {
  detail: "subscription",
  id: "88",
});

assert.deepEqual(
  helpers.withoutDetailRouteQuery({ detail: "movie", id: "1", doubanTags: "x", q: "keep" }),
  { q: "keep" },
);
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node frontend/src/__tests__/detail-route.test.mjs`

Expected: FAIL with `detail route helpers should start at DETAIL_QUERY_KEYS`.

- [ ] **Step 3: Implement the pure helpers**

Add helpers in `frontend/src/App.vue` after `routeToPage` and before `const page = computed`:

```js
const DETAIL_QUERY_KEYS = ["detail", "id", "doubanTags"];

function firstQueryValue(value) {
  return Array.isArray(value) ? value[0] : value;
}

function normalizeDetailRouteQuery(query) {
  const detail = String(firstQueryValue(query?.detail) || "").trim();
  const id = String(firstQueryValue(query?.id) || "").trim();
  if (!detail || !id) return null;
  if (["movie", "tv", "douban"].includes(detail)) {
    return { kind: "media", mediaType: detail, id };
  }
  if (detail === "subscription") return { kind: "subscription", id };
  return null;
}

function detailRouteQueryFromMediaCard(item, fallbackType) {
  const type = item?.source === "douban" ? "douban" : item?.media_type || fallbackType;
  const rawId = type === "douban" ? (item?.id ?? item?.subject_id) : item?.id;
  const id = String(rawId || "").trim();
  if (!id) return null;
  const query = { detail: type, id };
  const tags = Array.isArray(item?.tags) ? item.tags.join(" ") : item?.tags || "";
  if (type === "douban" && String(tags).trim()) query.doubanTags = String(tags).trim();
  return query;
}

function detailRouteQueryFromSubscriptionRecord(record) {
  const id = String(record?.subject_id || "").trim();
  return id ? { detail: "subscription", id } : null;
}

function withoutDetailRouteQuery(query) {
  const next = { ...(query || {}) };
  for (const key of DETAIL_QUERY_KEYS) delete next[key];
  return next;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node frontend/src/__tests__/detail-route.test.mjs`

Expected: PASS with exit code 0.

### Task 2: Wire Route Query to Drawer State

**Files:**

- Modify: `frontend/src/App.vue`
- Test: `frontend/src/__tests__/detail-route.test.mjs`

- [ ] **Step 1: Add route writer functions and internal loaders**

Change card click functions to push query:

```js
function openCardDetail(item, fallbackType) {
  const detailQuery = detailRouteQueryFromMediaCard(item, fallbackType);
  if (!detailQuery) return;
  router.push({ name: route.name || "main", query: { ...route.query, ...detailQuery } });
}

function openSubscriptionDetail(record) {
  const detailQuery = detailRouteQueryFromSubscriptionRecord(record);
  if (!detailQuery) return;
  router.push({
    name: "subscriptions",
    query: { ...withoutDetailRouteQuery(route.query), ...detailQuery },
  });
}
```

Rename the current media loader to `loadMediaDetailFromRoute(mediaType, id, options = {})` and keep its existing body. Add `loadSubscriptionDetailFromRoute(id)` that ensures subscriptions are loaded and selects the matching record.

- [ ] **Step 2: Add the route watcher**

Add a watcher on `[page.value, route.query.detail, route.query.id, route.query.doubanTags]`:

```js
watch(
  () => [page.value, route.query.detail, route.query.id, route.query.doubanTags],
  () => {
    syncDetailFromRoute().catch((err) => {
      detailOpen.value = true;
      detailLoading.value = false;
      detailError.value = err instanceof Error ? err.message : String(err);
    });
  },
  { immediate: true },
);
```

Implement `syncDetailFromRoute()` so missing/invalid query closes and resets detail, media query loads media detail, and subscription query loads subscription detail.

- [ ] **Step 3: Make close remove detail query**

Change `closeDetail()` to remove detail query with `router.back()` when possible after route-pushed card clicks, and `router.replace()` for direct-open detail URLs:

```js
function closeDetail() {
  if (normalizeDetailRouteQuery(route.query)) {
    router.replace({ name: route.name || "main", query: withoutDetailRouteQuery(route.query) });
    return;
  }
  detailOpen.value = false;
  resetDetail();
}
```

- [ ] **Step 4: Preserve subscription selection during refresh**

Keep the existing auto-sync selected ID refresh. In `loadSubscriptions`, after assigning `subscriptionState.value`, if the current route query is a subscription detail, refresh `selectedSubscription` from `subscriptionRecords`.

- [ ] **Step 5: Run focused tests**

Run:

```bash
node frontend/src/__tests__/detail-route.test.mjs
node frontend/src/__tests__/subscription-order.test.mjs
```

Expected: both commands exit 0.

### Task 3: Build Verification

**Files:**

- Modify: `frontend/src/App.vue`
- Modify only if generated by build: `static/index.html`, `static/assets/*`

- [ ] **Step 1: Run frontend checks**

Run: `npm run check`

Expected: exit code 0. If the command is unavailable or fails for an unrelated environment issue, capture the exact output and run `npm run build`.

- [ ] **Step 2: Run build**

Run: `npm run build`

Expected: exit code 0. Static assets may be regenerated because this repository tracks built frontend output.

- [ ] **Step 3: Inspect final diff**

Run:

```bash
git diff -- frontend/src/App.vue frontend/src/__tests__/detail-route.test.mjs docs/archive/superpowers/plans/2026-07-02-detail-url-state.md
git status --short
```

Expected: diff contains URL-driven detail logic, the new focused test, and this plan. Existing unrelated dirty files remain untouched except build outputs if `npm run build` rewrites them.
