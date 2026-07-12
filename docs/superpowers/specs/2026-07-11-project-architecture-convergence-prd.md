---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-12
implementation_status: in_progress
supersedes:
  - docs/archive/superpowers/specs/2026-07-02-detail-url-state-design.md
  - docs/archive/superpowers/specs/2026-07-08-subscription-state-convergence-prd.md
related:
  - docs/archive/superpowers/specs/2026-07-08-subscription-state-convergence-prd.md
---

# TMDB M-Team Hub Architecture Convergence PRD

## Background

The project has reached the point where feature behavior is useful and reasonably well covered by
unit tests, but the implementation boundaries have not kept pace with the product surface.

At the initial 2026-07-11 audit baseline, the repository had these concentration and governance
signals. The current state is tracked separately in the implementation-progress table below:

- Rust contains 16,658 lines, with `src/main.rs` and `src/subscription.rs` holding 81.5% of them.
- The frontend runtime contains 5,916 lines, with `App.vue` and `styles.css` holding about 91%.
- `src/main.rs` owns bootstrap, routing, configuration, HTTP handlers, external clients,
  subscription orchestration, torrent matching, episode parsing, hardlinking, and tests.
- `src/subscription.rs` owns domain models, state transitions, SQLite schema, repository, audit log,
  old-state compatibility concerns, and tests.
- Vue Router currently uses empty route components while `App.vue` mounts every page and switches
  them with `v-show`.
- Subscription persistence writes both a whole-account state blob and per-record JSON rows, while
  the runtime normally reads the whole-account blob.
- The background worker reads all subscriptions every second and the frontend downloads the full
  subscription aggregate every five seconds.
- Management APIs are unauthenticated, CORS allows any origin, and `/api/config` returns secrets.
- Malformed TOML silently falls back to defaults and startup then writes the defaults back.
- TV scheduling is selectable in production, but its executors are not implemented.
- Historical implementation plans have no lifecycle status and conflict with current behavior.

This PRD turns the architecture audit into one coordinated convergence program. The intent is not
to rewrite the application, but to preserve the useful product while removing the structural risks
that make subsequent iteration unsafe or expensive.

## Goals

1. Make the management plane safe by default.
2. Prevent configuration parse or state-initialization errors from destroying user data.
3. Keep a modular monolith: one Rust service, one SQLite database, and one Vue SPA.
4. Establish explicit HTTP, application, domain, repository, and external-adapter boundaries.
5. Make one per-record SQLite representation the authoritative subscription store.
6. Claim due work atomically and route each background/manual command through the same
   responsibility-specific application service.
7. Stop processing subscriptions that are no longer present in the latest Douban wanted list.
8. Ensure TV work is either implemented end to end or explicitly unavailable to the scheduler.
9. Return typed, stable API DTOs and split subscription list summaries from heavy details.
10. Replace the frontend monolith with real route pages and feature-owned state.
11. Replace source-layout tests with behavior, router, service, contract, and component tests.
12. Make a clean checkout independently buildable, testable, deployable, recoverable, and documented.

## Non-goals

- Splitting the application into microservices.
- Adding Redis, Kafka, a distributed queue, or a separate database server.
- Introducing multi-user tenancy or a general identity platform.
- Redesigning the visual language of the application.
- Adding new media providers or expanding the product surface during convergence.
- Preserving internal source layout solely to keep source-string tests passing.
- Migrating or importing legacy subscription SQLite/JSON state.
- Preserving legacy runtime state fields, account blobs, converter code, or old API shapes.

## Architecture principles

### One source of truth

Each durable concept must have one authoritative representation:

- Configuration: the validated in-memory configuration and one persisted TOML file.
- Subscription: one per-record SQLite aggregate plus indexed scheduling columns.
- Download progress: one download task artifact.
- Hardlink result: one link result artifact.
- API state: explicit DTOs, not direct serialization of internal aggregates.

Compatibility copies must not be dual-written. The latest subscription runtime uses a new canonical
database; production code does not enumerate, probe, open, read, convert, modify, or delete old state
files.

### Transport-independent application services

