---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-12
implementation_status: in_progress
spec: docs/superpowers/specs/2026-07-11-project-architecture-convergence-prd.md
---

# Latest Subscription Storage and Scheduler Plan

## Goal

Use one fresh, latest-schema SQLite database named `subscriptions.sqlite` as the only subscription
authority. HTTP and the worker must share application services over bounded repository ports;
selection, execution and terminal persistence must remain atomic, fenced and recoverable.

The previous `wanted.sqlite`, `wanted_*.json`, account blobs and schema-v4 fixtures are out of scope.
Production code never enumerates, probes, opens, reads, converts, modifies or deletes them.

## Non-goals

- No v4-to-v5 migration, import fallback, converter, backup command or migration CLI.
- No blob dual write or compatibility runtime.
- No microservice, Redis, external queue or distributed scheduler.
- No TV executor until it is implemented end to end; TV remains visible but `tv_not_supported`.
- No caller-supplied authoritative clock for Poll or Execution ownership.

## Target boundaries

```text
HTTP handlers ----\
                   -> SubscriptionService / SubscriptionQueryService
worker -----------/          |
                              v
                 Read | Mutation | Poll | Execution ports
                              |
                              v
                    latest SQLite adapters
```

The runtime database contains only:

- account Poll metadata and exact open Poll token;
- one row per subscription with summary projections and one validated detail payload;
- revision for cache/detail freshness;
- attempt ID, operation and lease for scheduler fencing;
- operation logs and current download/link artifacts;
- indexes and CHECK constraints required by bounded queries.

## Settled contracts

### Database identity

- Canonical file: `subscriptions.sqlite` under `SUBSCRIPTION_STATE_DIR`.
- A missing file is initialized directly from the latest manifest.
- Production startup does not enumerate, probe or open `wanted.sqlite` or `wanted_*.json`.
- An existing `subscriptions.sqlite` must exactly match the latest manifest or fail closed.
- Health/readiness may validate the current file but must not repair or rewrite it.

### Read and mutation

- Summary listing is bounded and cursor-scoped by account and filters.
- Detail loads by strict original-value ID and returns the nested DTO.
- One entity row is authoritative; account-wide snapshots are not runtime storage APIs.
- Detail CAS uses revision only for freshness.
- While execution is running, attention-tag set and `payload.skip_reason` cannot change through the
  generic detail command; other valid source/detail changes may advance revision.

### Poll

- `begin_poll` creates one exact account Poll token.
- Complete, incomplete and failure terminals consume that token exactly once.
- Partial snapshots merge only trustworthy seen rows and never infer absence.
- Complete snapshots deactivate missing rows and reactivate returned rows.
- Removing a live/expired execution attempt due to missing or movie-to-TV change writes one
  `subscription_scheduler / supersede_attempt / success` audit in the same transaction.

### Execution

- `attempt_id` is the fencing token; revision is not attempt identity.
- Repository clock defines lease liveness: `lease_until > now` is live, equality is expired.
- Claim priority is expired lease, explicit force, then normal due.
- Force may bypass future due/retry/skip, but not inactive, TV, completed, blocked or live lease.
- Claim/reclaim preserve due and force; exact terminal success/failure consumes force.
- Extend is strict-forward and keeps attempt identity.
- Finish/fail merge only operation-owned payload deltas into the latest row.
- Before-effect release requires the exact live attempt and preserves due/force.
- Row, payload and audit commit atomically; stale/expired attempts cannot overwrite a newer owner.

## Current implementation evidence

- Strict latest schema manifest, Read/Mutation/Poll/Execution contracts and staged SQLite adapters
  exist.
- Summary/detail queries, strict cursor parsing and nested detail DTOs are implemented behind staged
  query services and HTTP routes.
- Frontend list/detail cache now consumes the nested detail contract with revision-aware freshness,
  cancellation, stale-response isolation and typed missing-record state.
- Poll implements begin/complete/incomplete/failure with atomic token consumption and supersede audit.
- Execution implements claim/reclaim, extend, finish, fail and before-effect release with exact
  attempt/lease fencing and terminal audit rollback.
- Running detail CAS rejects execution-gate changes without blocking unrelated valid detail updates.
- The effect domain/port/fake foundation derives framed SHA-256 qB keys, reconciles stored hash and
  stable tag after response loss or crash, fails closed on hash/tag disagreement, and models
  deterministic non-destructive hardlink retry down to per-file outcomes.
