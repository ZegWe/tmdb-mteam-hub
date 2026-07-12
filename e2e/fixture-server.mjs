import { createReadStream, existsSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const STATIC_ROOT = join(ROOT, "static");
const DEFAULT_PORT = 4174;

const MIME_TYPES = Object.freeze({
  ".css": "text/css; charset=utf-8",
  ".gif": "image/gif",
  ".html": "text/html; charset=utf-8",
  ".ico": "image/x-icon",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".webp": "image/webp",
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

function redactedConfig(revision = 7, overrides = {}) {
  return {
    revision,
    has_admin_token: true,
    has_tmdb_api_key: true,
    has_mteam_api_key: true,
    has_douban_cookie: false,
    qb_servers: [],
    subscription_categories: [],
    subscription_watcher: subscriptionWatcher(),
    torrent_match_rules: [],
    restart_required: false,
    ...overrides,
  };
}

function summary(revision = 1) {
  const updated = revision > 1;
  return {
    subject_id: "fixture-subscription",
    revision,
    active: true,
    inactive_at: null,
    last_seen_snapshot_id: `fixture-snapshot-${revision}`,
    media_kind: "movie",
    schedulable: true,
    blocked_reason: null,
    lifecycle_state: updated ? "linking" : "downloading",
    execution_state: "idle",
    next_attempt_at: null,
    retry_count: 0,
    max_retries: 3,
    retry_blocked: false,
    force_eligible_once: false,
    updated_at: updated ? 200 : 100,
    title: updated ? "浏览器订阅（已更新）" : "浏览器订阅",
    release_year: 2026,
    poster_url: "",
    category_text: "电影",
    douban_sort_time: 90,
    attention_tags: [],
  };
}

function detail(revision) {
  const updated = revision > 1;
  return {
    summary: summary(revision),
    source: {
      original_title: "Browser Fixture Movie",
      synopsis: updated ? "后台轮询后返回的最新详情" : "浏览器 fixture 的初始详情",
    },
    observation: { created_at: 50, first_seen_at: 60, last_seen_at: updated ? 200 : 100 },
    issues: [],
    skip_reason: null,
    candidates: [
      {
        torrent_id: "fixture-torrent",
        title: "Browser.Fixture.2026.2160p",
        subtitle: "deterministic candidate",
        source: "mteam",
        selected: true,
        excluded_reason: null,
      },
    ],
    tv: null,
    downloads: [
      {
        id: "fixture-download",
        torrent_id: "fixture-torrent",
        torrent_title: "Browser.Fixture.2026.2160p",
        qb_server_id: "fixture-qb",
        qb_server_name: "Fixture qB",
        qb_category: "movie",
        qb_save_dir_name: "movies",
        qb_hash: "fixture-hash",
        qb_name: "Browser.Fixture.2026",
        qb_state: updated ? "uploading" : "downloading",
        state: updated ? "completed" : "downloading",
        progress: updated ? 1 : 0.25,
        total_size: 4096,
        files: [
          {
            name: "Browser.Fixture.2026.mkv",
            size: 4096,
            progress: updated ? 1 : 0.25,
          },
        ],
        pushed_at: 80,
        checked_at: updated ? 200 : 100,
        completed_at: updated ? 190 : null,
      },
    ],
    links: [
      {
        id: "fixture-link",
        download_artifact_id: "fixture-download",
        state: updated ? "completed" : "planned",
        source_path: "/downloads/Browser.Fixture.2026.mkv",
        target_dir: "/media/movies",
        checked_at: updated ? 200 : 100,
        completed_at: updated ? 200 : null,
        files: [],
      },
    ],
  };
}

export function createFixtureState() {
  return {
    config: redactedConfig(),
    listRequests: 0,
    detailRequests: 0,
    pollRequests: 0,
    mteamRequests: 0,
    settingsWrites: 0,
    lastSettingsPayload: null,
  };
}

export function resetFixtureState(state) {
  Object.assign(state, createFixtureState());
}

function json(body, { status = 200, delayMs = 0 } = {}) {
  return { status, body, delayMs };
}

export function resolveFixtureApi({ method = "GET", url, body = null, state }) {
  const parsed = new URL(url, "http://127.0.0.1");
  const path = parsed.pathname;

  if (path === "/__fixture__/health" && method === "GET") return json({ ok: true });
  if (path === "/__fixture__/reset" && method === "POST") {
    resetFixtureState(state);
    return json({ ok: true });
  }
  if (path === "/__fixture__/state" && method === "GET") {
    return json({
      listRequests: state.listRequests,
      detailRequests: state.detailRequests,
      pollRequests: state.pollRequests,
      mteamRequests: state.mteamRequests,
      settingsWrites: state.settingsWrites,
      lastSettingsPayload: state.lastSettingsPayload,
    });
  }

  if (path === "/api/auth/status" && method === "GET") {
    return json({ authenticated: true, token_configured: true, bootstrap_allowed: false });
  }
  if (path === "/api/auth/login" && method === "POST") {
    return json({ authenticated: true, token_configured: true, bootstrap_allowed: false });
  }
  if (path === "/api/auth/logout" && method === "POST") {
    return json({ authenticated: false, token_configured: true, bootstrap_allowed: false });
  }

  if (path === "/api/config" && method === "GET") return json(state.config);
  if (path === "/api/config" && method === "PUT") {
    state.settingsWrites += 1;
    state.lastSettingsPayload = body;
    state.config = redactedConfig(state.config.revision + 1, {
      has_admin_token: state.config.has_admin_token || typeof body?.admin_token === "string",
      has_tmdb_api_key: state.config.has_tmdb_api_key || typeof body?.tmdb_api_key === "string",
      has_mteam_api_key: state.config.has_mteam_api_key || typeof body?.mteam_api_key === "string",
      has_douban_cookie: state.config.has_douban_cookie || typeof body?.douban_cookie === "string",
      qb_servers: Array.isArray(body?.qb_servers) ? body.qb_servers : [],
      subscription_categories: Array.isArray(body?.subscription_categories)
        ? body.subscription_categories
        : [],
      subscription_watcher: body?.subscription_watcher || subscriptionWatcher(),
      torrent_match_rules: Array.isArray(body?.torrent_match_rules) ? body.torrent_match_rules : [],
    });
    return json(state.config);
  }

  if (path === "/api/search" && method === "GET") {
    return json({
      movies: [
        {
          id: 42,
          title: "浏览器验收电影",
          original_title: "Browser Acceptance Movie",
          media_type: "movie",
          release_date: "2026-07-12",
        },
      ],
      tv: [],
    });
  }
  if (path === "/api/tmdb/movie/42" && method === "GET") {
    return json({
      id: 42,
      title: "浏览器验收电影详情",
      original_title: "Browser Acceptance Movie",
      overview: "主详情不等待可选 M-Team 请求即可渲染。",
      release_date: "2026-07-12",
      runtime: 123,
      imdb_id: "tt0042",
    });
  }
  if (path === "/api/douban/tags" && method === "GET") return json({ tags: [] });
  if (path === "/api/mteam/torrents" && method === "GET") {
    state.mteamRequests += 1;
    return json(
      {
        items: [
          {
            id: "fixture-torrent",
            name: "Browser.Acceptance.Movie.2160p",
            size: 4096,
            seeders: 10,
            leechers: 1,
          },
        ],
        page: 1,
        page_size: 50,
      },
      { delayMs: 1200 },
    );
  }

  if (path === "/api/operation-logs" && method === "GET") {
    return json({
      items: [
        {
          id: 1,
          account_key: "fixture-account",
          created_at: 1_783_844_800,
          category: "torrent_search",
          action: "search_torrents",
          target_type: "mteam",
          target_id: "fixture-subscription",
          target_title: "浏览器验收电影",
          status: "success",
          summary: "M-Team 种子搜索完成：1 条候选",
          related: {
            source: "imdb",
            candidate_count: 1,
            page: 1,
            page_size: 50,
          },
        },
      ],
      page: 1,
      page_size: 30,
      total: 1,
      has_more: false,
    });
  }

  if (path === "/api/subscriptions/wanted" && method === "GET") {
    state.listRequests += 1;
    const revision = state.listRequests > 1 ? 2 : 1;
    return json({ items: [summary(revision)], next_cursor: null });
  }
  if (path === "/api/subscriptions/wanted/fixture-subscription" && method === "GET") {
    state.detailRequests += 1;
    return json(detail(state.listRequests > 1 ? 2 : 1));
  }
  if (path === "/api/subscriptions/wanted/poll" && method === "POST") {
    state.pollRequests += 1;
    return json({
      inserted: 0,
      updated: 1,
      unchanged: 0,
      reactivated: 0,
      deactivated: 0,
      fetched_items: 1,
      snapshot_complete: true,
    });
  }

  return json(
    { code: "fixture_route_not_found", message: `No fixture route for ${method} ${path}` },
    { status: 404 },
  );
}

async function readJsonBody(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  if (!chunks.length) return null;
  const text = Buffer.concat(chunks).toString("utf8");
  return text ? JSON.parse(text) : null;
}

function writeJson(response, result) {
  const payload = JSON.stringify(result.body);
  response.writeHead(result.status, {
    "Cache-Control": "no-store",
    "Content-Type": "application/json; charset=utf-8",
    "Content-Length": Buffer.byteLength(payload),
  });
  response.end(payload);
}

function safeStaticPath(pathname) {
  const relative = normalize(decodeURIComponent(pathname)).replace(/^[/\\]+/, "");
  const candidate = resolve(STATIC_ROOT, relative || "index.html");
  return candidate === STATIC_ROOT || candidate.startsWith(`${STATIC_ROOT}${sep}`)
    ? candidate
    : null;
}

function serveStatic(pathname, response) {
  let file = safeStaticPath(pathname);
  if (!file) {
    response.writeHead(400).end();
    return;
  }
  if (!existsSync(file) || !statSync(file).isFile()) file = join(STATIC_ROOT, "index.html");
  if (!existsSync(file)) {
    writeJson(
      response,
      json(
        {
          code: "frontend_not_built",
          message: "Run `npm run build` before starting the E2E fixture server",
        },
        { status: 503 },
      ),
    );
    return;
  }
  const stat = statSync(file);
  response.writeHead(200, {
    "Cache-Control": "no-store",
    "Content-Type": MIME_TYPES[extname(file)] || "application/octet-stream",
    "Content-Length": stat.size,
  });
  createReadStream(file).pipe(response);
}

export function createFixtureServer({ state = createFixtureState() } = {}) {
  return createServer(async (request, response) => {
    const method = String(request.method || "GET").toUpperCase();
    const url = String(request.url || "/");
    const pathname = new URL(url, "http://127.0.0.1").pathname;
    if (pathname.startsWith("/api/") || pathname.startsWith("/__fixture__/")) {
      try {
        const body = method === "GET" || method === "HEAD" ? null : await readJsonBody(request);
        const result = resolveFixtureApi({ method, url, body, state });
        if (result.delayMs) setTimeout(() => writeJson(response, result), result.delayMs);
        else writeJson(response, result);
      } catch (error) {
        writeJson(
          response,
          json(
            {
              code: "fixture_request_invalid",
              message: error instanceof Error ? error.message : String(error),
            },
            { status: 400 },
          ),
        );
      }
      return;
    }
    serveStatic(pathname, response);
  });
}

const isEntrypoint = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isEntrypoint) {
  const port = Number(process.env.E2E_PORT || DEFAULT_PORT);
  const server = createFixtureServer();
  server.listen(port, "127.0.0.1", () => {
    process.stdout.write(`E2E fixture listening on http://127.0.0.1:${port}\n`);
  });
  const close = () => server.close(() => process.exit(0));
  process.once("SIGINT", close);
  process.once("SIGTERM", close);
}