HTTP handlers and background workers must invoke the same responsibility-specific application
commands. A worker must never call an Axum handler directly, and an external adapter must not depend
on an HTTP-layer error type. Query, Poll, and Execution remain separate services rather than being
collected into one catch-all `SubscriptionService`.

### Safe automation

Automation that pushes torrents or writes media files must be explicitly enabled. Work selection,
claiming, execution, and result persistence must be idempotent and recoverable after process failure.

### Vertical feature boundaries

Backend and frontend modules should be grouped by stable reasons to change, not split mechanically by
line count. A module owns its domain rules, while adapters own upstream or storage details.

### Incremental convergence

The convergence must proceed through small, verifiable changes. Valid current configuration must
survive every intermediate version. Legacy subscription state is deliberately not imported; the
latest repository starts from `subscriptions.sqlite`.

## Functional requirements

### R1. Management-plane security

- The default listener must be loopback unless the user explicitly opts into wider exposure.
- Production API access must require an administrator credential or a trusted reverse-proxy identity.
- CORS must be absent for same-origin deployment or restricted to an explicit allowlist.
- Secret-bearing configuration must not be returned by general configuration reads.
- Configuration responses must expose `has_*` flags and redacted qB server summaries.
- qB action APIs must reference a configured server ID rather than accept arbitrary URLs/passwords.
- Internal automation endpoints must not remain publicly callable without authorization.
- Configuration and backup files containing secrets must be created with restrictive permissions.

### R2. Configuration safety

- A malformed existing TOML file must produce a startup error with location details.
- Parse failure must never create or overwrite a replacement configuration.
- Schema/default normalization must only persist when a validated change is necessary.
- Configuration normalization must create a backup before replacing the source file.
- Unknown fields must either be preserved or rejected explicitly; they must not disappear silently.
- Concurrent settings updates and QR-login cookie updates must not overwrite unrelated fields.

### R3. Automation controls

- `subscription_watcher.enabled` must exist and default to `false` for new installations.
- The UI and API must expose whether automation is enabled.
- Enabling automation must be an explicit settings action.
- Dry-run or paused execution must be available for validating candidate selection and hardlink plans.
- Poll generation/open-attempt identity, poll success, incomplete diagnostics, failure backoff, and
  next poll time must be persisted. Exactly one terminal result may consume an open poll token.

### R4. Backend boundaries

- `main.rs` must contain only bootstrap, dependency assembly, signal handling, and server startup.
- Router construction must be available through a testable `build_router` function.
- HTTP handlers must only authenticate, parse DTOs, invoke services, and map results.
- Subscription commands must live in focused Query, Poll, and Execution application services. Any
  command exposed through both HTTP and the worker must use the same focused service implementation.
- TMDB, Douban, M-Team, qBittorrent, cache, SQLite, and filesystem hardlinking must be adapters.
- Domain modules must not depend on Axum, Reqwest, Rusqlite, `FileConfig`, or upstream response DTOs.
- Blocking SQLite and filesystem operations must run on a blocking executor or dedicated worker.
- All upstream clients must define connect timeout, request timeout, redirect limit, and body limits.
- The shared upstream boundary is implemented through named `HttpClientPolicy` values and a bounded
  streaming response reader. M-Team, TMDB, Douban, qBittorrent, and torrent downloads all use that
  boundary; TMDB no longer uses the generated blocking client. Provider failures use a transport-
  neutral `ClientError` and must not include credentials, request URLs, or response bodies.

### R5. Subscription persistence and scheduling

- Per-record rows must become the only authoritative subscription representation.
- Account-level metadata must remain separate from record data.
- The whole-account state blob and legacy import paths must be removed; only
  `subscriptions.sqlite` participates in runtime storage.
- Repository operations must support `get`, summary listing, detail loading, optimistic update, and
  bounded `claim_due` queries.
- Records must include a revision and scheduling lease/attempt identity.
- A due task must be claimed atomically before external side effects begin.
- `attempt_id` is the scheduler fencing token and `lease_until` bounds live ownership; row revision is
  cache/detail freshness, not attempt identity. Results from a stale/expired attempt must not overwrite
  a newer attempt, while a current attempt must merge its operation-owned delta into the latest payload
  after legitimate Poll/detail revision changes.
