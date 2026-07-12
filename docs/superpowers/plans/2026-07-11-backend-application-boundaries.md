---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-12
prd: docs/superpowers/specs/2026-07-11-project-architecture-convergence-prd.md
workstream: backend-application-boundaries
implementation_status: in_progress
coordinates_with:
  - docs/superpowers/plans/2026-07-11-subscription-storage-scheduler.md
---

# Backend Application Boundaries Implementation Plan

> **Execution rule:** Complete the required service and safety seams before switching production
> subscription paths to the fresh latest-only repository.
> Each phase must leave the current API behavior runnable and must be committed separately from
> storage-schema or frontend visual changes.

## Goal

Turn the current `src/main.rs`-centred binary into a testable modular monolith with explicit HTTP,
application, domain, repository-port, and adapter boundaries. HTTP handlers and the background
worker must call the same `SubscriptionService`; adapters must return adapter/application errors
rather than Axum errors; blocking SQLite and filesystem work must leave Tokio worker threads; and
all upstream traffic must use constructed clients with explicit timeout, redirect, and body limits.

## Scope

This plan owns:

- `src/lib.rs`, `src/app.rs`, and a public `build_router` seam;
- extraction of HTTP handlers, request/response DTOs, and stable error responses;
- subscription application commands and adapter ports;
- removal of worker-to-handler calls;
- extraction of TMDB, Douban, M-Team, qBittorrent, cache, and hardlink adapters;
- a shared HTTP-client policy with connect/request/redirect/body limits;
- a single blocking-I/O boundary for current rusqlite and filesystem calls;
- service, adapter, and real-router contract tests;
- reducing `src/main.rs` to tracing, dependency assembly, signal handling, bind, and serve.

This plan does **not** change the subscription database schema, claim/lease semantics, scheduling
eligibility, or frontend presentation. Those changes belong to
the [subscription storage and scheduler plan](./2026-07-11-subscription-storage-scheduler.md) after
the service seam is available.

## Required dependency direction

```text
src/main.rs -> app assembly -> HTTP router/worker
                              |             |
                              +---- application services ----+
                                                              |
                                            domain models + ports
                                                              ^
                                                              |
                                SQLite / HTTP / cache / filesystem adapters
```

The following dependencies are forbidden after completion:

- `subscription/model.rs`, `subscription/policy.rs`, and `subscription/ports.rs` importing Axum,
  Reqwest, Rusqlite, `FileConfig`, or a provider response DTO;
- `clients/*`, `media/hardlink.rs`, or `storage/*` importing `http::error::ApiError`;
- `subscription/worker.rs` importing or invoking anything under `http::handlers`;
- handlers calling rusqlite, `std::fs`, provider clients, or domain mutation helpers directly.

## Target file layout

```text
src/
  main.rs
  lib.rs
  app.rs
  http/
    mod.rs
    router.rs
    error.rs
    dto/
      mod.rs
      common.rs
      config.rs
      media.rs
      subscriptions.rs
    handlers/
      mod.rs
      config.rs
      douban.rs
      media.rs
      operation_logs.rs
      qb.rs
      subscriptions.rs
  subscription/
    mod.rs
    error.rs
    model.rs
    policy.rs
    ports.rs
    service.rs
    worker.rs
  media/
    mod.rs
    episode.rs
    hardlink.rs
    torrent_match.rs
  clients/
    mod.rs
    http.rs
    douban.rs
    mteam.rs
    qbittorrent.rs
    tmdb.rs
  storage/
    mod.rs
    blocking.rs
    subscription_repo.rs
    audit_log.rs
    json_cache.rs
tests/
  blocking_boundaries.rs
  router_contract.rs
  subscription_service.rs
  upstream_client_policy.rs
```

During extraction, compatibility re-exports from `subscription/mod.rs`, `clients/mod.rs`, and
`media/mod.rs` are allowed. Delete them in the final phase once all call sites use the target path.

## Phase gates