- Real M-Team/qB/filesystem adapters are connected through `SubscriptionExecutionService`; qB add
  reconciles stable tag/hash, progress observes the same task, and hardlinks run in a bounded blocking
  executor with per-file retry.
- Production bootstrap now creates or opens only `subscriptions.sqlite`; `AppState` injects the latest
  repository into summary/detail queries, manual Poll, readiness, operation logs and a bounded
  Poll/Execution worker. Unported effect routes are absent from production.
- The old aggregate/schema-v4/blob/JSON store, watcher/service/policy stack, test-only effect routes and
  compatibility DTOs/tests are deleted. Execution claim/effect orchestration, batching,
  concurrency, jitter, backpressure, persisted Poll scheduling and cancellation are implemented.
- The current Rust baseline is 302 unit tests, 9 production-router contracts and 9 upstream-client
  contracts, all green under the locked offline all-target gate.

## Gates

| Gate | Requirement | Status |
| --- | --- | --- |
| L0 Fresh schema | `subscriptions.sqlite` initializes from latest manifest; old files stay untouched. | Complete in repository and production bootstrap |
| L1 Repository | Read/Mutation/Poll/Execution adapter tests cover current contracts. | Complete |
| L2 Concurrency | Real two-connection Poll-vs-claim and claim-vs-claim races are deterministic. | Complete, including poison/collision exhaustion |
| L3 Runtime | HTTP and worker use the same injected services and latest repository. | Complete for registered latest routes and worker |
| L4 Effects | qB and hardlink side effects are idempotent across response loss/crash/retry. | Complete in domain, real adapters and production Execution wiring |
| L5 Cleanup | Old store, blob, migration, legacy artifacts and fallback APIs are absent. | Complete; only manifest/sentinel-test literals remain |
| L6 Operations | Backup/restore, bounded queries, retention and startup smoke use latest state only. | Local evidence complete; first hosted container run pending |

## Task 1: Fresh latest-schema initialization

**Primary files**

- `src/storage/schema_v5.rs`
- `src/storage/sqlite.rs`
- `src/storage/subscription_repo.rs`
- `src/app.rs`

- [x] Add one manifest-backed initializer for a new `subscriptions.sqlite`.
- [x] Create through a private temporary path and atomically publish only after full validation, or
      create under an exclusive service lifecycle gate with equivalent fail-closed guarantees.
- [x] Set foreign keys, canonical journal mode, schema marker, tables, indexes, CHECK constraints and
      triggers directly to the latest contract.
- [x] Do not create `subscription_state_blobs`, `subscription_state_blobs_legacy_v4` or legacy guards.
- [x] Open existing current databases without `CREATE TABLE IF NOT EXISTS` repair behavior.
- [x] Add a test proving an existing sentinel `wanted.sqlite` remains byte-identical while
      `subscriptions.sqlite` is initialized and used.
- [x] Prove production repository open ignores byte-sentinel `wanted.sqlite` and `wanted_*.json`.
- [x] Make readiness distinguish missing/uninitialized current state from corrupt current state
      without consulting old files.

## Task 2: Complete bounded repository capabilities

- [x] Implement bounded summary list and strict detail-by-ID reads.
- [x] Implement optimistic detail CAS and running execution-gate conflict.
- [x] Implement complete/incomplete/failure Poll terminals.
- [x] Implement claim/reclaim with repository clock, nonce and bounded collision retry.
- [x] Implement strict-forward lease extension.
- [x] Implement exact finish/fail and before-effect release.
- [x] Replace remaining aggregate mutation with task-specific commands.
- [x] Add a generated 10,000-row query-plan assertion for list, due and expired-lease indexes.
- [x] Prove updating one record does not rewrite unrelated records or account metadata.

## Task 3: Prove concurrency and fencing

- [x] Prove two repositories cannot claim the same row concurrently.
- [x] Prove reclaim makes every old-token extend/finish/fail/release stale.
- [x] Prove Poll/detail revision changes do not prevent the exact live attempt from finishing.
- [x] Prove audit/meta/count failures roll back token, row and payload together.
- [x] Add a true simultaneous two-connection Poll-versus-claim race using deterministic barriers or
      equivalent execution hooks, not a merely sequential interleaving.
- [x] Cover complete missing, movie-to-TV, incomplete seen, expired reclaim and exact lease equality
      in the real race suite.
- [x] Add malformed forced/expired poison-row cases and collision exhaustion with a second eligible
      candidate to prove fail-closed behavior.

## Task 4: Inject the latest runtime graph

**Primary files**

