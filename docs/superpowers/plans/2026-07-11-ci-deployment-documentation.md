---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-12
prd: docs/superpowers/specs/2026-07-11-project-architecture-convergence-prd.md
workstream: ci-deployment-documentation
implementation_status: in_progress
depends_on:
  - docs/superpowers/plans/2026-07-11-safety-configuration-automation.md
---

# CI, Deployment, and Documentation Governance Implementation Plan

## Goal

Make a clean checkout independently verifiable and buildable, prevent failing code from being
published, separate durable state from caches, provide a correct NAS media mount, and replace the
current collection of unchecked historical plans with living operational and architecture docs.

## Scope

- Pull-request quality gates for Rust, frontend, documentation, and container smoke tests.
- A self-contained multi-stage Docker build.
- Health/readiness endpoints and a container health check.
- Separate config, state, cache, and shared media mounts.
- Backup, restore, upgrade, rollback, security, and troubleshooting documentation.
- Root README and document lifecycle metadata.
- Retention guidance for operation logs, caches, and container logs.

Authentication semantics and configuration behavior are implemented by the safety plan. Backend
module movement and typed API work are implemented by their dedicated plans.

See the [safety/configuration plan](./2026-07-11-safety-configuration-automation.md) and the
[backend-boundary plan](./2026-07-11-backend-application-boundaries.md) for those implementation
details.

## Target files

- Modify: `package.json`
- Modify: `Cargo.toml`
- Create: `.github/workflows/quality.yml`
- Modify: `.github/workflows/docker-publish.yml`
- Modify: `Dockerfile`
- Modify: `.dockerignore`
- Modify: `deploy/nas/docker-compose.yml`
- Replace: `deploy/nas/README.md`
- Create: `README.md`
- Create: `docs/architecture/overview.md`
- Create: `docs/architecture/data-storage.md`
- Create: `docs/operations/configuration.md`
- Create: `docs/operations/backup-restore.md`
- Create: `docs/operations/upgrade-rollback.md`
- Create: `docs/operations/security.md`
- Create: `docs/operations/troubleshooting.md`
- Create: `docs/adr/0001-standalone-detail-routes.md`
- Create: `docs/adr/0002-subscription-state-convergence.md`
- Create or modify backend health handler/router files introduced by the backend-boundary plan.

## Implementation progress

Last updated: 2026-07-11.

- Reproducible verification scripts/toolchains and strict Clippy cleanup are complete. Pull requests
  and `main` run Rust, frontend, documentation, and container-build jobs; image publication depends
  on that reusable quality workflow.
- The Docker build is self-contained and multi-stage. Container state/cache/media paths, tag
  selection, configurable UID/GID, and log rotation are represented in Compose and the NAS docs.
- Root README, architecture/storage docs, operations runbooks, ADRs, and lifecycle frontmatter are
  present. Recovery instructions exist but have not yet been exercised as a recorded deployment
  drill. All six superseded specs/plans now live under `docs/archive/` with non-executable warnings;
  five implemented historical documents remain in the living tree intentionally.
- CI preserves seven-day Rust/frontend/documentation/container diagnostics on failed gates. Compose
  now binds the host loopback by default, with an explicit `HOST_BIND_IP` opt-in for authenticated LAN
  exposure.
- Task 4 is implemented: unauthenticated minimal health/readiness routes, JSON 503 readiness errors,
  immutable/no-sidecar SQLite checks under the storage service lock, bounded cached probing, a Docker
  `HEALTHCHECK`, real-router tests, and a CI container startup/static-page smoke are present. Docker is
  unavailable in the local workspace, so the image health/smoke path still requires its first CI run.
- Task 8 retention/housekeeping is complete: a shared storage policy transactionally bounds
  per-account operation logs by configurable age/row limits; JSON caches delete expired
  `.json`/`.json.tmp` files on read, startup, and a configurable periodic pass; cleanup emits
  count-only diagnostics; and the maintenance runbook defines safe `VACUUM` boundaries. The latest
  subscription query, SQLite initializer/preflight, running-detail `ExecutionGateConflict`, Poll
  supersede, claim/Execution adapters,
  diagnostic-redaction, and frontend cursor/detail transport batches pass the final combined local
  repository gate: 302 Rust unit tests, 9 router contracts, 9 upstream contracts, and 247 frontend tests across
  43 files plus checked API types, format/lint/build. Production bootstrap, list/detail, manual Poll,
  Poll/Execution worker, readiness and operation logs now use the latest repository; real qB and
  filesystem effects are wired through the application service. No plan lifecycle status changes until the first CI container smoke and
  the remaining recovery/governance work are complete.

Local no-go evidence: neither `docker` nor `actionlint` is installed in this workspace. Do not mark the
container or recovery gates complete from YAML formatting alone. Completion requires a recorded CI run
whose container job builds from a clean checkout and passes health/static smoke, plus a temporary
deployment restore drill that records the image tag/digest, schema version, subscription count, restored
config mode, and last-verified date before and after restoration.

## Task 1: Establish one reproducible verification command