| Gate              | Required evidence                                                             |
| ----------------- | ----------------------------------------------------------------------------- |
| B0 Baseline       | Current Rust tests pass and existing route/status behavior is characterized.  |
| B1 Router seam    | The binary calls library `build_router`; router tests use `oneshot`.          |
| B2 Typed boundary | Public JSON uses DTOs and stable `{code,message,details?}` errors.            |
| B3 Service seam   | HTTP and worker both call `SubscriptionService`; no worker invokes a handler. |
| B4 Adapter seam   | Domain has no transport/storage dependencies; clients share policy.           |
| B5 Async safety   | SQLite/filesystem operations execute through the blocking boundary.           |
| B6 Convergence    | `main.rs` is bootstrap-only and all final verification commands pass.         |

## Implementation progress

Last updated: 2026-07-12.

- Gate B1 is complete: the binary is seven lines, application paths/state live in `app.rs`, routing
  lives in `http/router.rs`, `build_router` is public, and `tests/router_contract.rs` exercises the
  production router, method handling, security contracts, and SPA fallback with isolated paths.
- A production `SubscriptionQueryService` now owns `ListSubscriptions` and `GetSubscription` over an
  injected `SubscriptionReadRepository`, with transport-independent validation/not-found/unavailable/
  internal query errors. Manual and worker Poll share one injected `SubscriptionPollService`; due
  movie work runs through an injected `SubscriptionExecutionService`. The old
  push/progress/completion/retry/rerun service/policy seam, watcher and compatibility DTOs/tests are
  deleted. The production Execution graph injects M-Team/qB/filesystem effects, uses repository
  clock/attempt fencing and keeps filesystem work behind a bounded blocking executor.
- Gate B2's error half is complete: auth, handlers, JSON/query/path extractors, API 404/405, and
  readiness failures share flat `{code,message,details?}` responses; protected-path 405 handling stays
  behind management auth, malformed inputs use stable generic messages, and 5xx/upstream responses no
  longer expose internal paths, upstream bodies, or TMDB key-bearing URLs. Explicit redacted
  subscription summary/detail DTOs and a bounded scoped-v2 opaque cursor live in split boundary
  modules. The production list/detail HTTP adapter owns strict list/path parsing, cursor/error mapping,
  DTO construction, and recording-fake real-router contracts. Every handler reads exactly one
  `ConfigManager` snapshot and derives both account scope and the concrete config-aware diagnostic
  redactor from that snapshot; a hot-update test proves cursor scope and redaction move together.
  The routes are registered through the authenticated production router and use the latest repository
  from `AppState`; the recording-fake route remains test-only. qB, M-Team and TMDB search/detail now
  have feature-owned handlers and named success DTOs. The shared HTTP error boundary owns provider
  status mapping instead of crate root. Operation logs return their named page type directly, and
  Douban handlers now use named or provider-owned typed success responses.
- Repository contracts are owned transport/storage-neutral types with independent
  object-safe Read, Mutation, Poll, and Execution capabilities. Poll terminals use one
  persisted/consumed token and repository-owned backoff. Complete/incomplete Poll commands carry
  validated seen records and insert-only `NewRecordPolicy`; partial results expose effect counts,
  seen-only semantics, and exact TV parking without adding a global revision-finish promise. The
  latest-schema SQLite adapter implements bounded get/list/detail and a narrow optimistic detail CAS with
  typed revision conflicts. While execution is running, that CAS rejects attention-tag set or
  `skip_reason` changes with `ExecutionGateConflict`, while preserving the same attempt across other
  valid detail revisions. The Poll adapter uses exact account tokens and `BEGIN IMMEDIATE` for
  complete/partial/failure persistence and atomically writes `supersede_attempt` only when complete
  missing or complete/incomplete movie→TV clears a running attempt. The Execution adapter now
  implements `claim_due(limit = 1)`, `claim_one`, expired reclaim, repository clock/nonce injection,
  bounded collision retry, typed rejection, and atomic claim/reclaim audit. Read and Poll are connected
  to production `AppState`; Execution is connected to the worker and real effects. Strict-forward
  lease extension, exact finish/fail and pre-effect release remain repository capabilities, while the
  application service never releases after an effect may have happened.
- Latest-only cleanup reduced `subscription/model.rs` to the four core lifecycle/execution/attention/
  media enums and removed the old aggregate, TV/candidate/push/link compatibility graph plus
  `subscription/{service,policy}.rs`. Repository/Poll/Execution/effect contracts now own the current
  domain vocabulary; Execution runtime port injection is complete.