- `src/app.rs`
- `src/subscription/service.rs`
- `src/subscription/worker.rs`
- `src/http/subscriptions.rs`
- `src/http/subscription_queries.rs`

- [x] Put the latest repository plus query and Poll services in `AppState`.
- [x] Make production list/detail/Poll handlers authenticate, parse DTOs, invoke one service and map
      one result.
- [x] Move Poll and due Execution ownership into `subscription/worker.rs` application-service calls.
- [x] Use the same Poll service for manual and worker calls.
- [x] Limit each worker tick to a configurable batch; add jitter, backpressure and cancellation.
- [x] Automation-disabled startup serves reads and manual Poll but does not claim work.
- [x] Register production summary/detail routes only with the latest repository graph.
- [x] Remove the aggregate snapshot and unported effect endpoints from the production router.

## Task 5: Make external effects idempotent

- [x] Replace mutable duplicated status strings with one download artifact and one link artifact.
- [x] Derive a stable qB idempotency key from account, subscription, selected torrent and operation.
- [x] Reconcile qB by stored hash and stable tag before adding a torrent in the domain/port layer.
- [x] Model `qB accepted -> response/database finish lost`; fake retry finds the existing task.
- [x] Keep hardlink targets deterministic; same-inode targets are success and conflicting targets are
      explicit non-destructive failures.
- [x] Model per-file link outcomes and skip already verified files on retry.
- [x] Reject stale-attempt effect results while allowing the next attempt to reconcile them.
- [x] Connect the foundation to the real qB adapter and filesystem executor.
- [x] Add real-adapter/temporary-filesystem crash/retry tests.

## Task 6: Remove old storage and migration code

- [x] Delete `src/bin/subscription-migrate-v5.rs` and remove it from the runtime image contract.
- [x] Delete `src/subscription/migration.rs` and `src/subscription/migration/`.
- [x] Delete v4 SQL/JSON fixtures and `tests/subscription_v4_fixtures.rs`.
- [x] Replace `src/offline.rs` with the reusable `storage/service_lock.rs`, delete the migration API
      and retain only the tested single-service ownership lock.
- [x] Delete blob DDL, blob reads/writes, account-wide repair and legacy JSON import from runtime.
- [x] Delete duplicate legacy artifact/status fields after the current artifact model is wired.
- [x] Delete the inert `AppState.wanted_store`, test-only legacy effect router, old watcher/command
      stack and their compatibility tests.
- [x] Confirm production paths contain no old-file open/import or blob access. Old names remain only in
      the latest forbidden-object manifest, sentinel/protection tests and documentation that says they
      are ignored.

## Task 7: Complete API and frontend adoption

- [x] Define strict summary, list and nested detail DTOs.
- [x] Implement frontend revision-aware nested detail loading and cache invalidation.
- [x] Register latest list/detail routes in the production router.
- [x] Remove the frontend aggregate/full-projection loader and retry/rerun action compatibility code.
- [x] Check API types and reject missing required fields/lifecycle values at build time; generated
      backend artifacts remain open.
- [x] Exercise list pagination, detail reload, Poll-driven detail invalidation and missing-record UI
      through real router/component contracts.

## Task 8: Operations and performance

- [x] Scope operation-log retention by account and keep cleanup in the insert transaction.
- [x] Remove expired JSON cache entries without touching state/config files.
- [x] Back up and restore `subscriptions.sqlite` with the matching config; deployment runbooks retain
      the matching image digest as the operator contract.
- [x] Add a 1,000-record write-amplification assertion and 10,000-row indexed query smoke.
- [x] Keep `VACUUM` explicit and offline; never run it during startup.
- [ ] Container smoke must prove the migration binary is absent, the current DB is healthy and a
      sentinel old `wanted.sqlite` remains unchanged.

## Task 9: Final acceptance

Run on the latest-only worktree:

```bash
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline --all-targets
npm run verify:frontend
git diff --check
```

Also require:

```bash
rg -n 'subscription-migrate-v5|migrate_offline_v4' src tests Dockerfile
rg -n 'subscription_state_blobs|wanted\.sqlite|wanted_.*\.json' src tests
```

Expected: the migration-entry search is empty. Old storage names may appear only in the latest
manifest's forbidden-object list, no-clobber/sentinel tests, and comments that state they are ignored;
there must be no production open/query/write/import path. Documentation may mention old filenames only
to state that they are ignored and unsupported.

Mark this plan `implemented` only after Gates L0-L6 have direct executable evidence and no old storage
path remains in production code.
