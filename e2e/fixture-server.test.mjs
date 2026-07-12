import assert from "node:assert/strict";
import test from "node:test";
import { createFixtureState, resetFixtureState, resolveFixtureApi } from "./fixture-server.mjs";

test("fixture advances subscription revision without exposing action endpoints", () => {
  const state = createFixtureState();
  const first = resolveFixtureApi({
    method: "GET",
    url: "/api/subscriptions/wanted?limit=100",
    state,
  });
  const second = resolveFixtureApi({
    method: "GET",
    url: "/api/subscriptions/wanted?limit=100",
    state,
  });
  const unported = resolveFixtureApi({
    method: "POST",
    url: "/api/subscriptions/wanted/fixture-subscription/retry",
    state,
  });

  assert.equal(first.body.items[0].revision, 1);
  assert.equal(second.body.items[0].revision, 2);
  assert.equal(unported.status, 404);
});

test("settings writes return a redacted latest snapshot and reset clears observations", () => {
  const state = createFixtureState();
  const replacement = "fixture-mteam-secret";
  const saved = resolveFixtureApi({
    method: "PUT",
    url: "/api/config",
    body: {
      expected_revision: 7,
      mteam_api_key: replacement,
      qb_servers: [],
      subscription_categories: [],
      subscription_watcher: subscriptionWatcher(),
      torrent_match_rules: [],
    },
    state,
  });

  assert.equal(saved.body.revision, 8);
  assert.equal(saved.body.has_mteam_api_key, true);
  assert.equal(JSON.stringify(saved.body).includes(replacement), false);
  assert.equal(state.lastSettingsPayload.mteam_api_key, replacement);

  resetFixtureState(state);
  assert.equal(state.settingsWrites, 0);
  assert.equal(state.lastSettingsPayload, null);
});

test("fixture exposes one deterministic operation log page for visual acceptance", () => {
  const response = resolveFixtureApi({
    method: "GET",
    url: "/api/operation-logs?page=1&page_size=30",
    state: createFixtureState(),
  });

  assert.equal(response.status, 200);
  assert.equal(response.body.items.length, 1);
  assert.equal(response.body.items[0].summary, "M-Team 种子搜索完成：1 条候选");
  assert.equal(response.body.has_more, false);
});

function subscriptionWatcher() {
  return {
    enabled: false,
    dry_run: true,
    poll_interval_secs: 3600,
    library_limit: 200,
    max_retries: 3,
    search_interval_secs: 1800,
    progress_interval_secs: 5,
    link_retry_interval_secs: 900,
    system_retry_interval_secs: 600,
    bootstrap_existing_as_skipped: true,
  };
}