- Configuration loading and normalization live in `config.rs` behind a transport-neutral validation
  error. The private config DTOs, secret-aware patch merge, handlers, and focused API tests now live in
  `http/config.rs`; the operation-log query DTO and handler similarly live in `http/operation_logs.rs`.
  Both operation-log reads and `app/audit.rs` writes now use the latest repository and shared retention
  policy. qB URLs remain restricted to valid `http`/`https` URLs without userinfo,
  and every patch revalidates the complete candidate configuration. Config persistence is serialized
  and runs through its own bounded blocking executor without holding the state mutex across file I/O;
  failure/cancellation/concurrency and single-thread-runtime responsiveness are covered. All current
  handlers now use named HTTP-owned success DTOs, and focused provider services expose deterministic
  test seams.
- Strict Clippy warnings have been resolved and the current strict command is green. Final real-router
  success/failure coverage, HTTP/worker observability parity and external acceptance keep Gate B6 open.
- The current focused Rust, production-router, health, fresh-storage and upstream-client suites are
  green. The program and this plan remain `in_progress`: latest read/Poll/Execution and filesystem
  boundaries, handler DTOs and provider seams are assembled, while final parity evidence and external
  acceptance remain open.
- `src/lib.rs` is now a 41-line module/bootstrap facade after deletion of the old subscription
  command/watcher/effect stack and extraction of qB, M-Team, TMDB and Douban handlers.

---

## Task 1: Record the Behavior Baseline Before Moving Code

**Files:**

- Modify: `Cargo.toml`
- Create: `tests/router_contract.rs`
- Create: `tests/fixtures/http/`
- Modify: existing tests currently embedded in `src/main.rs`

- [x] Add `tower = { version = "0.5", features = ["util"] }` and `http-body-util = "0.1"`
      as dev dependencies so tests can exercise an Axum `Router` without binding a TCP port.
- [x] Add fixture builders for a temporary config path, TMDB/Douban cache directories, and
      subscription state directory. Tests must never read the repository's real `config.toml` or cache.
- [x] Characterize every current route and method under `/api`, including auth, 404 and 405 behavior,
      through one real production-router matrix covering all declared path/method pairs.
- [x] Lock current successful response shapes through closed HTTP DTO unit tests, generated OpenAPI
      schemas and real-Router provider-family tests without snapshotting secrets.
- [x] Lock error status behavior for malformed JSON/query/path, missing subscription, invalid qB
      request and provider failure using the final flat `{ code, message }` envelope.
- [x] Move behavior-focused async tests out of `src/main.rs` when they can call public library APIs.
      Do not preserve tests that assert function position, source slices, or handler names.

Run:

```bash
cargo test --all-targets
```

Expected: baseline passes before structural extraction. If a pre-existing failure is discovered,
record it in the implementing change before moving files; do not silently update the expectation.

**Rollback:** remove only the new test dependencies and fixtures. No production behavior changes in
this task.

## Task 2: Introduce `lib.rs`, Application Assembly, and `build_router`

**Files:**

- Create: `src/lib.rs`
- Create: `src/app.rs`
- Create: `src/http/mod.rs`
- Create: `src/http/router.rs`
- Modify: `src/main.rs`
- Modify: `tests/router_contract.rs`

- [x] Move module declarations from `main.rs` to `lib.rs`; expose only modules needed by the binary
      and integration tests.
- [x] Introduce `AppState` in `app.rs`. It should hold application services and read-only runtime
      infrastructure handles, not individual handler helpers.
- [x] Add an `AppPaths`/`BootstrapOptions` value containing config, cache, static, and subscription
      paths. Resolve environment variables in bootstrap code, not in handlers.
- [x] Add a deterministic constructor for tests that accepts all paths and fake adapters.
- [x] Add `http::router::build_api_router(state: AppState) -> Router` and
      `build_router(state: AppState, static_dir: PathBuf) -> Router`.
- [x] Move route registration, nesting under `/api`, and static fallback construction into
      `http/router.rs`. Security middleware may be assembled here by the safety workstream, but the
      router function must remain callable in tests.
