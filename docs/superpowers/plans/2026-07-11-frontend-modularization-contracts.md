---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-11
prd: docs/superpowers/specs/2026-07-11-project-architecture-convergence-prd.md
workstream: frontend-modularization-contracts
implementation_status: in_progress
depends_on:
  - docs/superpowers/plans/2026-07-11-backend-application-boundaries.md
coordinates_with:
  - docs/superpowers/plans/2026-07-11-subscription-storage-scheduler.md
---

# Frontend Modularization and API Contracts Implementation Plan

## Goal

Replace the single mounted `App.vue` state machine with real route pages and feature-owned state,
while preserving deep links, browser Back behavior, subscription polling, search return state, and
the current user-visible behavior. Establish behavior-based tests before moving large templates.

## Architecture

```text
AppShell + RouterView
  -> Page components
       -> feature composables/stores
            -> shared typed API client
       -> leaf components emitting user intent
```

The first pass does not require Pinia. Feature modules may use composables and explicit singleton
stores where list/detail routes genuinely share state.

## Target files

- Modify: `package.json`
- Modify: `vite.config.ts`
- Replace: `frontend/src/main.js` with app/router assembly or split it into:
  - `frontend/src/app/router.js`
  - `frontend/src/app/AppShell.vue`
- Create page components under `frontend/src/pages/`
- Create feature modules under `frontend/src/features/`
- Create shared modules under `frontend/src/shared/`
- Gradually reduce and then replace `frontend/src/App.vue`
- Split `frontend/src/styles.css` into a documented style entry point and owned style modules.
- Replace source-string tests under `frontend/src/__tests__/` with Vitest suites.
- Create Playwright configuration and focused end-to-end tests.

## Required preserved behavior

- `#/detail/:mediaType/:id` deep links remain refresh-safe.
- `#/subscriptions/:id` deep links load the selected subscription.
- Browser Back returns from details to the correct list page.
- Search results and source selection survive a detail round trip.
- Subscription list and detail share fresh records while only the subscription route tree polls.
- Theme selection remains system/light/dark and survives reload.
- Optional M-Team failures do not block the primary media detail.
- Settings changes affect runtime caches only after a successful save.

## Implementation progress

Last updated: 2026-07-11.

- Tasks 1-3 are complete: Vitest/Vue Test Utils/happy-dom, importable domain helpers, the shared API
  client, endpoint modules, stable `ApiError`, abort/timeout handling, and latest-request-wins tests
  are in place. The temporary Node test runner has now been retired; all retained frontend contracts
  run through Vitest. Generic poster/image resolution now lives in `shared/media/images.js`, while
  search-card identity/subtitle rules live in `features/search/domain.js`; Search and Subscriptions no
  longer import these concerns from the Media Detail feature.
- Task 4's route/race safety net is complete. Deep links, Back navigation, media/search/M-Team/log
  stale-response protection, and subscription polling shutdown outside the route tree are covered.
  Search source/query/page and log filters now use URL query state as their shareable authority;
  feature stores retain only drafts, cached results, pagination data, and request state.
- The subscription feature store (Task 6) uses only the fresh summary/detail API. Summary pages are
  aggregated through bounded opaque cursors before one cache commit, and all 22 required summary
  fields, enum values, flags, timestamps, IDs, and attention tags fail closed at the endpoint boundary.
  Production now serves the latest list/detail routes, so normal runtime requests use the bounded
  summary path and nested detail-by-ID path. The aggregate/full-projection parser and retry/rerun
  API/store/UI have been deleted. Summary-only records load nested detail by ID,
  reuse it only while its revision is current, and cancel obsolete route requests.
  Settings exposes watcher enabled/dry-run safely, preserves the complete runtime watcher DTO, and
  requires explicit confirmation only for persisted false -> true changes. AuthGate prevents
  application creation and configuration requests before auth.
- Task 5 page ownership is complete: all product routes lazily load real pages through `AppShell`,
  production route records own navigation metadata, Media Detail owns isolated
  primary/interest/season/M-Team state, a catch-all route renders `NotFoundPage`, and `App.vue` is a
  seven-line composition root with no page `v-show` state.