- A crashed process must allow leased work to become eligible again after lease expiry.
- qB push and hardlink operations must remain idempotent across retries.

### R6. Subscription lifecycle correctness

- Records absent from the latest complete Douban wanted poll must become inactive.
- Inactive records must not be selected by the scheduler.
- A later reappearance must reactivate the record without losing useful history.
- The latest poll must distinguish a complete source snapshot from a partial/upstream-failed poll.
- A partial snapshot must retain fetched-page/limit/end diagnostics and may merge only the trustworthy
  records actually seen, preserving the established seen-only behavior. It must not advance the last successful
  complete-poll marker/time, deactivate unseen records, or otherwise infer absence from a partial view.
- TV records must not enter an unimplemented executor loop.
- If TV remains enabled, meta/search/progress/link execution and scoped retry must be implemented and
  covered end to end.
- If TV is deferred, creation or scheduling must reject/park it explicitly rather than repeatedly fail.
- Artifact status strings that represent finite state must become enums at domain boundaries.
- Duplicate fields between `DownloadTaskRecord`, `TorrentPushRecord`, and hardlink results must be
  converged into one download artifact and one link artifact.

### R7. API contracts

- Public endpoints must use explicit request and response DTO structs.
- Error responses must contain stable machine-readable codes and human-readable messages.
- Subscription list responses must contain only card/summary fields.
- Subscription details must be loaded by ID and contain heavy candidates/files/artifacts.
- List endpoints must support pagination or bounded cursors.
- API contracts must be expressible as OpenAPI or JSON Schema and consumable by frontend types.
- Contract tests must exercise the real Axum router, status codes, authentication, and response shape.

### R8. Frontend architecture

- `App.vue` must become an application shell with `<RouterView>`.
- Every route must render a real page component and support route-level lazy loading.
- Page modules must own route parameters and orchestration.
- Feature composables/stores must own API state, caching, polling, cancellation, and commands.
- Leaf components must emit user intent rather than receive large sets of function props.
- The shared API client must support timeout, abort signals, stable errors, and latest-request-wins.
- Main media details must render independently of optional M-Team loading.
- Settings form state must be separate from the last saved runtime configuration snapshot.
- Secrets must only be fetched on the authenticated settings route.
- Search and filter state that must survive navigation must live in URL state or an explicit store.
- Dead functions, unused refs, and unreachable styles must be removed.
- DaisyUI theme tokens and custom component primitives must be converged to one styling authority.

### R9. Testing and CI

- Rust formatting, Clippy, unit tests, service tests, and HTTP contract tests must gate pull requests.
- Frontend tests must run through Vitest with importable modules and mounted Vue components.
- At least two Playwright flows must cover navigation and subscription behavior.
- Source-string tests may remain only for narrow manifest invariants that cannot be expressed through
  public behavior; file position and function ordering must not be test contracts.
- CI must run on pull requests as well as publish events.
- Image publication must depend on all quality gates.
- A built container must pass a health/smoke check before publication.

### R10. Deployment and operations

- Docker must build the frontend and backend from a clean checkout in independent stages.
- Durable state must live under `/data/state`, separate from rebuildable caches.
- NAS deployment must mount a shared media root so download and link targets are on one filesystem.
- The service must expose health and readiness endpoints.
- Deployment must document UID/GID or file-permission behavior.
- Operation logs, Docker logs, and caches must have retention/cleanup policies.
- Config and SQLite state must have documented backup, restore, upgrade, and rollback procedures.
- Versioned image tags or digests must be the normal deployment path; `latest` must not be the only
  rollback reference.

### R11. Documentation governance

- A root README must explain purpose, development, verification, and deployment entry points.
- Living documentation must be organized into architecture, API, operations, and ADR sections.
- Every spec/plan must declare `draft`, `accepted`, `implemented`, `superseded`, or `archived` status.
- The old query-parameter detail drawer design must be marked superseded.
- Subscription state convergence must have one authoritative document.
- Completed implementation checklists must not remain as apparently active unchecked plans.
- Documentation formatting and link checks must run in CI.