- [x] Make `main.rs` call `app::bootstrap`, `build_router`, `TcpListener::bind`, and `axum::serve`.
      Do not move business logic into `app.rs` as a substitute for moving it out of `main.rs`.
- [x] Add a test that builds the production router with test state and calls at least one API route,
      one method-not-allowed path, and the SPA fallback through `tower::ServiceExt::oneshot`.

Run:

```bash
cargo test --test router_contract
cargo check --all-targets
```

Expected: the real router is testable without a socket and all existing routes remain reachable.

**Rollback:** the binary can temporarily restore inline router assembly while keeping `lib.rs`, but
do not proceed to later tasks until the public router test passes.

## Task 3: Define Stable HTTP DTOs and Typed Error Mapping

**Files:**

- Create: `src/http/error.rs`
- Create: `src/http/dto/mod.rs`
- Create: `src/http/dto/common.rs`
- Create: `src/http/dto/config.rs`
- Create: `src/http/dto/media.rs`
- Create: `src/http/dto/subscriptions/{mod,cursor,summary,detail,artifacts,tests}.rs`
- Create: `src/subscription/error.rs`
- Modify: `src/http/router.rs`
- Modify: `tests/router_contract.rs`

- [x] Replace provider-shaped values at every public handler boundary with closed named request and
      response structs. qB, M-Team, TMDB, Douban, Poll, operation logs and subscriptions now map
      application/domain outcomes into HTTP-owned DTOs. Binary Douban image endpoints remain explicit
      byte responses rather than JSON wrappers.
- [x] Define the public error body as:

  ```rust
  pub struct ErrorResponse {
      pub code: &'static str,
      pub message: String,
  }
  ```

  The latest error envelope is deliberately closed to `code` plus public `message`; no arbitrary
  `details` value crosses the HTTP boundary.

- [x] Define responsibility-specific application error taxonomies instead of one catch-all enum:
      Query distinguishes validation/not-found/unavailable/internal; Poll adds upstream/conflict;
      configuration distinguishes mutation/stale/persist; focused provider services preserve typed
      `ClientError` timeout/availability/status classes; authentication owns unauthorized/rate-limit.
      Stable public codes remain HTTP-owned in `http/error.rs` so application services stay transport
      independent.
- [x] Map application error variants to HTTP status only in `http/error.rs`. Provider adapters must
      not construct `StatusCode` or `IntoResponse` values.
- [x] Define `SubscriptionSummaryDto`, `SubscriptionDetailDto`, `SubscriptionListResponse`, and a
      bounded scoped-v2 opaque cursor type backed by latest SQL projections.
- [x] Add `From`/mapping functions from domain/application outputs to every public DTO. Do not derive
      direct public serialization on internal service results merely to avoid mappings.
  - [x] Subscription list/detail mappings are explicit, reduce storage/provider-only fields, expose
        retry controls, bind cursors to account/filter scope, and require diagnostic redaction.
  - [x] qB, M-Team and TMDB success mappings are explicit, named and feature-owned.
  - [x] Douban, Poll and operation-log success mappings no longer serialize provider/domain values
        directly. Library/tags use closed DTOs, QR cookies cannot enter a response, and operation-log
        account identity/arbitrary nested metadata are filtered by an explicit DTO mapper.
- [x] Return `Content-Type: application/json` and the stable error body for extractor rejections as
      well as handler errors. Add a fallback mapper for malformed JSON and query parameters.
- [x] Update router tests for malformed JSON/query/path, semantic handler errors, API 404/405,
      protected method handling, exact error codes, and absence of submitted/internal secret values.
  - [x] Implement subscription list/detail query/path parsing, scoped cursor errors, stable
        400/404/500/503 mappings, redacted success DTOs, and recording-fake router tests.
  - [x] Derive account scope and the diagnostic redactor once per request from the same live
        `ConfigManager` snapshot; hot configuration changes must not mix old scope with new secrets.
  - [x] Register list/detail in the production router with the latest repository injected through
        `AppState`.

Run:

```bash
cargo test --test router_contract
cargo test --all-targets
```

Expected: no public handler returns the legacy `{ "error": ... }` body, and response contracts no
longer depend on internal aggregate serialization.