- Task 6 now has current-backend leaf boundaries for the subscription card, lifecycle graph,
  download/episode state, and link/file results. Pages retain store/router/Poll orchestration and
  leaf components accept data models plus semantic events only. Fresh summaries must contain the same
  22 fields as the Rust DTO, require a positive safe-integer revision and valid enums/types, and cannot
  overwrite candidates/issues/source/artifacts or other detail-only fields. One entity map remains the
  mutable authority; per-ID summary/detail freshness metadata and `hasFreshDetail(id)` distinguish a
  preserved-but-stale detail after a newer summary. IDs use the backend's strict original-value,
  1..=256 UTF-8 byte, no-control/NUL/slash/backslash contract. Explicit `ordered_ids` preserve backend
  order even for integer-like Douban IDs, and invalid subscription deep links do not enter the route
  context or issue list/detail requests. The strict nested detail-by-ID transport is now wired through
  revision-aware cache/loading into the detail page and download/link/diagnostic/candidate UI.
- The current frontend gate passes 247 Vitest cases across 43 files, a dedicated checked-JavaScript
  API type gate, formatting/lint, and a production build. DaisyUI is now the documented primitive and
  theme authority; search/log/media-detail/subscription/settings/qB styles are feature-owned and
  route-lazy, known focus/contrast regressions have automated coverage, and `MediaDetailView` is now a
  thin panel composition boundary. Four Playwright journeys pass in Chromium desktop and mobile
  profiles; the first hosted Chromium/Firefox run and reviewed screenshots remain pending.
- The first zero-runtime-impact cleanup removed one fully superseded source-layout test and 276 net
  lines of unreachable CSS (legacy hidden/button/library/subscription-stage/notice/modal selectors)
  while preserving active DaisyUI primitives and `subscription-state-*` styles.
- The remaining four source/layout-oriented Node files were retired after preserving their useful
  route, display, subscription-domain, and Settings security assertions as importable or mounted
  Vitest behavior. `verify:frontend` now has one test runner.

## Task 1: Install a real frontend test harness

- [x] Add Vitest, `@vue/test-utils`, and `happy-dom` or `jsdom` as development dependencies.
- [x] Add `npm test` and `npm run test:watch`; retire the temporary `test:legacy` script after the
      migration completes.
- [x] Configure Vitest through `vite.config.ts` or a dedicated config.
- [x] Add a test setup file for DOM cleanup, fetch mocks, and timer cleanup.
- [x] Keep existing Node tests running during migration, then remove the runner only after retained
      invariants have Vitest behavior or pure-domain coverage.
- [x] Create one mounted smoke test for the application shell.

Verification:

```bash
npm test
npm run check
npm run build
```

Commit boundary: test tooling and smoke test only.

## Task 2: Extract importable pure domain helpers

- [x] Move theme functions into `shared/theme/theme-mode.js` with direct unit tests.
- [x] Move route normalization/location helpers into `app/detail-routes.js`.
- [x] Move formatting helpers into `shared/lib/formatters.js`.
- [x] Move media metadata mapping into `features/media-detail/domain.js`.
- [x] Move subscription lifecycle labels, ordering, summaries, actions, and artifact-row mapping into
      `features/subscriptions/domain.js`.
- [x] Fix the current summary mismatch so it counts `queued`, `meta`, `searching`, `downloading`,
      `linking`, `completed`, and attention tags rather than legacy keys.
- [x] Convert current `vm.runInNewContext` tests into normal imports.
- [x] Delete source-slice assertions only after equivalent behavior tests pass.

Task 2 ownership evidence (2026-07-11):

- `shared/media/images.js` owns TMDB poster URL construction and provider-neutral image selection.
- `features/search/domain.js` owns search-card keys and compact subtitles. Search pages and
  subscription cards no longer depend on `features/media-detail/domain.js` for unrelated helpers.
- Direct tests preserve provider precedence, TMDB URL construction, key fallbacks, and Douban/TMDB
  subtitle output.

Verification:

```bash
npm test -- theme detail-routes media-detail subscriptions
```

Commit boundary: pure functions and tests; no template movement.

## Task 3: Introduce the shared API client

- [x] Create `shared/api/client.js` with:
  - JSON request/response handling.
  - Stable `ApiError` shape and server error codes.
  - Request and connect-like timeout behavior via `AbortController`.
  - Caller-provided abort signals.
  - No JSON `Content-Type` header for GET requests without a body.
- [x] Create endpoint modules for search, media details, subscriptions, settings, logs, and qB.
- [x] Keep current response adaptation inside endpoint modules rather than page/components.
- [x] Add tests for success, empty body, invalid JSON, HTTP errors, network errors, timeout, and abort.
- [x] Add a latest-request-wins helper or request sequence abstraction for interactive queries.
- [x] Ensure errors preserve the original cause where available.

