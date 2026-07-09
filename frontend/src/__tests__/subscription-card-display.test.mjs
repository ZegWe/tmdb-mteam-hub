import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(__dirname, "../App.vue"), "utf8");
const subscriptionDetailSource = readFileSync(
  resolve(__dirname, "../components/SubscriptionDetailView.vue"),
  "utf8",
);
const stylesSource = readFileSync(resolve(__dirname, "../styles.css"), "utf8");
const constantsStart = appSource.indexOf("const SUB_LIFECYCLE_LABELS");
const constantsEnd = appSource.indexOf("\nconst OPERATION_LOG_CATEGORIES", constantsStart);
const functionStart = appSource.indexOf("function subscriptionPollToast");
const functionEnd = appSource.indexOf("\nfunction openSubscriptionDetail", functionStart);
const detailRowsStart = appSource.indexOf("function subscriptionDetailRows");
const detailRowsEnd = appSource.indexOf("\n\nfunction pushRows", detailRowsStart);

assert.notEqual(
  constantsStart,
  -1,
  "subscription display constants should start at SUB_LIFECYCLE_LABELS",
);
assert.notEqual(constantsEnd, -1, "subscription display constants should end before log constants");
assert.notEqual(
  functionStart,
  -1,
  "subscription display helpers should start at subscriptionPollToast",
);
assert.notEqual(functionEnd, -1, "subscription display helpers should end before route helpers");
assert.notEqual(detailRowsStart, -1, "subscription detail rows helper should exist");
assert.notEqual(detailRowsEnd, -1, "subscription detail rows helper should end before push rows");

const helpers = vm.runInNewContext(
  `${appSource.slice(constantsStart, constantsEnd)}
${appSource.slice(functionStart, functionEnd)}
({
  subscriptionDisplayStatus,
  subscriptionCardMeta,
  subscriptionCardNotices,
  subscriptionCardSubtitle,
  subscriptionLifecycleKey,
  subscriptionAttentionKey,
  subscriptionLifecycleNodes,
  canRetrySubscription,
  canRerunSubscription,
});`,
);

const detailHelpers = vm.runInNewContext(
  `${appSource.slice(constantsStart, constantsEnd)}
function row(label, value, href = "") {
  if (value == null || String(value).trim() === "") return null;
  const text = String(value);
  const link = String(href || "").trim();
  return link ? { label, value: text, href: link } : { label, value: text };
}
function formatUnixSeconds(value) {
  return value ? "formatted-time" : "";
}
function subscriptionDisplayStatus() {
  return { text: "待处理" };
}
function formatSubscriptionSkipReason(value) {
  return value || "";
}
${appSource.slice(functionStart, functionEnd)}
${appSource.slice(detailRowsStart, detailRowsEnd)}
({
  subscriptionDetailRows,
});`,
);

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