**Rollback:** preserve the DTO types and revert individual handler mappings if necessary. Do not
restore adapter dependencies on HTTP errors.

## Task 4: Extract Domain Models, Policies, and Ports

**Files:**

- Replace: `src/subscription.rs` with `src/subscription/mod.rs` plus extracted modules
- Create: `src/subscription/model.rs`
- Create: `src/subscription/policy.rs`
- Create: `src/subscription/ports.rs`
- Create: `src/subscription/service.rs`
- Modify: `src/config.rs`
- Modify: `src/douban.rs`
- Modify: tests formerly embedded in `src/subscription.rs`

**Implementation status: complete (Git adoption pending).** The intermediate compatibility slices have been
superseded by latest-only cleanup. `subscription/model.rs` now retains only the four core enums used by
latest DTO/repository contracts; old aggregate/failure/TV/candidate/push/link records and
`subscription/{service,policy}.rs` are deleted. Current work should extend the latest repository,
worker and effect ports rather than recreate those serialized compatibility types. Poll source values
are provider-neutral `WantedSnapshot`/`WantedItem` objects mapped by an isolated Douban adapter;
Execution accepts a narrow neutral policy instead of `FileConfig`. The Rusqlite-backed latest schema
manifest now belongs to `storage/schema_v5.rs`, outside the subscription namespace.

- [x] Keep only latest lifecycle enums and provider-neutral subscription/effect inputs in domain
      modules. The old serialized aggregate and compatibility record move is superseded by latest-only
      cleanup and must not be recreated.
- [x] Replace `SubscriptionWatcherConfig` arguments in Poll/Execution service rules with narrow
      application policies containing only the required account, interval, retry and effect fields;
      configuration snapshots are mapped before service/effect invocation.
- [x] Replace direct `DoubanLibraryItem` dependencies in Poll with provider-neutral
      `WantedSnapshot` and `WantedItem` values. Mapping belongs in the Douban adapter.
- [x] Define object-safe async ports for latest repository Read/Mutation/Poll/Execution, wanted
      source, execution effects/qB/filesystem, audit and clock responsibilities. Focused TMDB,
      M-Team, Douban and manual-qB services now also wrap live clients behind deterministic provider
      seams without adding `async-trait`.
- [x] Remove the temporary aggregate persistence adapter after the latest repository, revision and
      claim/lease contracts are proven.
- [x] Keep pure torrent matching and hardlink layout tests next to their modules. TV remains
      explicitly unsupported, so the old episode-execution parser was deleted rather than retained as
      dead code. Tests call behavior rather than inspect file text.
- [x] Add a compile/check acceptance command that proves domain modules contain no forbidden imports:

  ```bash
  rg -n 'axum|reqwest|rusqlite|FileConfig|DoubanLibrary(Item|List)|QbTorrent' \
    src/subscription/model.rs src/subscription/policy.rs src/subscription/ports.rs
  ```

  Expected: no matches. This is an implementation review gate, not a source-layout unit test.

Run:

```bash
cargo test subscription::
cargo check --all-targets
```

**Rollback:** compatibility re-exports may keep old import paths alive for one phase. Revert the
consumer migration before reverting model extraction so serialized compatibility is not lost.

## Task 5: Build Shared Upstream Clients with Enforced Policies

**Implementation status: complete for the upstream-policy slice.** `AppState` now owns the
long-lived Douban, M-Team, TMDB, and torrent-download clients. qB must retain a cookie jar and may
use different TLS verification settings per configured server, so it constructs a short-lived,
policy-bound authenticated session through the same factory for each operation. There are no
remaining production providers using an unbounded Reqwest response or a generated blocking client.
`src/douban.rs` remains the parsing/domain facade; provider-specific request construction and every
network send live behind `clients/douban.rs` and the shared bounded executor.

**Files:**

- Create: `src/clients/mod.rs`
- Create: `src/clients/http.rs`
- Create: `src/clients/douban.rs`
- Create: `src/clients/mteam.rs`
- Create: `src/clients/qbittorrent.rs`
- Create: `src/clients/tmdb.rs`
- Modify: `src/douban.rs`
- Modify: `src/qbittorrent.rs`
- Modify: `src/app.rs`
- Modify: `Cargo.toml`
- Create: `tests/upstream_client_policy.rs`

