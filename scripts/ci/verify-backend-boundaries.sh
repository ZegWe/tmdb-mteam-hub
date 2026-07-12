#!/usr/bin/env bash

set -Eeuo pipefail

fail_if_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  local output

  if output="$(rg -n "$pattern" "$@")"; then
    printf '%s\n%s\n' "$label" "$output" >&2
    return 1
  fi
}

fail_if_production_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  local file
  local matches
  local output=""

  for file in "$@"; do
    if matches="$(sed '/^#\[cfg(test)\]/,$d' "$file" | rg -n "$pattern")"; then
      output+="${file}:${matches}"$'\n'
    fi
  done

  if [[ -n "$output" ]]; then
    printf '%s\n%s' "$label" "$output" >&2
    return 1
  fi
}

fail_if_production_match \
  "domain/application core imports a forbidden framework, storage, config, or provider DTO" \
  'axum|reqwest|rusqlite|FileConfig|DoubanLibrary(Item|List)|QbTorrent' \
  src/subscription/model.rs \
  src/subscription/ports.rs \
  src/subscription/queries.rs \
  src/subscription/execution.rs

fail_if_match \
  "application service imports an HTTP handler or HTTP error" \
  'crate::http|http::error|ApiError|StatusCode|IntoResponse' \
  src/subscription/worker.rs \
  src/subscription/queries.rs \
  src/subscription/execution.rs

fail_if_match \
  "provider/storage/effect adapter imports an HTTP-layer error or response type" \
  'http::error|ApiError|StatusCode|IntoResponse' \
  src/clients \
  src/storage \
  src/subscription/effect_adapters.rs \
  src/subscription/execution_effects.rs \
  src/subscription/wanted_source.rs

fail_if_match \
  "public HTTP handler returns an unnamed Json<Value> success boundary" \
  'Json<Value>|Result<Json<Value>' \
  src/http

fail_if_match \
  "public HTTP handler directly serializes a provider or domain success DTO" \
  'Result<Json<(crate::)?(douban|subscription)::|Json<(crate::)?(douban|subscription)::' \
  src/http

fail_if_match \
  "public HTTP response uses a transparent JSON value wrapper" \
  'serde\(transparent\)' \
  src/http

fail_if_match \
  "production tree contains an obsolete subscription migration entry point" \
  'subscription-migrate-v5|migrate_offline_v4' \
  src Dockerfile Cargo.toml

node scripts/ci/verify-openapi-parity.mjs

printf 'backend boundaries verified: framework direction, named DTOs, and latest-only entry points\n'