Verification:

```bash
npm test -- api
```

Commit boundary: API infrastructure and tests; App continues using current state.

## Task 4: Protect route and asynchronous behavior before page extraction

- [x] Add router tests using an in-memory history.
- [x] Test direct media-detail and subscription-detail entry.
- [x] Test card navigation and browser Back behavior.
- [x] Test rapid detail-route changes where an older response resolves last.
- [x] Test rapid search source/query changes.
- [x] Test rapid M-Team source-tab changes.
- [x] Test log filter changes with stale responses.
- [x] Test that hidden/unmounted subscription routes stop polling.
- [x] Decide where search query/source/page and log filters live:
  - Use URL query state for shareable/filter state.
  - Use a feature store only for non-shareable cached results.

Task 4 state-ownership evidence (2026-07-11):

- `features/search/route.js` normalizes `source`, submitted `q`, and Douban `page` into URL query
  state. Direct search deep links hydrate and load once; remounting after a detail round trip reuses the
  matching result cache instead of issuing a duplicate search.
- Draft text remains store-owned until submit. Source switches and pagination publish canonical route
  state, while stale requests remain fenced by the existing latest-request-wins store boundary.
- `features/logs/route.js` continues to own category/status/query URL synchronization. Search and log
  route tests cover direct links, default omission, Back/Forward behavior, and one-load-per-transition.

Verification:

```bash
npm test -- router race polling
```

Commit boundary: behavior characterization only.

## Task 5: Introduce AppShell and real route pages

**Implementation status: complete for page ownership.** Every route now renders a real lazy page
through `AppShell`/`RouterView`; remaining work belongs to feature/component and styling tasks.

- [x] Create `app/AppShell.vue` containing navigation, theme control, global toast, and `<RouterView>`.
- [x] Replace every `EmptyRoute` with a real lazily imported page component.
- [x] Use route metadata for navigation selection instead of duplicated route-to-page mappings.
- [x] Create all route pages:
  - [x] `pages/SearchPage.vue`
  - [x] `pages/MediaDetailPage.vue`
  - [x] `pages/SubscriptionsPage.vue`
  - [x] `pages/SubscriptionDetailPage.vue`
  - [x] `pages/LogsPage.vue`
  - [x] `pages/SettingsPage.vue`
- [x] Add a not-found route rather than silently rendering the search page.
- [x] Move templates one page at a time, keeping feature logic temporarily injectable if necessary.
- [x] Remove the corresponding `v-show` section from `App.vue` after each page test is green.
- [x] Preserve search results and list-return behavior explicitly across detail navigation.

Verification after each route move:

```bash
npm test -- router pages
npm run build
```

Route metadata evidence (2026-07-11):

- `app/routes.js` is the production route-record authority. Search/media detail, subscriptions/detail,
  logs, and settings expose stable `meta.navPage` ownership; not-found exposes an explicit empty
  navigation state.
- `AppShell.vue` reads only `route.meta.navPage` and navigates by route name. It no longer contains a
  route-name-to-navigation or target-to-path mapping.
- Router and mounted App tests cover every metadata mapping, media/subscription detail deep links,
  and a not-found route with no selected primary navigation item.

Commit boundaries: one page family per commit; do not move all pages in one unreviewable change.

## Task 6: Build the subscriptions feature store

**Implementation status: complete for the latest-only subscription frontend.** Real lazy list/detail pages,
route lifecycle, capabilities, watcher banner, fail-closed summary contracts, cursor aggregation,
strict IDs, revision-aware nested-detail cache, route loading/cancellation, and nested UI are complete.
Production registration and runtime adoption of the fresh backend list/detail routes are complete.
The aggregate/full-projection parser, retry/rerun endpoint exports, store commands, detail events,
buttons and action capabilities are deleted.

- [x] Create `features/subscriptions/store.js` owning records, summaries, selected ID, loading states,
      polling, and semantic commands.