- [x] Introduce a `HttpClientPolicy` with explicit connect timeout, total request timeout, redirect
      limit, and response-body byte limit. Give each provider a named policy; do not rely on Reqwest
      defaults.
- [x] Construct long-lived Reqwest clients once during application bootstrap and inject them into
      adapters. QR sessions that need cookie isolation may construct a bounded session client through
      the same factory.
- [x] Add a bounded response reader that checks `Content-Length` when present and reads chunks only
      up to the configured limit when it is absent or inaccurate.
- [x] Move all M-Team request code out of the application facade into `clients/mteam.rs`.
- [x] Move qB request code into `clients/qbittorrent.rs`, change its error type to `ClientError`, and
      make HTTP/application layers map it. Delete `use crate::ApiError` from the adapter.
- [x] Move Douban client construction and transport code into `clients/douban.rs`; keep HTML/JSON
      parsing pure and independently tested.
- [x] Implement `clients/tmdb.rs` as the only TMDB transport boundary. The generated `tmdb_client`
      default blocking client cannot enforce a body cap, so either inject a configured blocking client
      for the temporary transition and wrap every call in a timeout, or replace the small used endpoint
      set with bounded Reqwest calls. The phase is not complete until timeout and body-limit tests cover
      TMDB too.
- [x] Normalize upstream failures into timeout, body-too-large, authentication, rate-limited,
      protocol, and unavailable variants without exposing credentials or response bodies in messages.
- [x] Add local stub-server tests proving connect/request timeout, redirect limit, and body cap.
      Tests must not use the public network.

Run:

```bash
cargo test --locked --offline --test upstream_client_policy
cargo test --locked --offline clients::
```

Expected: `rg -n 'Client::new\(|Client::builder\(' src --glob '*.rs'` finds the sole Reqwest
builder in `clients/http.rs` (plus wrapper construction and test-only calls), while
`rg -n 'ApiError|StatusCode|axum' src/clients src/qbittorrent.rs` has no matches.

**Rollback:** retain adapter interfaces and switch one provider implementation back at a time. Do
not let a rollback reintroduce `ApiError` into an adapter.

## Task 6: Make `SubscriptionService` the Only Command Orchestrator

**Files:**

- Modify: `src/subscription/service.rs`
- Create: `src/subscription/worker.rs`
- Create: `src/http/handlers/subscriptions.rs`
- Create: `src/http/handlers/operation_logs.rs`
- Modify: `src/http/router.rs`
- Modify: `src/app.rs`
- Modify: `src/main.rs`
- Create: `tests/subscription_service.rs`

**Implementation status: complete (Git adoption pending).** Production list/detail use `SubscriptionQueryService`,
manual/worker Poll share `SubscriptionPollService`, and the worker uses
`SubscriptionExecutionService` over the latest repository. Unported
push/progress/completion/retry/rerun routes, their old service/policy stack and the watcher are deleted.
Execution is a worker-owned application service with real qB/filesystem ports, bounded batches,
attempt-fence tests and graceful cancellation. Manual effect routes remain intentionally absent rather
than restoring the old contracts. Manual qB push, M-Team search and TMDB media catalog now use focused
application services; request normalization, provider-response parsing, caching and audit
orchestration no longer live in those HTTP handlers. Douban search/detail/interest/library/tags/QR
orchestration now lives in `DoubanCatalogService`; image proxying remains an explicit binary adapter.

- [x] Define the latest-only command surface through focused Poll, list/detail Query, and bounded
      Execution services. Old manual push/progress/link/retry/rerun commands stay deleted rather than
      being recreated for compatibility.
  - [x] `ListSubscriptions` and `GetSubscription` run behind production `SubscriptionQueryService`
        with independent query errors.
  - [x] Manual and worker Poll use the same `SubscriptionPollService`.
- [x] Move remaining provider/audit orchestration into focused application services. A
      handler authenticates/extracts context, validates a DTO, invokes one service method, maps the
      result, and returns. qB, M-Team, media/TMDB and Douban are complete.
- [x] Inject repository/client/hardlink/audit ports into Execution services. Tests replace
      each with deterministic fakes and a fake clock.