## Target backend structure

```text
src/
  main.rs
  lib.rs
  app.rs
  http/
    router.rs
    auth.rs
    error.rs
    handlers/
  config/
    model.rs
    service.rs
    store.rs
  subscription/
    model.rs
    movie.rs
    tv.rs
    queries.rs
    execution.rs
    worker.rs
    repository.rs
  media/
    torrent_match.rs
    episode.rs
    hardlink.rs
  clients/
    tmdb.rs
    mteam.rs
    qbittorrent.rs
    douban/
  storage/
    sqlite.rs
    schema.rs
    subscription_repo.rs
    audit_log.rs
    json_cache.rs
```

Expected dependency direction:

```text
HTTP handlers ----\
                   -> application services -> domain + repository/client ports
worker -----------/                              ^
                                                  |
                         SQLite / HTTP / filesystem adapters
```

## Target frontend structure

```text
frontend/src/
  app/
    AppShell.vue
    router.js
  pages/
    SearchPage.vue
    MediaDetailPage.vue
    SubscriptionsPage.vue
    SubscriptionDetailPage.vue
    LogsPage.vue
    SettingsPage.vue
  features/
    search/
    media-detail/
    subscriptions/
    settings/
    logs/
    qb/
  shared/
    api/client.js
    lib/
    ui/
    theme/
  styles/
```

Pinia is not required for the first convergence pass. Feature-level composables or stores are
sufficient until genuinely shared cross-feature state requires a dedicated library.

## Workstreams and dependency order

The accepted implementation plans are:

- [Safety, configuration, and automation](../plans/2026-07-11-safety-configuration-automation.md)
- [Backend application boundaries](../plans/2026-07-11-backend-application-boundaries.md)
- [Subscription storage and scheduler](../plans/2026-07-11-subscription-storage-scheduler.md)
- [Frontend modularization and contracts](../plans/2026-07-11-frontend-modularization-contracts.md)
- [CI, deployment, and documentation](../plans/2026-07-11-ci-deployment-documentation.md)

### Workstream A: Safety, configuration, and operations

Security defaults, authentication, redaction, fail-fast configuration, watcher enablement, state
volume separation, clean Docker builds, backups, and health checks.

This stream begins first because later refactoring must not continue exposing destructive behavior.

### Workstream B: Backend boundaries and application services

Create `lib.rs`, `build_router`, HTTP modules, external clients, domain-safe errors, and shared
application commands. Stop worker-to-handler calls before changing persistence semantics.

### Workstream C: Subscription storage and scheduler

Initialize a fresh latest-schema `subscriptions.sqlite`, establish authoritative per-record storage,
active state, revision, claim/lease, bounded due queries, idempotent outcomes, and summary/detail
queries. Production code does not enumerate, probe, open, read, convert, modify, or delete old
`wanted.sqlite`/JSON state, and all converter/blob compatibility code is deleted.

This stream depends on the application-service seam from Workstream B.

### Workstream D: Frontend modularization and contracts

Introduce the shared API client and importable domain helpers, migrate tests to Vitest, add real route
pages, split feature state, separate settings form/runtime data, and converge styles.

Typed API generation can be introduced after Workstream B defines stable DTOs; route/page extraction
can begin earlier while preserving current API shapes.

### Workstream E: CI, deployment, and documentation governance

Add pull-request gates, Docker smoke tests, multi-stage builds, state/media volumes, operational
runbooks, ADRs, and document lifecycle status. This stream runs alongside A and continues throughout.

## Implementation progress

Last updated: 2026-07-12.

### Safety and configuration

- Evidence: fail-fast/atomic configuration, redacted revision-aware patches, management auth/CORS,
  safe watcher defaults, dry-run, TV parking, complete-only inactive/reactivate behavior, capability
  guards, and runtime mode banners are implemented and tested. Poll failures and staged detail DTOs
  reuse one config-aware diagnostic redactor; URL structure is sanitized before configured-string
  replacement so combined userinfo/query/config-secret leaks are closed. Settings supports redacted
  management-token Set/Clear and immediate HttpOnly-session refresh; protected 401s return to AuthGate.
  Login failures are bounded per direct peer, cookie mutations require same-origin Fetch Metadata, and
  explicit cross-site loopback bootstrap mutations are rejected.