- [x] Add explicit frontend scripts:
  - `test` for the single Vitest suite.
  - Retire the temporary `test:legacy` runner after all retained contracts move to Vitest.
  - `check` for formatting/lint/type checks.
  - `verify:frontend` that runs tests, check, and build.
- [x] Add repository documentation for the Rust gate:
  - `cargo fmt -- --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test --locked --all-targets`
- [x] Pin supported toolchains:
  - Add `rust-version` to `Cargo.toml`.
  - Add `engines.node` and an exact `packageManager` declaration to `package.json`.
- [x] Fix existing formatting and Clippy findings in behavior-preserving commits.
- [x] Remove only genuinely unused code; do not hide warnings with new broad `allow` attributes.

Verification:

```bash
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
npm run check
npm test
npm run build
```

Commit boundary: toolchain metadata, scripts, and warning cleanup only.

## Task 2: Add pull-request quality gates

- [x] Create `.github/workflows/quality.yml` for pull requests and as a reusable gate called by the
      `main`/tag publication workflow, without a duplicate direct `main` trigger.
- [x] Use separate jobs for Rust, frontend, docs, and container build validation.
- [x] Cache Cargo registry/target and npm cache without caching generated `static/` as a source.
- [x] Rust job runs fmt, Clippy, and all tests.
- [x] Frontend job runs `npm ci`, tests, check, and production build.
- [x] Documentation job runs formatting and internal-link validation.
- [x] Upload useful logs/artifacts when a gate fails.
- [x] Make image publication depend on the reusable quality workflow or repeat the required gates.
- [x] Ensure tag publication cannot bypass tests.

Verification:

```bash
actionlint .github/workflows/quality.yml .github/workflows/docker-publish.yml
```

If `actionlint` is not available locally, validate workflow syntax through a pull request before
enabling required checks.

Commit boundary: CI workflows only.

## Task 3: Make Docker builds self-contained

- [x] Write a frontend stage based on the pinned Node version.
- [x] Copy `package.json` and `package-lock.json` before source files and run `npm ci`.
- [x] Build the frontend inside the image and export `/app/static`.
- [x] Keep the Rust dependency layer independent from frontend assets.
- [x] Build the Rust binary with `--locked`.
- [x] Copy the fresh frontend output and binary into the runtime stage.
- [x] Remove `tmdb_client`/native-tls and OpenSSL build/runtime packages; all current upstream
      clients use the shared Reqwest rustls policy.
- [x] Run as a configurable non-root UID/GID through the Compose `PUID`/`PGID` setting where NAS
      permissions permit it.
- [x] Ensure `.dockerignore` excludes local `static/`, `target/`, caches, secrets, and agent state.
- [x] Add OCI labels for source revision and version.

Verification from a clean tree or clean temporary checkout:

```bash
docker build --no-cache -t tmdb-mteam-hub:local .
```

The build must not depend on a pre-existing host `static/` directory.

Commit boundary: Dockerfile and dockerignore only.

Local dependency evidence contains no `tmdb_client`, `native-tls`, `openssl` or `openssl-sys` package.
The first clean CI container build remains the executable proof that the Debian package reduction is
complete in the image environment.

## Task 4: Add health, readiness, and container smoke tests

- [x] Add unauthenticated `GET /healthz` that proves the process event loop is alive and returns no
      configuration or account data.
- [x] Add `GET /readyz` that verifies the validated configuration is loaded and the SQLite state
      repository can be opened without mutating data.
- [x] Keep health endpoints outside the management-auth boundary but return minimal information.
- [x] Add a Docker `HEALTHCHECK` or Compose healthcheck against `/healthz`.
- [x] Add an integration test through the real router for both endpoints.
- [x] Add a CI smoke job that starts the container with temporary config/state directories, waits for
      health, requests the static index, and then stops the container.

Verification:

```bash
docker compose -f deploy/nas/docker-compose.yml config
curl --fail http://127.0.0.1:8787/healthz
curl --fail http://127.0.0.1:8787/
```

Commit boundary: health handlers/tests and container health configuration.

## Task 5: Separate config, state, cache, and media mounts

- [x] Change the container runtime subscription state path from `/data/cache/subscriptions` to
      `/data/state`.
- [x] Preserve an explicit `SUBSCRIPTION_STATE_DIR` override.
- [x] Update Compose to mount:
  - `./config:/data/config`
  - `./state:/data/state`
  - `./cache/tmdb:/data/cache/tmdb`
  - `./cache/douban:/data/cache/douban`
  - one shared host media root at `/srv/media`
- [x] Express example `download_dir` and `link_target_dir` beneath `/srv/media`.
- [x] Document that hardlinks require source and target to be on the same filesystem.
- [x] Document required read/write permissions and the effect of container UID/GID.
- [x] Add log rotation options to Compose.
- [x] Use a version tag variable instead of hard-coding `latest` as the only image reference.

Verification:

```bash
docker compose -f deploy/nas/docker-compose.yml config
```

Perform a manual or automated same-filesystem hardlink smoke test using temporary files under the
shared media mount.

Commit boundary: Compose, example config paths, and NAS deployment guide.