- [x] Move the worker loop to `subscription/worker.rs`. It owns persisted Poll scheduling and cancellation logic,
      but every poll and due operation must be a service command.
- [x] Delete calls equivalent to
      `wanted_subscription_push(State(...), Path(...), Json(...)).await` from worker code. The worker
      must never parse a handler's JSON result to decide whether to run the next command.
- [x] Return typed service outcomes such as `ProgressOutcome::Downloading` and
      `ProgressOutcome::ReadyToLink`; keep HTTP response construction in the handler.
- [x] Preserve one operation-log write per semantic attempt. A service-level fail-once finish-audit
      test proves SQLite rollback, deterministic reclaim with a new attempt, stable production qB
      idempotency identity, two effect executions but one physical add, and exactly one successful
      terminal audit after retry.
- [x] Add service tests proving manual and worker Poll callers use the exact same service/repository/
      source graph, focused provider failures produce one typed failure audit, and Execution persists
      typed finish/failure outcomes without depending on HTTP JSON.

Run:

```bash
cargo test --test subscription_service
cargo test --test router_contract subscriptions
rg -n 'http::handlers|wanted_subscription_(push|progress|completion)\(' \
  src/subscription/worker.rs src/subscription/service.rs
```

Expected: tests pass and the final `rg` reports no handler dependency/call.

**Rollback:** route both HTTP and worker through a temporary service method that delegates to old
helpers. Never rollback by making the worker call an Axum handler again.

## Task 7: Put SQLite and Filesystem Work Behind a Blocking Boundary

**Files:**

- Create: `src/storage/mod.rs`
- Create: `src/storage/blocking.rs`
- Create: `src/storage/subscription_repo.rs`
- Create: `src/storage/audit_log.rs`
- Create: `src/storage/json_cache.rs`
- Create: `src/media/hardlink.rs`
- Modify: `src/tmdb_cache.rs`
- Modify: `src/subscription/service.rs`
- Modify: `src/app.rs`
- Create: `tests/blocking_boundaries.rs`

**Implementation status: complete.** `storage/blocking.rs` provides a reusable semaphore-bounded
`spawn_blocking` executor. Latest SQLite, production hardlink effects, and configuration persistence
use bounded blocking boundaries; JSON caches use `tokio::fs`. Current-thread tests prove Tokio
responsiveness, detached-work permit ownership after caller cancellation, and independence of
separate executor pools. Bootstrap lock/config loading and fresh/existing latest repository creation
also run through bounded blocking executors before async preflight.

- [x] Introduce a bounded blocking executor wrapper over `tokio::task::spawn_blocking` guarded by a
      semaphore. Separate SQLite and filesystem concurrency limits so a large hardlink plan cannot
      starve database access.
- [x] Move latest production `Connection::open`, query, transaction, schema initialization, and
      rusqlite row parsing into closures executed by the SQLite blocking executor.
- [x] Move hardlink directory creation, metadata/canonicalize checks, and `std::fs::hard_link` into
      the filesystem blocking executor. Pure plan construction remains synchronous domain code.
- [x] Move any synchronous cache directory/file operation not already using `tokio::fs` into the
      same boundary or convert it to `tokio::fs`.
- [x] Ensure spawned closures own their inputs and return typed adapter results. Do not hold a Tokio
      `RwLock` guard across `spawn_blocking` or an upstream await.
- [x] Add a test with a single-thread Tokio runtime and a deliberately blocked fake storage closure;
      a timer/fake HTTP future must still make progress.
- [x] Add cancellation semantics: cancelling a caller does not pretend a blocking operation was
      cancelled; its result may be discarded, and idempotent retry behavior is required from the
      storage/scheduler plan.

Run:

```bash
cargo test --test blocking_boundaries
cargo test --all-targets
```

Expected: no production async handler/service invokes rusqlite or `std::fs::hard_link` directly.

**Rollback:** reduce executor concurrency to one if contention appears. Do not move blocking work
back onto Tokio runtime workers.

## Task 8: Extract Remaining Handlers and Shrink the Compatibility `lib.rs`

**Files:**