- Remaining gate: execute and record the complete manual management/security smoke matrix in the target
  deployment topology; local behavior tests do not substitute for proxy/NAS acceptance.

### Backend boundaries

- Evidence: `main.rs` is seven lines; router/application seams are in place. Production
  `SubscriptionQueryService` owns list/detail queries and independent query errors, while manual and
  worker Poll share `SubscriptionPollService`. The HTTP adapter has strict parsers, cursor/error
  mapping, recording-fake tests and authenticated production registration; each request derives
  account scope and redaction from one `ConfigManager` snapshot. Unported subscription effect routes
  and the old watcher are deleted. Configuration/operation-log HTTP ownership is extracted, and the
  current subscription domain is reduced to four core enums plus latest repository/Poll/Execution/
  effect contracts. All production
  upstream calls now use AppState-injected named Reqwest/rustls policies with bounded bodies and a
  transport-neutral `ClientError`; qB no longer depends on HTTP error types and response URL metadata
  strips credentials/query/fragment.
- Evidence also includes deletion of the old compatibility command/watcher/store stack. qB, M-Team,
  TMDB and Douban routes plus named success DTOs now live under feature-owned `http/` modules.
  Douban library/tags/QR orchestration is application-owned, QR cookies cannot enter responses, Poll
  has an HTTP-owned DTO, and operation logs map to a closed page that omits account identity and
  filters unstructured metadata. The shared HTTP error boundary no longer lives at crate root, and
  `lib.rs` is a small module/bootstrap facade. `subscription.rs` remains a thin module facade.
- `SubscriptionExecutionService` now owns claim/effect/finish-fail orchestration over the same latest
  repository. Production M-Team/qB/filesystem effects are injected from `AppState`; filesystem work
  uses a bounded blocking executor, while worker batching, concurrency, jitter, backpressure,
  persisted Poll scheduling and graceful cancellation are explicit.
- Poll maps Douban results into provider-neutral wanted snapshot/item values before application
  processing. Execution receives a narrow neutral policy instead of the complete `FileConfig`, and
  the Rusqlite-backed schema manifest is owned by `storage/schema_v5.rs`.
- Bootstrap lock/config loading and latest repository creation, hot configuration persistence,
  SQLite operations and filesystem effects all use bounded blocking boundaries; JSON caches use
  asynchronous filesystem APIs.
- A backend-owned OpenAPI 3.1 document now covers all 27 production methods. A source-derived parity
  gate rejects missing/stale paths, verifies management-session security and requires closed Douban
  and operation-log schemas; generated checked-JS contracts consume those schemas.
- Evidence now includes real production-Router success/failure shapes for TMDB, Douban, M-Team and qB,
  a manual/worker Poll test over the exact same service graph, and a service-level finish-audit failure
  retry proving physical qB add is not repeated.

### Subscription storage

- Evidence: the latest manifest initializes a brand-new `subscriptions.sqlite`; legacy blob/staging
  objects are forbidden and an adjacent `wanted.sqlite` sentinel remains byte-identical. Fresh creation
  builds and validates a unique temporary inode, then atomically publishes without clobber; injected
  failure removes the temporary file and permits an immediate clean retry. Read/Mutation/Poll/Execution
  contracts, typed payload convergence, bounded read/detail CAS and repository-clock fencing are
  implemented. A running detail CAS rejects attention-tag-set or `skip_reason` changes with
  `ExecutionGateConflict`, while other valid detail changes may advance revision without replacing
  attempt identity. Poll implements `begin_poll`, complete, incomplete and failure terminals under
  `BEGIN IMMEDIATE`; exact tokens are consumed atomically, partial snapshots are seen-only, complete
  snapshots deactivate missing rows, and superseded attempts receive one atomic audit. Execution
  implements claim/reclaim, strict-forward extend, exact finish/fail and pre-effect release over
  `(key, operation, attempt_id, lease)`, never claim-time revision. Success/failure merge only
  operation-owned deltas into the newest payload. File-backed rollback tests and four real
  dual-connection Poll-vs-claim/reclaim races prove legal serialization, fencing and same-token retry
  after injected audit failure. Preflight remains read-only, query-only, no-sidecar, DELETE-mode,
  FK/manifest/integrity validating.
