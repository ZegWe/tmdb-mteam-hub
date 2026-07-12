#!/usr/bin/env bash

set -Eeuo pipefail

image="${1:-tmdb-mteam-hub:quality}"
run_key="${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}-$$"
root="$(mktemp -d)"
source_container="tmdb-mteam-hub-source-${run_key}"
restored_container="tmdb-mteam-hub-restored-${run_key}"

log() {
  printf '[container-acceptance] %s\n' "$*"
}

cleanup_container() {
  local container="$1"
  if docker inspect "$container" >/dev/null 2>&1; then
    docker logs "$container" 2>/dev/null || true
    docker rm --force "$container" >/dev/null 2>&1 || true
  fi
}

cleanup() {
  cleanup_container "$source_container"
  cleanup_container "$restored_container"
  rm -rf "$root"
}
trap cleanup EXIT

for command in curl docker python3 sha256sum; do
  if ! command -v "$command" >/dev/null 2>&1; then
    log "required command is unavailable: $command"
    exit 1
  fi
done

sha256_file() {
  sha256sum "$1" | cut -d' ' -f1
}

start_container() {
  local container="$1"
  local deployment_root="$2"

  docker run --detach --name "$container" \
    --publish 127.0.0.1::8787 \
    --volume "$deployment_root/config:/data/config" \
    --volume "$deployment_root/state:/data/state" \
    --volume "$deployment_root/cache/tmdb:/data/cache/tmdb" \
    --volume "$deployment_root/cache/douban:/data/cache/douban" \
    "$image" >/dev/null
}

wait_for_container() {
  local container="$1"
  local host_port
  local base_url

  host_port="$(
    docker inspect \
      --format '{{(index (index .NetworkSettings.Ports "8787/tcp") 0).HostPort}}' \
      "$container"
  )"
  base_url="http://127.0.0.1:${host_port}"

  for _ in $(seq 1 60); do
    if [[ "$(docker inspect --format '{{.State.Status}}' "$container")" == "exited" ]]; then
      log "$container exited before becoming ready"
      return 1
    fi
    if [[ "$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{end}}' "$container")" == "healthy" ]] \
      && curl --fail --silent --show-error "$base_url/healthz" >/dev/null \
      && curl --fail --silent --show-error "$base_url/readyz" >/dev/null; then
      curl --fail --silent --show-error "$base_url/" >"$root/${container}.index.html"
      test -s "$root/${container}.index.html"
      printf '%s\n' "$base_url"
      return 0
    fi
    sleep 1
  done

  log "$container did not become healthy and ready within 60 seconds"
  return 1
}

database_evidence() {
  local database="$1"

  python3 - "$database" <<'PY'
import json
import sqlite3
import sys
from pathlib import Path

database = Path(sys.argv[1]).resolve()
connection = sqlite3.connect(f"file:{database}?mode=ro&immutable=1", uri=True)
try:
    schema_version = connection.execute(
        "SELECT value FROM subscription_schema_meta WHERE key = 'schema_version'"
    ).fetchone()
    if schema_version is None:
        raise SystemExit("missing subscription schema version")
    integrity = connection.execute("PRAGMA integrity_check").fetchone()
    if integrity != ("ok",):
        raise SystemExit(f"SQLite integrity check failed: {integrity!r}")
    evidence = {
        "schema_version": schema_version[0],
        "subscription_count": connection.execute(
            "SELECT count(*) FROM wanted_subscription_records"
        ).fetchone()[0],
        "operation_log_count": connection.execute(
            "SELECT count(*) FROM operation_logs"
        ).fetchone()[0],
    }
    print(json.dumps(evidence, sort_keys=True, separators=(",", ":")))
finally:
    connection.close()
PY
}