## Task 6: Document backup, restore, upgrade, and rollback

- [x] Define durable data as exactly `config.toml` plus `subscriptions.sqlite`; adjacent old state
      files and rebuildable caches are excluded.
- [x] Define TMDB/Douban caches as rebuildable.
- [x] Document a safe stopped-container backup procedure.
- [x] Record that no online SQLite backup procedure is implemented or supported; stopped-container
      copy is the only documented path.
- [x] Require a timestamped pre-upgrade backup before every release that may change current state.
- [x] Document daily/weekly retention recommendations.
- [x] Document restoration into a clean deployment.
- [x] Document that binary rollback may require restoring the matching pre-upgrade database.
- [x] Add a CI acceptance script that performs a stopped backup of only `config.toml` and
      `subscriptions.sqlite`, restores them into a clean deployment, and compares integrity, schema
      version, subscription count, operation-log count, config mode, health, readiness, and static
      assets while proving adjacent legacy sentinels were not copied or changed.
- [ ] Test the written procedure using a temporary deployment and record the last verified date.

Verification evidence:

- A backup contains config and state, but may omit caches.
- A new container started from the restored data returns the same subscription count and schema
  version.
- The procedure includes secret-file permission restoration.

Commit boundary: operational runbooks only.

There is no supported online backup command in the production runtime. This conditional item remains
open/not-applicable until an operator-facing online snapshot path is designed and tested; ordinary file
copy while the service writes is not a supported substitute.

## Task 7: Create living architecture and ADR documentation

- [x] Add a root `README.md` with project purpose, supported deployment model, development commands,
      verification commands, and links to operations docs.
- [x] Document the modular-monolith dependency direction in `docs/architecture/overview.md`.
- [x] Document authoritative latest state, ignored legacy files and cache rules in `data-storage.md`.
- [x] Convert the standalone-detail decision into an ADR and mark the old drawer spec superseded.
- [x] Convert subscription state convergence into an ADR and identify the single authoritative PRD.
- [x] Add lifecycle front matter to active specs and plans.
- [x] Move all superseded/contradicted specs and plans to `docs/archive/` without deleting useful
      historical rationale; archive metadata marks their commands and checklists non-executable.
- [ ] Ensure untracked convergence documents are either adopted and committed or explicitly archived.
- [x] Remove stale unchecked task status from completed work. Remaining unchecked items now represent
      real Git-adoption, visual, hosted CI, deployment or target-environment gates rather than old
      implementation slices; commit commands are labeled as delivery actions instead of task gaps.

Verification:

```bash
npm run check
rg -n '^status:' docs README.md
rg -n 'detail=movie|detail=subscription' docs --glob '!docs/archive/**'
```

No living document may still present the query-based drawer as the current architecture.

Commit boundary: README, architecture docs, ADRs, and archive moves.

Governance gate: superseded documents have been moved under `docs/archive/` with non-executable
metadata and updated backlinks. The current dirty worktree still contains adopted but uncommitted
convergence documents; they must be committed before clean-checkout evidence can be claimed.

## Task 8: Add retention and housekeeping policies

- [x] Add configurable operation-log retention by age and/or maximum row count.
- [x] Scope operation-log queries and cleanup by account where applicable.
- [x] Delete or compact expired cache files rather than leaving every expired entry on disk.
- [x] Document when SQLite `VACUUM` or incremental vacuum is appropriate.
- [x] Add metrics/log messages for cleanup results without exposing secrets.
- [x] Add tests proving cleanup never deletes active state or configuration.

Verification:

```bash
cargo test retention
cargo test cache_cleanup
cargo test operation_logs
```

Commit boundary: retention implementation and operations documentation.

## Rollout order

1. Task 1 and Task 2 establish gates.
2. Task 3 makes the image reproducible.
3. Task 4 adds smoke-testable health.
4. Task 5 fixes volumes before users rely on hardlink automation.
5. Task 6 documents and tests recovery before final runtime acceptance.
6. Task 7 and Task 8 finish governance and long-running maintenance.

## Completion gate

Current status: `in_progress`. Passing the current local code gates does not complete this plan while
container smoke, exercised recovery, and documentation-governance acceptance remain open.

Retention is now locally implemented and tested; the remaining external acceptance is specifically:

- first successful `container` job in `.github/workflows/quality.yml` on GitHub-hosted Docker;
- first hosted execution of `scripts/ci/container-acceptance.sh`, which now automates the documented
  stopped-deployment restore contract; local
  independent-root restore/preflight evidence already covers detail/log equality, a missing current
  database and corrupt restored database fail-closed behavior;
- explicit adoption of remaining untracked convergence documents before commit.

This plan is complete when:

- All verification commands pass locally and in pull-request CI.
- Image publication cannot run without the required gates.
- `docker build .` works from a clean checkout.
- A container passes health and static-page smoke tests.
- Config, state, cache, and media are mounted according to their durability and filesystem needs.
- Backup/restore and rollback have been exercised from the written runbook.
- Living docs identify the current architecture and all historical contradictions are archived or
  marked superseded.