- Modify: `src/http/config.rs`
- Create: `src/http/douban.rs`
- Create: `src/http/media.rs`
- Create: `src/http/qb.rs`
- Create: `src/http/subscriptions.rs`
- Create: `src/media/episode.rs`
- Create: `src/media/torrent_match.rs`
- Modify: `src/http/mod.rs`
- Modify: `src/http/router.rs`
- Modify: `src/app.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Modify: existing unit/integration tests

**Implementation status: complete (Git adoption pending).** Feature-owned HTTP modules now exist directly
under `src/http/`. `http/subscription_queries.rs` owns production list/detail and
`http/subscriptions.rs` owns manual Poll. Candidate/push/retry/rerun/progress/completion routes and
their compatibility runners are deleted; real-router contracts prove those paths return 404. State,
Poll, readiness and operation logs use the latest repository. qB, M-Team, TMDB and Douban live in
their feature modules with named success boundaries, the crate-root HTTP error type is deleted, and
`lib.rs` is only a module/bootstrap facade.

- [x] Move each remaining route handler into the feature file that owns its transport contract.
- [x] Keep torrent matching and filename/path sanitization/hardlink planning in pure
      execution-effect modules, with provider response conversion isolated from handlers. TV episode
      execution remains absent because TV is hard-parked as `tv_not_supported`.
- [x] Remove temporary subscription compatibility re-exports and dead helper functions after call sites have
      migrated.
- [x] Keep `app.rs` responsible for dependency construction and worker lifecycle only. It must not
      accumulate handler or domain logic removed from `main.rs`.
- [x] Reduce `main.rs` to tracing setup, options/path resolution, bootstrap, signal handling, bind,
      and serve. Target fewer than 150 non-test lines and no route path strings.
- [x] Move all behavior tests out of `main.rs`; a tiny bootstrap/path-resolution unit test module is
      acceptable.

Run:

```bash
rg -n '\.route\(|Json<|rusqlite|reqwest|hard_link|WantedSubscriptionRecord' src/main.rs
wc -l src/lib.rs src/main.rs
cargo test --all-targets
```

Expected: the first command has no matches, `main.rs` remains bootstrap-only, `lib.rs` keeps shrinking
as feature owners absorb handlers/helpers, and tests pass.

**Rollback:** revert one extraction commit at a time. Do not combine the rollback with schema,
configuration, or frontend changes.

## Task 9: Final Boundary and Contract Verification

**Files:**

- Modify: `tests/router_contract.rs`
- Modify: `tests/subscription_service.rs`
- Modify: `tests/upstream_client_policy.rs`
- Modify: `tests/blocking_boundaries.rs`
- Modify: `docs/superpowers/plans/2026-07-11-backend-application-boundaries.md`

- [x] Exercise every production route/method through the real Router authentication/405 matrix; lock
      closed success/failure JSON through real Router tests for each provider family plus
      config/subscription/log/health/auth suites. The source-derived OpenAPI gate covers all 27
      production methods and every closed object schema.
- [x] Prove the manual Poll adapter and worker use the same Poll service/repository/source graph; the
      HTTP handler contains only config-to-policy mapping, one service call and DTO/error mapping.
- [x] Prove all external clients enforce their named timeout/redirect/body policies.
- [x] Prove the runtime remains responsive while SQLite/filesystem work is blocked.
- [x] Confirm domain files contain none of the forbidden dependencies listed above.
- [x] Confirm worker/service files contain no handler imports and adapters contain no HTTP error type.
- [ ] Mark this plan `implemented` only after the commands below are green on a clean checkout.

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo check --release
```

## Rollout and Rollback

1. Merge Tasks 1-3 as a router/contract change with no storage change.
2. Merge Tasks 4-6 as domain/application extraction; keep the current repository adapter.
3. Merge Tasks 7-8 as adapter/blocking extraction.
4. Compare operation logs and subscription outcomes for manual versus watcher commands in a staging
   data copy before enabling the next storage plan.
5. If a regression appears, rollback only the most recent phase. Because this plan does not change
   durable schema, the previous binary remains data-compatible.
6. Production read/Poll cutover may proceed only through the verified application seams; Execution
   cutover still requires Gate B6 and zero worker-to-handler production call sites.