- [x] Initially isolate, then delete, the former whole-state endpoint adapter.
- [x] Migrate production loading to list-summary and detail-by-ID endpoints.
  - [x] Accept only strict `{items,next_cursor}` pages; reject unknown fields and malformed cursor
        chains before the cache changes.
  - [x] Require and whitelist all 22 `SubscriptionSummaryDto` fields at endpoint/store boundaries;
        validate enums, booleans, nullable timestamps/text, retry ranges, unique attention tags, and
        positive safe-integer revisions before preventing summary data from erasing detail fields.
  - [x] Keep one entity map as authority with separate summary/detail freshness and expose
        `hasFreshDetail(id)` for route decisions.
  - [x] Enforce ID/key equality and the backend's original-value 1..=256 UTF-8 byte contract.
  - [x] Aggregate every cursor page before applying a complete summary snapshot, with bounded page and
        record counts, abort/repeat detection, and explicit `ordered_ids` from raw item order.
  - [x] Add the strict nested detail-by-ID endpoint transport and path/body ID/revision validation.
  - [x] Add revision-aware detail cache, route loading, stale-response, missing-record, and nested UI
        behavior.
- [x] Start polling when entering the subscriptions route layout and stop on route-tree exit.
- [x] Pause polling when the document is hidden and prevent overlapping requests.
- [x] Merge detail updates by revision/updated time so stale responses do not regress state.
- [x] Move lifecycle graph and artifact mapping to the feature domain module.
- [x] Derive inactive, TV unsupported, backend-blocked, and schedulable capabilities in the domain
      module while treating missing legacy `active`/`schedulable` fields compatibly.
- [x] Keep inactive history navigable and remove every unported side-effect command/button; TV and
      blocked records remain explicit read-only states.
- [x] Render unloaded, disabled, dry-run, and live watcher modes from the shared settings runtime store;
      keep watcher DTO normalization in the settings helper and reuse one subscriptions banner on list
      and detail routes.
- [x] Create components:
  - [x] `SubscriptionCard.vue`
  - [x] `LifecycleGraph.vue`
  - [x] `DownloadTaskList.vue`
  - [x] `LinkResult.vue`
  - [x] TV episode/task execution components are intentionally N/A while the backend contract remains
        `tv_not_supported`; do not add dead execution UI ahead of an end-to-end executor.
- [x] Replace function props with emitted commands or feature context.

Subscription component evidence (2026-07-11):

- `features/subscriptions/SubscriptionCard.vue` accepts only a subscription record and emits one
  semantic `open` intent. `SubscriptionsPage.vue` retains store, refresh, notification, and router
  ownership.
- Card status, subtitle, and movie/inactive/TV/backend-blocked capability badges reuse the existing
  subscription domain helpers; no capability rules are duplicated in the component.
- Mounted component tests cover badge text/tone plus click and Enter activation. Subscription page
  route tests continue to prove list-to-detail navigation and browser Back behavior.
- `features/subscriptions/LifecycleGraph.vue` accepts only a record and reuses the lifecycle-node and
  display-status domain helpers. It preserves the six-node state/attention classes, ARIA label, and
  “等待发布 / 阻塞 / 跳过 / 失败” graph copy while leaving detail header/error ownership in the parent
  view.
- `features/subscriptions/DownloadTaskList.vue` accepts only a record, owns the download metadata and
  episode-task rendering, and reuses the progress, push-row, status-label, and percentage helpers. It
  prefers non-empty completion episodes while preserving push-episode fallback.
- Mounted DownloadTaskList tests cover the not-pushed empty state, normalized overall progress and
  push metadata, episode progress/status/file counts, push fallback, and completion precedence.
- `features/subscriptions/LinkResult.vue` accepts only a record, reuses completion-row and shared file
  formatters, and owns hard-link metadata plus the merged file list. Dedicated completion linked files
  retain priority over push fallback files; push download files remain first and the combined list is
  capped at 120 rows.
- Mounted LinkResult tests cover empty rendering, completion metadata, dedicated/fallback linked-file
  selection, error/source/size note priority, status/progress display, merge order, and the 120-row
  cap. The parent detail view now retains header/meta/actions/error ownership and delegates its body to
  LifecycleGraph, DownloadTaskList, and LinkResult leaves.
- SubscriptionCard emits `open`; subscription detail is read/Poll-only; all three detail leaves are
  record-only. No subscription component receives store/router callbacks or function props.
- `shared/api/endpoints/subscriptions.js` accepts only the latest list contract, aggregates every
  bounded summary page, preserves raw item order through `ordered_ids`, and projects summaries through
  an explicit 22-field whitelist. Its detail transport accepts only the exact nested DTO envelope and
  matching path/body ID. `features/subscriptions/store.js` repeats projection/ID/order/snapshot checks
  before mutating its single entity map.
- Summary and detail freshness are tracked independently. A newer summary preserves older detail-only
  fields but makes `hasFreshDetail(id)` false; a matching/newer full detail restores it. Strict ID tests
  cover byte length, boundary whitespace, controls/NUL, separators, map-key equality, zero-request
  invalid deep links, and invalid-to-valid route recovery.