seed_acceptance_log() {
  local database="$1"

  python3 - "$database" <<'PY'
import sqlite3
import sys

connection = sqlite3.connect(sys.argv[1])
try:
    connection.execute(
        """
        INSERT INTO operation_logs (
            account_key, created_at, category, action, target_type,
            target_id, target_title, status, summary, error, related_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            "container-acceptance",
            1,
            "acceptance",
            "seed_restore_evidence",
            "deployment",
            "source",
            "Container acceptance",
            "success",
            "Non-empty stopped-backup evidence",
            None,
            '{"fixture":true}',
        ),
    )
    connection.commit()
finally:
    connection.close()
PY
}

source_root="$root/source"
backup_root="$root/backup"
restored_root="$root/restored"

mkdir -p \
  "$source_root/config" \
  "$source_root/state" \
  "$source_root/cache/tmdb" \
  "$source_root/cache/douban"
chmod 700 "$source_root/config" "$source_root/state"

cat >"$source_root/config/config.toml" <<'TOML'
listen_ip = "0.0.0.0"
listen_port = 8787

[management]
admin_token = "ci-container-health-token-123456789"
allowed_origins = []
secure_cookie = false

[subscription_watcher]
enabled = false
dry_run = true
TOML
chmod 600 "$source_root/config/config.toml"

printf 'legacy-wanted-sqlite-must-stay\n' >"$source_root/state/wanted.sqlite"
printf '{"legacy":"wanted-json-must-stay"}\n' >"$source_root/state/wanted_ci.json"
printf 'rebuildable-cache-must-not-be-backed-up\n' >"$source_root/cache/tmdb/sentinel"

source_legacy_sqlite_sha256="$(sha256_file "$source_root/state/wanted.sqlite")"
source_legacy_json_sha256="$(sha256_file "$source_root/state/wanted_ci.json")"

image_id="$(docker image inspect --format '{{.Id}}' "$image")"
image_digests="$(docker image inspect --format '{{join .RepoDigests ","}}' "$image")"
log "verified_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
log "image_reference=$image image_id=$image_id repo_digests=${image_digests:-none}"
log "starting source deployment"
start_container "$source_container" "$source_root"
source_url="$(wait_for_container "$source_container")"
log "source deployment ready at $source_url"

test -s "$source_root/state/subscriptions.sqlite"
test "$(sha256_file "$source_root/state/wanted.sqlite")" = "$source_legacy_sqlite_sha256"
test "$(sha256_file "$source_root/state/wanted_ci.json")" = "$source_legacy_json_sha256"

log "stopping source deployment before copying SQLite"
docker stop --time 15 "$source_container" >/dev/null
test ! -e "$source_root/state/subscriptions.sqlite-wal"
test ! -e "$source_root/state/subscriptions.sqlite-shm"
test ! -e "$source_root/state/subscriptions.sqlite-journal"
seed_acceptance_log "$source_root/state/subscriptions.sqlite"

source_config_sha256="$(sha256_file "$source_root/config/config.toml")"
source_config_mode="$(stat -c '%a' "$source_root/config/config.toml")"
source_database_evidence="$(database_evidence "$source_root/state/subscriptions.sqlite")"
source_database_sha256="$(sha256_file "$source_root/state/subscriptions.sqlite")"
log "source database evidence: $source_database_evidence"
log "source config mode: $source_config_mode"
test "$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["operation_log_count"])' "$source_database_evidence")" -eq 1

mkdir -p "$backup_root/config" "$backup_root/state"
chmod 700 "$backup_root/config" "$backup_root/state"
cp -p "$source_root/config/config.toml" "$backup_root/config/config.toml"
cp -p "$source_root/state/subscriptions.sqlite" "$backup_root/state/subscriptions.sqlite"

test "$(find "$backup_root" -type f | wc -l)" -eq 2
test ! -e "$backup_root/state/wanted.sqlite"
test ! -e "$backup_root/state/wanted_ci.json"
test ! -e "$backup_root/cache"
test "$(sha256_file "$backup_root/config/config.toml")" = "$source_config_sha256"
test "$(sha256_file "$backup_root/state/subscriptions.sqlite")" = "$source_database_sha256"
test "$(stat -c '%a' "$backup_root/config/config.toml")" = "600"
backup_config_mode="$(stat -c '%a' "$backup_root/config/config.toml")"
log "backup config mode: $backup_config_mode"

mkdir -p \
  "$restored_root/config" \
  "$restored_root/state" \
  "$restored_root/cache/tmdb" \
  "$restored_root/cache/douban"
chmod 700 "$restored_root/config" "$restored_root/state"
cp -p "$backup_root/config/config.toml" "$restored_root/config/config.toml"
cp -p "$backup_root/state/subscriptions.sqlite" "$restored_root/state/subscriptions.sqlite"

test "$(sha256_file "$restored_root/config/config.toml")" = "$source_config_sha256"
test "$(sha256_file "$restored_root/state/subscriptions.sqlite")" = "$source_database_sha256"
test "$(stat -c '%a' "$restored_root/config/config.toml")" = "600"
restored_config_mode="$(stat -c '%a' "$restored_root/config/config.toml")"
log "restored config mode: $restored_config_mode"
test ! -e "$restored_root/state/wanted.sqlite"
test ! -e "$restored_root/state/wanted_ci.json"

log "starting clean restored deployment"
start_container "$restored_container" "$restored_root"
restored_url="$(wait_for_container "$restored_container")"
log "restored deployment ready at $restored_url"

docker stop --time 15 "$restored_container" >/dev/null
restored_database_evidence="$(database_evidence "$restored_root/state/subscriptions.sqlite")"
log "restored database evidence: $restored_database_evidence"

test "$restored_database_evidence" = "$source_database_evidence"
test "$(sha256_file "$source_root/state/wanted.sqlite")" = "$source_legacy_sqlite_sha256"
test "$(sha256_file "$source_root/state/wanted_ci.json")" = "$source_legacy_json_sha256"

log "PASS: clean build startup, latest-only state, stopped backup, and clean restore are accepted"
