import {
  SUBSCRIPTION_ATTENTION_TAGS,
  SUBSCRIPTION_EXECUTION_STATES,
  SUBSCRIPTION_LIFECYCLE_STATES,
  SUBSCRIPTION_MEDIA_KINDS,
} from "./contracts.js";
import { getAuthStatus, loginAuthSession, logoutAuthSession } from "./endpoints/auth.js";
import { getTvSeasonEpisodes } from "./endpoints/media-details.js";
import { pushMteamTorrent, testQbServer } from "./endpoints/qb.js";
import { getSettings, updateSettings } from "./endpoints/settings.js";

/** @param {import("./contracts.js").SubscriptionSummaryDto} value */
function acceptsSubscriptionSummary(value) {
  return value;
}

/** @param {import("./contracts.js").SubscriptionSummaryPageDto} value */
function acceptsSubscriptionSummaryPage(value) {
  return value;
}

/** @param {import("./contracts.js").NormalizedSubscriptionSummaryState} value */
function acceptsNormalizedSubscriptionState(value) {
  return value;
}

/** @param {import("./contracts.js").MteamSearchResponseDto} value */
function acceptsMteamSearchResponse(value) {
  return value;
}

/** @param {import("./contracts.js").TmdbMediaDetailDto} value */
function acceptsTmdbMediaDetail(value) {
  return value;
}

/** @type {import("./contracts.js").SubscriptionSummaryDto} */
const validSummary = {
  subject_id: "subject-1",
  revision: 1,
  active: true,
  inactive_at: null,
  last_seen_snapshot_id: "snapshot-1",
  media_kind: "movie",
  schedulable: true,
  blocked_reason: null,
  lifecycle_state: "queued",
  execution_state: "idle",
  next_attempt_at: null,
  retry_count: 0,
  max_retries: 3,
  retry_blocked: false,
  force_eligible_once: false,
  updated_at: 1,
  title: "Movie",
  release_year: 2026,
  poster_url: "",
  category_text: null,
  douban_sort_time: null,
  attention_tags: [],
};

acceptsSubscriptionSummary(validSummary);
acceptsSubscriptionSummaryPage({ items: [validSummary], next_cursor: null });
acceptsNormalizedSubscriptionState({
  next_cursor: null,
  ordered_ids: [validSummary.subject_id],
  records: { [validSummary.subject_id]: validSummary },
});
acceptsMteamSearchResponse({
  items: [
    {
      id: "42",
      name: "Movie",
      small_description: "UHD",
      size: 4096,
      seeders: 8,
      leechers: 2,
      created_at: "2026-07-12",
    },
  ],
  page: 1,
  page_size: 50,
});
acceptsTmdbMediaDetail({
  media_type: "tv",
  id: 84,
  title: "Series",
  original_title: "Original Series",
  overview: "Overview",
  tagline: null,
  poster_path: null,
  poster_url: null,
  backdrop_path: null,
  backdrop_url: null,
  release_date: null,
  first_air_date: "2026-07-12",
  last_air_date: null,
  runtime: null,
  status: "Returning Series",
  vote_average: 8.5,
  vote_count: 42,
  genres: [],
  production_countries: [],
  spoken_languages: [],
  origin_country: ["CN"],
  imdb_id: "tt0084",
  douban_id: null,
  douban_url: null,
  number_of_seasons: 1,
  number_of_episodes: 8,
  episode_run_time: [45],
  networks: [],
  series_type: "Scripted",
  seasons: [],
});

// @ts-expect-error M-Team candidates require a stable id
acceptsMteamSearchResponse({ items: [{ name: "missing id" }], page: 1, page_size: 50 });

// @ts-expect-error TMDB detail uses one normalized title field, not provider-specific name aliases
acceptsTmdbMediaDetail({ media_type: "tv", id: 84, name: "legacy provider title" });

// @ts-expect-error latest list responses do not expose an aggregate records object
acceptsSubscriptionSummaryPage({ records: { [validSummary.subject_id]: validSummary } });

// @ts-expect-error the backend lifecycle contract has no paused state
acceptsSubscriptionSummary({ ...validSummary, lifecycle_state: "paused" });

const { title: _removedTitle, ...missingTitle } = validSummary;
// @ts-expect-error title is required by SubscriptionSummaryDto
acceptsSubscriptionSummary(missingTitle);

/** @type {import("./contracts.js").SubscriptionMediaKind[]} */
const mediaKinds = [...SUBSCRIPTION_MEDIA_KINDS];
/** @type {import("./contracts.js").SubscriptionLifecycleState[]} */
const lifecycleStates = [...SUBSCRIPTION_LIFECYCLE_STATES];
/** @type {import("./contracts.js").SubscriptionExecutionState[]} */
const executionStates = [...SUBSCRIPTION_EXECUTION_STATES];
/** @type {import("./contracts.js").SubscriptionAttentionTag[]} */
const attentionTags = [...SUBSCRIPTION_ATTENTION_TAGS];