- `store.loadDetail(id)` coalesces per-ID work, owns its `AbortController`, skips a network request
  for a current cached revision, retries one summary/detail race, and rejects exhausted stale detail
  without regressing the entity. Detail errors/loading remain per ID and cancellations stay silent.
- `SubscriptionDetailPage.vue` waits for the shared initial summary-list request, then loads detail
  only when `hasFreshDetail(id)` is false. Poll-driven
  revision changes trigger a new detail load; route changes/unmount abort the previous ID; typed 404
  responses render a stable missing-record state.
- Nested `source` and `observation` fields drive detail rows; `issues` and `candidates` render
  directly from the DTO; DownloadTaskList and LinkResult consume `downloads[]` and `links[]`. No
  aggregate or full-projection fallback remains.
- Focused store/context/page/component tests cover current-cache reuse, revision invalidation, stale
  response retry/no-regression, A→B cancellation, missing records, poll-triggered reload, and nested
  artifacts. The latest `npm run verify:frontend` passes all 247 tests across 43 files, checked API
  types, formatting/lint checks, and the production build.

Verification:

```bash
npm test -- subscriptions
```

Include fake-timer tests for polling start, stop, hidden-page pause, error recovery, and detail refresh.

Commit boundary: store first, then list page, then detail page/components.

## Task 7: Split media-detail features and decouple optional loading

**Implementation status: complete.** `MediaDetailPage` owns an importable store with independent
primary, interest, season, and M-Team state plus cancellation/latest-request-wins; qB dialog state is
separate. `MediaDetailView` is a 54-line composition boundary over focused primary, metadata, Douban
interest, TV season, and M-Team torrent panels, with one read-only model and semantic events.

- [x] Extract the primary TMDB/Douban request and derived detail model into
      `features/media-detail/primary-store.js` (or a composable only if component lifecycle ownership
      requires one). Do not create `useMediaDetail.js` merely to satisfy a filename.
- [x] Create independent feature modules for:
  - [x] Douban interest editing in `features/media-detail/interest-store.js`.
  - [x] TV season/episode loading in `features/media-detail/season-store.js`.
  - [x] M-Team torrent search in `features/media-detail/mteam-store.js`.
  - [x] qB push dialog orchestration in `features/qb/push-dialog-store.js`.
- [x] Mark the primary detail request complete before starting or awaiting M-Team search.
- [x] Give each optional panel its own loading and error state.
- [x] Cancel old detail and panel requests on route change/unmount.
- [x] Replace the current 34-prop media detail boundary with a cohesive read-only model and emitted
      semantic events.
- [x] Ensure child components do not mutate nested prop objects directly.

Verification:

```bash
npm test -- media-detail douban-interest season torrent qb
```

Test that a never-resolving or failed M-Team request does not leave the main detail loading.

Evidence:

- `features/media-detail/primary-store.js` owns the primary TMDB/Douban request, route identity,
  loading/error state, latest-request-wins cancellation, disposal guards, and the derived title,
  date, overview, poster, season, metadata, external-link, and page-heading model. Its four focused
  tests cover TMDB and Douban projections, stale-route isolation, reset/error behavior, and a late
  response after disposal. `features/media-detail/store.js` is now only the primary/interest/season/
  M-Team orchestration layer and preserves the page-facing contract.
- `features/media-detail/interest-store.vitest.js` covers initialization and derived fields, hydrate
  cancellation/latest-route guards, tag-history request de-duplication and error isolation, mark
  editing, normalized saves, stale-save isolation, and reset/dispose behavior. The media-detail store
  integration test verifies tag history and hydrate requests start only after primary loading clears
  and remain non-blocking.
- `features/media-detail/season-store.vitest.js` covers same-season request de-duplication, concurrent
  loading across different seasons, per-season errors, response normalization, and reset/dispose
  cancellation with late-response isolation. The composed store retains the existing season model
  keys and `loadSeason` return contract.
- `features/media-detail/mteam-store.vitest.js` covers ordered source construction, response
  normalization, per-context caching, latest-request-wins behavior, optional-panel error isolation,
  and reset/dispose cancellation. `features/media-detail/store.js` composes that store only after
  publishing the resolved primary detail while retaining the existing `mteam` model and
  `selectTorrentSource` boundary.
