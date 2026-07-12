---
status: implemented
owner: tmdb-mteam-hub
last_verified: 2026-07-11
spec: docs/superpowers/specs/2026-07-07-subscription-cover-cards-design.md
related_adr: docs/adr/0002-subscription-state-convergence.md
---

# Subscription Cover Cards Implementation Plan

> Historical implementation record: the card behavior remains implemented, but the
> `WantedSubscriptionRecord`/`src/subscription.rs` storage references below describe the original
> implementation. Current persistence is the latest-only `subscriptions.sqlite` repository defined by
> the [architecture convergence PRD](../specs/2026-07-11-project-architecture-convergence-prd.md).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render subscriptions as poster cards and remove the separate Douban library option from the search page.

**Architecture:** Add poster fields to subscription records and populate them from Douban wanted-list items. Update the Vue template and CSS so subscription cards mirror search cards: cover image, title, concise metadata, and status. Remove the Douban library button and unused library view rendering from the search page while keeping Douban search.

**Tech Stack:** Rust, serde, Vue 3, Vite, Node assertion tests, CSS.

---

### Task 1: Backend Subscription Cover Fields

**Files:**
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write the failing test**

Add a test in `src/subscription.rs` asserting records created and refreshed from `DoubanLibraryItem` preserve `poster_url` and `cover_url`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test subscription::tests::wanted_records_preserve_douban_cover_urls`

Expected: FAIL because `WantedSubscriptionRecord` has no poster fields yet.

- [ ] **Step 3: Implement the minimal backend change**

Add `poster_url` and `cover_url` to `WantedSubscriptionRecord`, initialize them in fallback constructors, and copy them in `record_from_item()` and `refresh_record_from_item()`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test subscription::tests::wanted_records_preserve_douban_cover_urls`

Expected: PASS.

### Task 2: Frontend Subscription Poster Cards

**Files:**
- Modify: `frontend/src/App.vue`
- Modify: `frontend/src/styles.css`
- Modify: `frontend/src/__tests__/subscription-card-display.test.mjs`

- [ ] **Step 1: Write the failing frontend test**

Update `subscription-card-display.test.mjs` to assert the subscription card template renders an image using `itemImageUrl(record) || transparentPixel`, title, status, and no inline retry/rerun action buttons.

- [ ] **Step 2: Run the test to verify it fails**

Run: `node frontend/src/__tests__/subscription-card-display.test.mjs`

Expected: FAIL because the current subscription cards are status workflow cards without a poster image.

- [ ] **Step 3: Implement the subscription card template and CSS**

Change the subscription card to a poster card. Keep click-to-detail behavior, status badge, and concise metadata. Remove inline retry/rerun buttons from the card.

- [ ] **Step 4: Run the test to verify it passes**

Run: `node frontend/src/__tests__/subscription-card-display.test.mjs`

Expected: PASS.

### Task 3: Remove Douban Library Search Page Entry

**Files:**
- Modify: `frontend/src/App.vue`
- Modify: `frontend/src/__tests__/search-card-display.test.mjs`

- [ ] **Step 1: Write the failing frontend test**

Update `search-card-display.test.mjs` to assert the main search header does not contain the Douban library action and the template no longer renders the `douban-library` view branch.

- [ ] **Step 2: Run the test to verify it fails**

Run: `node frontend/src/__tests__/search-card-display.test.mjs`

Expected: FAIL because the template still has the Douban library button and view.

- [ ] **Step 3: Remove the library entry**

Remove the header action, library bar, library-specific section title branches, and the unused `loadDoubanLibrary()` path from the search page.

- [ ] **Step 4: Run the test to verify it passes**

Run: `node frontend/src/__tests__/search-card-display.test.mjs`

Expected: PASS.

### Task 4: Full Verification

**Files:**
- No source edits expected.

- [ ] **Step 1: Run focused frontend tests**

Run:

```bash
node frontend/src/__tests__/subscription-card-display.test.mjs
node frontend/src/__tests__/search-card-display.test.mjs
```

Expected: both PASS.

- [ ] **Step 2: Run focused backend test**

Run: `cargo test subscription::tests::wanted_records_preserve_douban_cover_urls`

Expected: PASS.

- [ ] **Step 3: Run broader checks**

Run:

```bash
npm run build
cargo test
```

Expected: both commands exit 0.