- Evidence also includes complete removal of the offline API, migration CLI/converter modules and v4
  SQL/JSON fixture suite; the reusable storage service lock remains independently tested.
- Production bootstrap creates or opens only `subscriptions.sqlite`; the same latest repository now
  serves list/detail, manual Poll, the Poll/Execution worker, readiness and operation logs. Byte-sentinel
  tests prove adjacent old SQLite/JSON files remain untouched, and unported effect routes return 404.
- The effect domain and production adapters derive a domain-separated, length-framed SHA-256 key from
  account, subject, selected torrent and operation. It reconciles qB by stored hash/exact stable tag
  across response loss and crash retry, fails closed on hash/tag disagreement, treats same-inode
  hardlinks as success, preserves conflicting targets, and retries only missing/failed file outcomes.
  The production Execution graph fixes response-loss retries to the persisted torrent identity,
  rejects stale attempt terminals, and never releases an attempt after an effect may have happened.
- Evidence also includes deletion of the aggregate/schema-v4/blob/JSON runtime, whole-account
  load/save, old service/policy/watcher/effect routes and their compatibility tests. Only latest
  forbidden-object manifest literals and byte-sentinel protection tests retain old storage names.
- Backup/restore is exercised by copying only `config.toml` and `subscriptions.sqlite` into an
  independent root and reopening/preflighting the same detail and operation log. Remaining external
  gate: the first GitHub-hosted container startup smoke against `subscriptions.sqlite` only.

### Frontend

- Evidence: all routes are real lazy pages, feature state is modular, and optional Media Detail panels
  no longer block primary loading. Production subscriptions aggregate every bounded opaque-cursor
  summary page before one store commit, preserve backend order through explicit
  `ordered_ids`, reject mixed/invalid/repeated/oversized chains, and expose a strict nested detail-by-ID
  transport. The store keeps one entity authority with separate summary/detail freshness, loads and
  reuses nested detail by revision, retries one summary/detail race, and cancels obsolete per-ID work.
  The detail page renders nested source/observation/issues/candidates/download/link data, handles typed
  missing records, and reloads when polling advances the selected summary. Route IDs use the backend's
  original-value 1..=256-byte contract without trim aliases. Search source/query/page now has one URL
  authority; a backend-owned OpenAPI 3.1 artifact generates the checked-JS contracts, required
  summary fields and enum constants, while a digest/typecheck gate rejects stale output;
  DaisyUI owns shared primitives/theme tokens while search/log styles load with their routes. The
  frontend passes 247 Vitest cases across 43 files plus typecheck/format/lint/build. The legacy
  aggregate/full-projection parser, retry/rerun API/store/UI and client-side fallback sorting are
  deleted. A deterministic same-origin Node fixture now drives four Playwright journeys covering
  search/detail/Back, subscription detail polling without duplicate scheduling or legacy actions,
  authenticated settings redaction, and direct deep-link reload. The eight functional Chromium
  desktop/mobile cases plus 24 visual cases all pass locally; CI adds Firefox desktop and retains
  browser diagnostics.
- Local visual evidence now covers six routes in light/dark at desktop/mobile breakpoints with 24
  non-empty, no-horizontal-overflow screenshots. Remaining gate: confirm and review the first hosted
  Chromium/Firefox quality artifacts.

### Delivery and operations

- Evidence: reproducible gates, gated multi-stage images, separated mounts, living docs/runbooks,
  liveness/readiness, health checks, CI startup/static smoke, failed-gate diagnostic artifacts,
  host-loopback-by-default Compose, per-account operation-log retention, and periodic JSON-cache cleanup
  are present. The generated native-TLS TMDB dependency and OpenSSL image packages are removed; current
  clients use Reqwest/rustls. The housekeeping runbook defines Docker log rotation and explicit offline
  `VACUUM` rules.
