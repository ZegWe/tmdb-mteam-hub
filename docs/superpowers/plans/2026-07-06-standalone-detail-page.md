# Standalone Detail Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the detail drawer with standalone detail routes that keep the left navigation visible, then refactor the detail page into focused Vue components.

**Architecture:** Phase A keeps the existing single-file Vue structure and changes routing, state sync, and CSS so detail content renders as a normal right-side page. Phase C preserves behavior and extracts detail rendering into focused components with route-driven loading still owned by the app shell.

**Tech Stack:** Vue 3 `<script setup>`, Vue Router hash history, Vite/Vite Plus, Node assertion tests.

---

### Task 1: Phase A Route Contract

**Files:**

- Modify: `frontend/src/__tests__/detail-route.test.mjs`
- Modify: `frontend/src/main.js`
- Modify: `frontend/src/App.vue`

- [ ] **Step 1: Write the failing route test**

Update `frontend/src/__tests__/detail-route.test.mjs` so route helpers assert path-based detail routes:

```js
assert.deepEqual(
  plain(helpers.detailRouteLocationFromMediaCard({ id: 42, media_type: "tv" }, "movie")),
  {
    name: "media-detail",
    params: { mediaType: "tv", id: "42" },
    query: {},
  },
);

assert.deepEqual(
  plain(
    helpers.detailRouteLocationFromMediaCard(
      { id: "subject-9", source: "douban", tags: "tag" },
      "movie",
    ),
  ),
  {
    name: "media-detail",
    params: { mediaType: "douban", id: "subject-9" },
    query: { doubanTags: "tag" },
  },
);

assert.deepEqual(plain(helpers.detailRouteLocationFromSubscriptionRecord({ subject_id: 88 })), {
  name: "subscription-detail",
  params: { id: "88" },
  query: {},
});
```

- [ ] **Step 2: Verify RED**

Run: `node frontend/src/__tests__/detail-route.test.mjs`

Expected: FAIL because `detailRouteLocationFromMediaCard` and `detailRouteLocationFromSubscriptionRecord` do not exist.

- [ ] **Step 3: Implement minimal route helpers and routes**

Add `media-detail` and `subscription-detail` routes in `frontend/src/main.js`. Replace query helper usage in `frontend/src/App.vue` with path-based route helpers, but keep the existing detail loader functions.

- [ ] **Step 4: Verify GREEN**

Run: `node frontend/src/__tests__/detail-route.test.mjs`

Expected: PASS.

### Task 2: Phase A Standalone Page Layout

**Files:**

- Modify: `frontend/src/App.vue`
- Modify: `frontend/src/styles.css`
- Test: `frontend/src/__tests__/detail-route.test.mjs`

- [ ] **Step 1: Extend failing layout assertions**

Assert the app renders a `page === 'detail'` section and no fixed detail drawer selector remains:

```js
assert.match(appSource, /page === 'detail'/, "detail should render as an app page");
assert.doesNotMatch(
  stylesSource,
  /#detail\.detail-drawer/,
  "detail page should not use fixed drawer styles",
);
```

- [ ] **Step 2: Verify RED**

Run: `node frontend/src/__tests__/detail-route.test.mjs`

Expected: FAIL because the drawer is still present.

- [ ] **Step 3: Move the detail template into the app content**

Render detail content inside `<section id="page-detail" class="app-page detail-page">`, keep the left navigation, and convert the close button into a back button.

- [ ] **Step 4: Replace drawer CSS with page CSS**

Remove fixed drawer positioning. Add `.detail-page`, `.detail-page-top`, and `.detail-body` styles so media/subscription detail content uses the available right-side width.

- [ ] **Step 5: Verify GREEN**

Run:

```bash
node frontend/src/__tests__/detail-route.test.mjs
npm run build
```

Expected: both PASS.

- [ ] **Step 6: Commit Phase A**

Run:

```bash
git add docs/superpowers/plans/2026-07-06-standalone-detail-page.md frontend/src/main.js frontend/src/App.vue frontend/src/styles.css frontend/src/__tests__/detail-route.test.mjs
git commit -m "feat: make detail view a standalone page"
```

### Task 3: Phase C Component Refactor

**Files:**

- Create: `frontend/src/components/MediaDetailView.vue`
- Create: `frontend/src/components/SubscriptionDetailView.vue`
- Modify: `frontend/src/App.vue`
- Test: `frontend/src/__tests__/detail-component-boundary.test.mjs`

- [ ] **Step 1: Write the failing component boundary test**

Create `frontend/src/__tests__/detail-component-boundary.test.mjs` to assert `App.vue` imports and uses `MediaDetailView` and `SubscriptionDetailView`, and the component files contain only their focused detail templates.

- [ ] **Step 2: Verify RED**

Run: `node frontend/src/__tests__/detail-component-boundary.test.mjs`

Expected: FAIL because the component files do not exist.

- [ ] **Step 3: Extract media detail template**

Move the media detail article template into `MediaDetailView.vue`, pass data and callbacks as props, and preserve all existing UI states and action wiring.

- [ ] **Step 4: Extract subscription detail template**

Move the subscription detail article template into `SubscriptionDetailView.vue`, pass data and callbacks as props, and preserve all existing UI states and action wiring.

- [ ] **Step 5: Verify GREEN**

Run:

```bash
node frontend/src/__tests__/detail-route.test.mjs
node frontend/src/__tests__/detail-component-boundary.test.mjs
npm run build
```

Expected: all PASS.

- [ ] **Step 6: Commit Phase C**

Run:

```bash
git add frontend/src/App.vue frontend/src/components/MediaDetailView.vue frontend/src/components/SubscriptionDetailView.vue frontend/src/__tests__/detail-component-boundary.test.mjs
git commit -m "refactor: split detail views into components"
```