- `features/qb/push-dialog-store.js` owns runtime server loading, dialog/form state, ID-only push
  submission, duplicate-submit protection, and dispose-safe open/submit lifecycles. Its focused tests
  cover missing or invalid servers, current-request failures, runtime-load failures, duplicate
  submissions, active-torrent stability during a pending push, and stale completion after dispose;
  `features/qb/domain.js` owns the push payload so the qB feature no longer depends on the settings
  form model.
- `MediaPrimaryPanel.vue` and `MediaMetadataPanel.vue` own primary identity/external links and metadata;
  `DoubanInterestPanel.vue`, `TvSeasonPanel.vue`, and `MteamTorrentPanel.vue` own their feature markup
  and emit only editing, expansion, source-selection, and push intent. Four focused mounted tests prove
  their projections, event payloads, and no nested prop mutation. Existing page/router/qB tests remain
  green after the split.

Commit boundary: primary detail, then each optional panel separately.

## Task 8: Separate settings DTOs, form state, and runtime state

- [x] Create `features/settings/api.js` with redacted configuration DTOs from the safety/backend plan.
- [x] Create `features/settings/form-model.js` for UI-only fields and normalization.
- [x] Maintain separate objects for:
  - Last saved runtime configuration summary.
  - Editable settings draft.
  - Per-row testing/status UI state.
- [x] Fetch only the redacted configuration after authentication; no frontend path fetches a
      secret-bearing general configuration response.
- [x] Treat an omitted secret as “keep existing” and an explicit clear action as “delete secret”.
- [x] Update shared qB/category summaries only after a successful save response.
- [x] Send qB test requests through server IDs for saved servers; use a separately authorized draft-test
      endpoint only if testing unsaved servers remains required.
- [x] Ensure QR completion consumes `cookie_saved` and refreshes the redacted revision snapshot
      without receiving or round-tripping the Douban Cookie.

Verification:

```bash
npm test -- settings qb-config qr
```

Include a test proving unsaved form edits do not change the server used by the qB push dialog.

Commit boundary: DTO/form split before template extraction.

## Task 9: Extract logs and qB dialog features

- [x] Move log filters, pagination, formatting, and API state into `features/logs/`.
- [x] Store filters in URL query parameters where appropriate.
- [x] Cancel stale log queries when filters change.
- [x] Move qB dialog state/API into `features/qb/`.
- [x] Use semantic dialog open/close behavior with focus return and Escape support.
- [x] Replace static `open` binding with controlled `showModal`/`close` integration or an accessible
      Vue dialog component.
- [x] Add keyboard and focus-management component tests.

Verification:

```bash
npm test -- logs qb-dialog accessibility
```

Task 9 evidence (2026-07-11):

- `features/logs/domain.js`, `store.js`, and `route.js` now own formatting, filter/query
  normalization, pagination, API/loading/error/toast state, request cancellation, and route history
  synchronization. `LogsPage.vue` is a thin template adapter.
- Category, status, and keyword filters round-trip through `/logs` query parameters. Deep links and
  browser Back/Forward reload exactly once per route transition; latest-request-wins prevents stale
  responses from replacing the active result.
- `npm test -- logs` passes 14 focused store/domain/route/page tests.
- `features/qb/ControlledDialog.vue` keeps the qB store and native dialog state synchronized through
  guarded `showModal()`/`close()` calls. Native cancel/Escape, backdrop and close-button dismissal,
  successful submit, external native close, focus return, and unmount cleanup share the same store
  close path.
- Mounted media-detail tests spy on `showModal`, `close`, cancel prevention, and focus restoration;
  repeated open requests do not call `showModal` twice, and a disabled qB server select is skipped
  for initial focus.
- This Task 9 slice originally passed 132 tests; the current combined baseline is 247 tests across 43
  files with checked API types, formatting/lint checks, and the production build.

Commit boundary: logs and qB dialog separately.

## Task 10: Converge API types

**Implementation status: complete (Git adoption pending).** Backend-owned OpenAPI 3.1 schemas now generate the checked-
JavaScript contract module and enum/field constants. A digest gate rejects stale generated output,
and the compiler gate consumes that output. M-Team search and TMDB search/detail/season now have
closed schemas; the frontend no longer accepts provider-shaped M-Team alternatives or TMDB
`name/original_name/type/external_ids` aliases. Auth/settings/qB endpoints also have strict checked-JS
request/response DTOs. Douban search/detail/interest/library/tags/QR and operation-log responses now
have closed schemas, and subscription detail now has explicit source/observation/issue/candidate/
TV/download/link schemas. A Router-source parity gate verifies all 27 production methods,
management security, every object schema is closed, and every operation references the shared
401/405/500/503 response contracts where applicable.