void [mediaKinds, lifecycleStates, executionStates, attentionTags];

const watcher = {
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

/** @type {import("./endpoints/settings.js").SettingsSnapshotDto} */
const validSettingsSnapshot = {
  revision: 7,
  listen_ip: "127.0.0.1",
  listen_port: 8787,
  has_tmdb_api_key: true,
  has_mteam_api_key: true,
  has_douban_cookie: false,
  has_admin_token: true,
  qb_servers: [
    {
      id: "nas",
      name: "NAS",
      base_url: "http://127.0.0.1:8080",
      username: "admin",
      insecure_tls: false,
      has_password: true,
    },
  ],
  subscription_categories: [
    {
      name: "电影",
      wanted_tag: "movie",
      qb_server_id: "nas",
      qb_category: "movie",
      qb_save_dir_name: "movies",
      download_dir: "/downloads/movies",
      link_target_dir: "/media/movies",
    },
  ],
  subscription_watcher: watcher,
  torrent_match_rules: [
    {
      name: "UHD",
      priority: 100,
      mode: "all",
      title_keywords: ["2160p"],
      resolution_keywords: ["2160p"],
      source_keywords: ["BluRay"],
    },
  ],
  allowed_origins: [],
  secure_cookie: false,
  restart_required: false,
};

/** @type {import("./endpoints/settings.js").SettingsUpdateDto} */
const validSettingsUpdate = {
  expected_revision: validSettingsSnapshot.revision,
  mteam_api_key: "replacement-key",
  qb_servers: [
    {
      id: "nas",
      name: "NAS",
      base_url: "http://127.0.0.1:8080",
      username: "admin",
      insecure_tls: false,
    },
  ],
  subscription_categories: validSettingsSnapshot.subscription_categories,
  subscription_watcher: watcher,
  torrent_match_rules: validSettingsSnapshot.torrent_match_rules,
};

/** @type {import("./endpoints/auth.js").AuthStatusDto} */
const validAuthStatus = {
  authenticated: true,
  token_configured: true,
  bootstrap_allowed: false,
};

/** @type {import("./endpoints/qb.js").QbTestResponseDto} */
const validQbTestResponse = { ok: true, version: "5.0.4" };

/** @type {import("./endpoints/qb.js").QbPushMteamResponseDto} */
const validQbPushResponse = { ok: true };

/** @type {Promise<import("./endpoints/auth.js").AuthStatusDto>} */
const authStatusRequest = getAuthStatus();
/** @type {Promise<import("./endpoints/auth.js").AuthStatusDto>} */
const loginRequest = loginAuthSession("management-token-123456789");
/** @type {Promise<import("./endpoints/auth.js").AuthStatusDto>} */
const logoutRequest = logoutAuthSession();
/** @type {Promise<import("./endpoints/settings.js").SettingsSnapshotDto>} */
const settingsRequest = getSettings();
/** @type {Promise<import("./endpoints/settings.js").SettingsSnapshotDto>} */
const settingsUpdateRequest = updateSettings(validSettingsUpdate);
/** @type {Promise<import("./endpoints/qb.js").QbTestResponseDto>} */
const qbTestRequest = testQbServer({ server_id: "nas" });
/** @type {Promise<import("./endpoints/qb.js").QbPushMteamResponseDto>} */
const qbPushRequest = pushMteamTorrent({ server_id: "nas", torrent_id: "42" });
/** @type {Promise<import("./contracts.js").TmdbSeasonDetailDto>} */
const seasonRequest = getTvSeasonEpisodes(84, 1);

// @ts-expect-error login tokens are strings; endpoint code no longer coerces arbitrary values
loginAuthSession(42);

// @ts-expect-error auth endpoint methods are fixed by the endpoint contract
getAuthStatus({ method: "POST" });

// @ts-expect-error settings mutations require an optimistic-concurrency revision
updateSettings({ mteam_api_key: "missing revision" });

// @ts-expect-error qB actions require a saved server ID
testQbServer({ base_url: "http://127.0.0.1:8080" });

// @ts-expect-error qB pushes require both server and torrent identity
pushMteamTorrent({ torrent_id: "42" });

// @ts-expect-error redacted settings responses never contain secret values
validSettingsSnapshot.tmdb_api_key = "must not exist";

// @ts-expect-error current auth responses do not expose legacy logged_in aliases
validAuthStatus.logged_in = true;

// @ts-expect-error qB connection responses always include the detected version
validQbTestResponse.version = undefined;

void [
  authStatusRequest,
  loginRequest,
  logoutRequest,
  settingsRequest,
  settingsUpdateRequest,
  qbTestRequest,
  qbPushRequest,
  seasonRequest,
  validAuthStatus,
  validQbTestResponse,
  validQbPushResponse,
];
