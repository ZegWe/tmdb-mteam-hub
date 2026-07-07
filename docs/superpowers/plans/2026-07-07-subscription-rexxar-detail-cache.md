# Subscription Rexxar Detail Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cache Douban rexxar subject metadata during wanted polling and display release-date-oriented subscription cards and richer subscription detail rows.

**Architecture:** Expose a rexxar-only detail fetcher in `src/douban.rs`; call it best-effort from `run_wanted_watch_poll()` and pass a subject-id detail map into `WantedSubscriptionStore`. Store cached detail fields on `WantedSubscriptionRecord`; render `date_published` on cards and cached media facts in subscription detail.

**Tech Stack:** Rust, serde, Vue 3, Node assertion tests.

---

### Task 1: Store Cached Rexxar Detail

**Files:**
- Modify: `src/subscription.rs`
- Modify: `src/main.rs`
- Modify: `src/douban.rs`

- [ ] **Step 1: Write failing Rust tests**

Add tests proving `WantedSubscriptionRecord` stores and refreshes rexxar detail fields from `DoubanSubjectDetail`.

- [ ] **Step 2: Run focused test**

Run: `cargo test subscription::tests::wanted_records_cache_douban_subject_detail`

Expected: FAIL because record fields and detail-aware apply path do not exist.

- [ ] **Step 3: Implement storage and fetch path**

Add optional detail fields to `WantedSubscriptionRecord`, add a detail-aware store method, expose `douban::subject_detail_rexxar()`, and call it best-effort during wanted polling.

- [ ] **Step 4: Run focused test**

Run: `cargo test subscription::tests::wanted_records_cache_douban_subject_detail`

Expected: PASS.

### Task 2: Render Card and Detail Rows

**Files:**
- Modify: `frontend/src/App.vue`
- Modify: `frontend/src/__tests__/subscription-card-display.test.mjs`

- [ ] **Step 1: Write failing frontend assertions**

Assert subscription card subtitles prefer `date_published` over `douban_date`, and subscription detail rows include cached media metadata.

- [ ] **Step 2: Run focused frontend test**

Run: `node frontend/src/__tests__/subscription-card-display.test.mjs`

Expected: FAIL because subtitles still use `douban_date` and detail rows lack cached metadata.

- [ ] **Step 3: Implement frontend formatting**

Update `subscriptionCardSubtitle()` and `subscriptionDetailRows()` to use cached detail fields.

- [ ] **Step 4: Run focused frontend test**

Run: `node frontend/src/__tests__/subscription-card-display.test.mjs`

Expected: PASS.

### Task 3: Verify

**Files:**
- No source edits expected.

- [ ] **Step 1: Run focused tests**

Run:

```bash
cargo test subscription::tests::wanted_records_cache_douban_subject_detail
node frontend/src/__tests__/subscription-card-display.test.mjs
```

Expected: both pass.

- [ ] **Step 2: Run broader checks**

Run:

```bash
npm run build
cargo test
```

Expected: both commands exit 0.