- [x] Replace the checked mirror with generated TypeScript/JavaScript types from the backend
      OpenAPI/JSON Schema artifact when that artifact exists.
- [x] Add checked JavaScript typedefs for subscription summary/detail, operation-log, and search DTOs,
      with one shared enum/field vocabulary consumed by runtime subscription validation.
- [x] Type the generic API client plus search, logs, and subscription endpoint modules before their
      page/component consumers.
- [x] Type auth/settings/media-detail/qB endpoint modules, including Douban detail, interest, tags and
      QR session responses from generated contracts.
- [x] Remove defensive parsing branches for response shapes the backend no longer emits. M-Team,
      TMDB and Douban search/detail now consume one canonical response shape.
- [x] Add compile/type-check failures for missing required fields and invalid lifecycle values, plus
      runtime failures for missing/invalid 22-field subscription summaries.
- [x] Keep upstream-provider variability normalized on the backend adapter side. M-Team, TMDB and
      Douban all expose closed response DTOs.
- [x] Keep feature migration incremental through checked JavaScript; do not combine a wholesale
      TypeScript migration with route/component extraction.

Verification:

```bash
npm run check:api-contracts
npm run check
npm run typecheck:api
npm test
```

Commit boundary: generated contract plumbing, then one feature at a time.

## Task 11: Establish one styling authority and remove dead CSS

**Implementation status: complete locally (Git adoption/hosted review pending).** DaisyUI is the explicit primitive/theme authority, global
primitive collisions are removed, tokens/base rules have explicit entry points, and every current
feature cluster is feature-owned. Only reviewed light/dark desktop/mobile screenshots remain open.

- [x] Inventory DaisyUI component classes, Tailwind utilities, and custom primitives.
- [x] Choose DaisyUI as the authority for buttons, cards, inputs, dialogs, tabs, badges, and theme
      tokens; custom selectors may only add feature layout or semantic variants.
- [x] Retain DaisyUI, remove custom global redefinitions of its primitives, and keep feature
      layout styles scoped or namespaced.
- [x] Reject the custom-primitives alternative and remove duplicate theme definitions from CSS.
- [x] Split tokens/base/layout from all feature-owned styles.
  - [x] Move search and logs into route-lazy `features/*/styles.css` modules.
  - [x] Move media-detail, subscriptions, qB, and settings compatibility clusters.
- [x] Remove confirmed dead selectors including legacy drawer/library/subscription-stage/card-notice
      rules and unused `#q`, `.btn.primary`, `.btn.secondary`, and `.btn-mini` rules.
- [x] Remove the independently verified dead hidden/button/library/subscription-stage/card-notice,
      stale subscription-card branch, modal-wide selector batch, unreachable legacy subscription
      status arms, and the unused `#q` rules.
- [x] Audit dead refs/functions: remove the unused detail endpoint alias and stale public exports;
      confirm `detailOpen`/manual progress-completion paths are absent and retain `settingsLoaded`
      because it still gates save submission.
- [x] Fix known color-contrast and keyboard-focus regressions with executable WCAG ratio tests,
      focusable search result cards, role focus styles, `aria-current`, and source `aria-pressed` state.
- [x] Capture the reproducible post-convergence baseline for all six routes in light/dark at desktop
      and mobile breakpoints. `e2e/visual.spec.js` produces 24 screenshots, asserts the expected main
      heading/theme, rejects horizontal overflow and rejects empty PNGs. A historical pre-convergence
      baseline is not fabricated after the fact; the current evidence becomes the review baseline.

Verification:

```bash
npm run check
npm run build
```

Run visual regression or reviewed screenshot comparison at desktop and mobile breakpoints.

Latest local evidence: 24/24 visual cases passed and wrote
`test-results/visual-evidence/<project>/<theme>/<route>.png`; representative desktop/mobile and
light/dark screenshots were manually inspected. A complete hosted artifact review remains part of the
first cross-browser CI gate.

Task 11 evidence (2026-07-11):

- The selector audit identified global `.btn`, `.card`, `.grid`, `.select`, and `.modal` collisions.
  Those global overrides are now gone: DaisyUI owns primitives, `.media-grid` is feature-specific,
  and search/log CSS ships as separate lazy route assets.