function cssBlock(selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return stylesSource.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`))?.[1] ?? "";
}

function sourceBetween(start, end, description, source = appSource) {
  const startIndex = source.indexOf(start);
  assert.notEqual(startIndex, -1, `${description} should have a start marker`);
  const endIndex = source.indexOf(end, startIndex);
  assert.notEqual(endIndex, -1, `${description} should have an end marker`);
  return source.slice(startIndex, endIndex);
}

for (const [name, body] of [
  [
    "subscriptionDisplayStatus",
    sourceBetween(
      "function subscriptionDisplayStatus",
      "\nfunction subscriptionProgress",
      "display status helper",
    ),
  ],
  [
    "subscriptionLifecycleKey",
    sourceBetween(
      "function subscriptionLifecycleKey",
      "\nfunction subscriptionAttentionKey",
      "lifecycle key helper",
    ),
  ],
  [
    "subscriptionAttentionKey",
    sourceBetween(
      "function subscriptionAttentionKey",
      "\nfunction subscriptionStageTrackLabel",
      "attention helper",
    ),
  ],
  [
    "canRetrySubscription",
    sourceBetween(
      "function canRetrySubscription",
      "\nfunction canRerunSubscription",
      "retry helper",
    ),
  ],
]) {
  assert.doesNotMatch(
    body,
    /record\?*\.status|record\?*\.processing_stage/,
    `${name} must not read legacy subscription state`,
  );
}

assert.equal(appSource.includes("SUB_LEGACY_LIFECYCLE_BY_STATUS"), false);

const failedRecord = {
  lifecycle_state: "searching",
  attention_tags: ["failed"],
  failure: { message: "M-Team API Key 未配置" },
  last_error: "M-Team API Key 未配置",
};

assert.deepEqual(
  plain(helpers.subscriptionCardNotices(failedRecord)),
  [
    {
      key: "failure",
      kind: "error",
      text: "M-Team API Key 未配置",
    },
  ],
  "failed semantic state should render one state-driven error notice",
);

assert.deepEqual(
  plain(
    helpers.subscriptionCardNotices({
      lifecycle_state: "queued",
      attention_tags: ["skipped"],
      skip_reason: "initial_bootstrap_existing_wish",
    }),
  ),
  [{ key: "skipped", kind: "stage", text: "历史想看，首次同步跳过" }],
  "skipped subscriptions should render the skip reason as a status notice, not an error",
);

assert.deepEqual(
  plain(
    helpers.subscriptionCardNotices({
      lifecycle_state: "searching",
      attention_tags: ["failed"],
      failure: { message: "qB 添加失败" },
    }),
  ),
  [{ key: "failure", kind: "error", text: "qB 添加失败" }],
  "semantic push failure should render from failure state, not artifact status",
);

assert.deepEqual(
  plain(
    helpers.subscriptionCardMeta({
      douban_date: "2026-07-04",
      release_year: "2026",
      category_text: "电影",
      lifecycle_state: "queued",
    }),
  ),
  ["豆瓣 2026-07-04", "2026", "电影"],
  "card meta should not duplicate the current stage/status",
);

assert.equal(
  helpers.subscriptionCardSubtitle({
    date_published: "2026-07-01",
    douban_date: "2026-06-01",
    release_year: "2026",
    category_text: "电影",
  }),
  "2026-07-01",
  "subscription card subtitle should prefer release date over Douban wanted date",
);

assert.equal(
  helpers.subscriptionCardSubtitle({
    release_year: "2026",
    douban_date: "2026-06-01",
  }),
  "2026",
  "subscription card subtitle should fall back to release year, not Douban wanted date",
);

assert.deepEqual(
  plain(
    detailHelpers
      .subscriptionDetailRows({
        subject_id: "1292052",
        category_text: "电影",
        date_published: "1994-09-10",
        release_year: 1994,
        rating_value: 9.7,
        rating_count: 123456,
        original_title: "The Shawshank Redemption",
        aka: ["刺激1995"],
        genres: ["剧情", "犯罪"],
        countries: ["美国"],
        languages: ["英语"],
        directors: ["弗兰克·德拉邦特"],
        actors: ["蒂姆·罗宾斯", "摩根·弗里曼"],
        duration: "142分钟",
        summary: "希望让人自由。",
        retry_count: 0,
        max_retries: 3,
      })
      .slice(0, 11),
  ),
  [
    { label: "豆瓣 ID", value: "1292052" },
    { label: "分类文本", value: "电影" },
    { label: "上映日期", value: "1994-09-10" },
    { label: "评分", value: "9.7（123,456 人）" },
    { label: "原名", value: "The Shawshank Redemption" },
    { label: "又名", value: "刺激1995" },
    { label: "类型", value: "剧情 · 犯罪" },
    { label: "国家/地区", value: "美国" },
    { label: "语言", value: "英语" },
    { label: "导演", value: "弗兰克·德拉邦特" },
    { label: "主演", value: "蒂姆·罗宾斯 · 摩根·弗里曼" },
  ],
  "subscription detail should show cached Douban rexxar media rows first",
);

{
  const rows = plain(
    detailHelpers.subscriptionDetailRows({
      subject_id: "waiting-release",
      lifecycle_state: "searching",
      attention_tags: ["waiting_release"],
      failure: null,
      last_error: "未搜索到候选种子",
    }),
  );
  assert.equal(
    rows.some((row) => row.label === "失败"),
    false,
    "waiting-release details should not label explanatory last_error as failure",
  );
  assert.deepEqual(
    rows.find((row) => row.label === "说明"),
    {
      label: "说明",
      value: "未搜索到候选种子",
    },
  );
}

assert.match(
  stylesSource,
  /\.subscription-list\s*\{[\s\S]*grid-template-columns:\s*repeat\(auto-fill,\s*minmax\(140px,\s*1fr\)\);/,
  "subscription cards should use the same compact poster grid as search cards",
);
{
  const cardSource = sourceBetween(
    'v-for="record in subscriptionRecords"',
    "</article>",
    "subscription card template",
  );
  assert.match(
    cardSource,
    /<img\s+:src="itemImageUrl\(record\) \|\| transparentPixel"[\s\S]*loading="lazy"/,
    "subscription cards should render the saved Douban cover image",
  );
  assert.match(
    cardSource,
    /<div class="title">\{\{ record\.title \|\| record\.subject_id \}\}<\/div>/,
    "subscription cards should show the subscription name as the primary label",
  );
  assert.match(
    cardSource,
    /class="subscription-status badge"/,
    "subscription cards should keep the status badge",
  );
  assert.doesNotMatch(
    cardSource,
    /retrySubscriptionCurrent|rerunSubscription|subscription-card-actions|subscription-stage-track|subscription-card-notices/,
    "subscription cover cards should not include workflow actions, stage tracks, or notices",
  );
}
assert.match(
  stylesSource,
  /\.subscription-card\s*\{[\s\S]*display:\s*flex;[\s\S]*flex-direction:\s*column;/,
  "subscription cards should use the same vertical poster-card structure as search cards",
);
assert.match(
  stylesSource,
  /\.subscription-card \.subscription-status\s*\{[\s\S]*position:\s*absolute;/,
  "subscription status should overlay the poster instead of consuming title space",
);
{
  const cardSource = sourceBetween(
    'v-for="record in subscriptionRecords"',
    "</article>",
    "subscription card template",
  );
  assert.doesNotMatch(
    cardSource,
    /subscriptionProgress\(record\)|subscription-card-progress|下载进度/,
    "subscription cards should not show download progress; progress belongs in detail download section",
  );
}
{
  const detailDownloadSource = sourceBetween(
    "<h4>下载</h4>",
    '<section v-if="subscriptionEpisodes.length"',
    "subscription detail download section",
    subscriptionDetailSource,
  );
  assert.match(
    detailDownloadSource,
    /subscription-detail-download-progress[\s\S]*subscriptionProgress\(selectedSubscription\)/,
    "subscription detail download section should show the overall download progress",
  );
}
assert.doesNotMatch(
  cssBlock(".subscription-progress"),
  /margin-top:/,
  "subscription progress bars should not add layout outside the reserved slot",
);
assert.doesNotMatch(
  appSource,
  /subscriptionNoteAlreadyShown|normalizeSubscriptionCardText/,
  "subscription card display should be state-driven instead of filtering duplicate text",
);

assert.deepEqual(
  plain(
    helpers.subscriptionLifecycleNodes({
      lifecycle_state: "downloading",
      attention_tags: ["waiting_release"],
    }),
  ).map(({ key, label, state, attention }) => ({ key, label, state, attention })),
  [
    { key: "queued", label: "入队", state: "done", attention: "" },
    { key: "meta", label: "元数据", state: "done", attention: "" },
    { key: "searching", label: "搜索", state: "done", attention: "" },
    { key: "downloading", label: "下载", state: "current", attention: "waiting_release" },
    { key: "linking", label: "硬链接中", state: "todo", attention: "" },
    { key: "completed", label: "完成", state: "todo", attention: "" },
  ],
  "subscription lifecycle helper should expose a fixed state graph",
);

assert.deepEqual(
  plain(
    helpers
      .subscriptionLifecycleNodes({
        lifecycle_state: "linking",
        attention_tags: ["failed"],
        failure: { message: "硬链接失败" },
      })
      .find((node) => node.key === "linking"),
  ),
  { key: "linking", label: "硬链接中", state: "current", attention: "failed" },
);

assert.deepEqual(
  plain(
    helpers.subscriptionDisplayStatus({
      lifecycle_state: "linking",
      attention_tags: ["failed"],
      failure: { message: "硬链接失败" },
    }),
  ),
  { key: "failed", text: "失败" },
  "status badge should prefer attention over lifecycle",
);

assert.equal(
  helpers
    .subscriptionLifecycleNodes({
      lifecycle_state: "searching",
      attention_tags: ["skipped"],
    })
    .find((node) => node.state === "current").attention,
  "skipped",
  "subscription lifecycle nodes should trust explicit attention tags",
);

assert.match(
  subscriptionDetailSource,
  /class="subscription-state-graph"/,
  "subscription detail should render the lifecycle as a node graph",
);
assert.match(
  subscriptionDetailSource,
  /subscriptionLifecycleNodes\(selectedSubscription\)/,
  "subscription detail should use lifecycle nodes instead of status text only",
);
assert.doesNotMatch(
  appSource.slice(detailRowsStart, detailRowsEnd),
  /row\("状态"/,
  "subscription detail rows should not duplicate the primary lifecycle graph as text",
);