- Remaining gate: the first real container smoke in CI and final historical-document governance.

The program remains `in_progress`: latest read/Poll/Execution runtime, effect wiring, old-state
cleanup, focused provider seams, generated API contracts, recovery evidence, Router/provider parity,
service-level audit-failure idempotency and 32 local functional/visual browser cases are complete.
Clean-checkout Git adoption remains the local delivery gate; hosted cross-browser/container and
target NAS/proxy acceptance remain external gates.

## Latest-state constraints

- `config.toml` and `subscriptions.sqlite` are current production data and must never be replaced by
  defaults after a read/validation failure.
- A missing `subscriptions.sqlite` is initialized directly from the exact latest manifest.
- An existing current database must match that manifest or fail closed; startup does not repair it.
- `wanted.sqlite`, `wanted_*.json`, account blobs and old fixtures are unsupported inputs. Production
  code does not enumerate, probe, open, read, convert, modify or delete them.
- No migration CLI, converter, legacy import fallback or blob dual write may remain in production code.
- Binary rollback that cannot read current state requires restoring the matching current-version
  `config/` and `state/` backup; renaming an old database is not a recovery procedure.
- Frontend route extraction must preserve browser Back behavior and detail deep links.
- Search return state and subscription-detail polling must have tests before pages begin unmounting.
- Security changes must include a documented first-login or token-bootstrap path.

## Acceptance criteria

The architecture convergence is complete only when all of the following are true:

### Safety

- A clean install does not expose an unauthenticated management API to the LAN.
- Cross-origin sites cannot read configuration or invoke management actions.
- Malformed TOML leaves the original file untouched and prevents startup.
- Automation is disabled until explicitly enabled.

### Backend

- `main.rs` is limited to bootstrap and assembly.
- Worker and HTTP callers share application services and no worker invokes a handler.
- Domain code does not depend on Axum, Reqwest, Rusqlite, or upstream DTOs.
- Blocking storage/filesystem work does not execute directly on async runtime workers.
- All upstream clients have enforced timeouts.

### Persistence and scheduling

- Subscription records have one authoritative durable representation.
- No normal update rewrites every subscription record.
- Due work is selected through bounded indexed queries and atomically claimed.
- Concurrent manual and background attempts cannot duplicate a push or overwrite a newer attempt.
- Removing an item from Douban wanted makes it inactive and stops future automatic work.
- No TV record can enter an unimplemented one-second error loop.

### API and frontend

- List and detail APIs use typed DTOs and are covered through the real router.
- The frontend uses real route components and `RouterView`.
- API requests can be cancelled and stale responses cannot replace current page state.
- Settings drafts do not alter runtime qB/category state before save.
- Source-layout tests are replaced with behavior tests.
- Dead frontend code and unreachable legacy styles are removed.

### Delivery and operations

- `cargo fmt --check`, Clippy, Rust tests, frontend checks/tests, and production build pass in CI.
- A clean `docker build` includes a fresh frontend build and passes a smoke test.
- State, cache, and media mounts are separated and documented.
- Backup/restore and upgrade/rollback have been tested from documentation.
- Living documentation identifies the authoritative architecture and superseded plans.

## Success metrics

- Zero unauthenticated secret-bearing endpoints.
- Zero silent configuration fallback/writeback paths.
- Zero production worker-to-handler calls.
- Zero whole-state dual writes in steady state.
- Zero scheduler paths that select an unimplemented executor.
- All route pages represented by actual Vue components.
- All required verification commands green before image publication.
- Historical plans clearly marked implemented, superseded, or archived.

## Delivery strategy

The implementation must use separate plans per workstream. Each plan should contain behavior-preserving
preparatory steps, tests that prove the relevant boundary, implementation tasks, current-state checks,
and rollback notes. High-risk storage and routing changes must not be combined with unrelated visual
or dependency cleanups in the same change.