- `shared/theme/theme-contract.js` is the single light/dark DaisyUI and semantic-token authority used
  by `tailwind.config.js`. `styles/foundation.css` only aliases those generated theme variables, and
  `styles/base.css` owns reset/form/focus rules. `main.js` imports foundation, base, then the remaining
  compatibility stylesheet in the original cascade order.
- Before dead cleanup, the split build emitted the exact same CSS filename and SHA-256
  `c3dca6a170314042b5f275fe0e2e19ac15361c2041463a2ba0d055efa254c7cb`; `cmp` confirmed byte-for-byte
  equivalence with the pre-split artifact.
- Template/dynamic-class reachability proves `#q` and legacy linked/pushed/downloaded/matching/
  processing subscription-status selectors are unreachable. They were removed while current
  lifecycle/attention selectors remain covered by mounted/domain tests.
- Contrast tests require WCAG AA (4.5:1) for text/status pairs and 3:1 for focus rings against both
  surfaces in both themes. Light muted text was corrected, and hard-coded low-contrast banner, toast,
  overview, subscription-status, and qB-success colors now use the theme contract.
- `styles.css` is reduced from 1,458 to 381 lines and now contains application layout plus genuinely
  shared detail/form helpers. Media-detail, subscriptions, settings, and qB own dedicated style files
  loaded by their lazy pages/components; the production build emits separate feature CSS assets.
- `npm run verify:frontend` passes 247 tests across 43 files, generated-contract drift checking,
  checked API types, all checked frontend/config formatting and lint, and the production build. The
  eight functional plus 24 visual Chromium desktop/mobile Playwright cases also pass after the
  component/style split.

Commit boundary: tokens/primitives first, then one feature stylesheet at a time, then dead cleanup.

## Task 12: Add end-to-end acceptance flows

- [x] Install and configure Playwright against a disposable backend fixture or controlled mock server.
- [x] Add search flow:
  - Search.
  - Open detail.
  - Confirm primary detail while optional provider loads.
  - Browser Back returns to preserved results.
- [x] Add subscription flow:
  - Open list.
  - Open detail.
  - Observe polling update.
  - Confirm unported side-effect actions are absent.
  - Return to list without duplicate polling.
- [x] Add authenticated settings save/redaction flow.
- [x] Add a direct deep-link reload case.
- [ ] Confirm the first hosted cross-browser CI run. The quality workflow now installs Chromium and
      Firefox, runs both Chromium viewport profiles plus Firefox desktop, and uploads traces,
      screenshots, video, and the HTML report on failure.

Verification:

```bash
npx playwright test
```

Current local evidence (2026-07-12): the deterministic fixture contract test passes. Playwright
discovers 32 default cases: four functional journeys in both Chromium desktop/mobile profiles plus
six routes in light/dark across both profiles. All pass using the installed system Chrome. The
default local command remains:

```bash
npm run test:e2e
```

The hosted quality job additionally sets `PLAYWRIGHT_CROSS_BROWSER=1` after installing Chromium and
Firefox, producing twelve cases. Its first GitHub-hosted run remains an external acceptance gate.

The Playwright configuration starts a disposable Node fixture on loopback, serves `static/`, and
provides deterministic `/api/config`, search/detail, subscription list/detail/poll, and auth/settings
endpoints. Chromium desktop/mobile are the default local profiles; Firefox desktop is enabled by the
CI-only cross-browser flag now that the required flows and direct deep-link reload are green.

Commit boundary: E2E infrastructure and flows after route/service convergence is stable.

## Rollout order

1. Tasks 1–4 build the safety net without changing page ownership.
2. Task 5 introduces real routing one page family at a time.
3. Tasks 6–9 create feature ownership.
4. Task 10 adopts stable backend DTOs.
5. Task 11 removes styling/dead-code redundancy after templates settle.
6. Task 12 proves the final user journeys.

## Completion gate

This plan is complete when:

- `App.vue` is an app shell or has been removed in favor of `AppShell.vue`.
- Every route uses a real page component and no page switching relies on global `v-show`.
- Search/detail races cannot apply stale responses.
- Subscription polling is owned by the subscription route tree and cannot overlap.
- Settings drafts are isolated from saved runtime state.
- Media detail is not blocked by M-Team loading.
- Large callback prop tunnels are removed.
- Source-layout tests are replaced by import, component, router, and E2E behavior tests.
- One documented styling authority remains and confirmed dead CSS/code is gone.
- Frontend tests, checks, builds, and E2E flows pass in CI.
