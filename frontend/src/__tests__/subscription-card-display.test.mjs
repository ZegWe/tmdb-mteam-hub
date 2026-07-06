import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(__dirname, "../App.vue"), "utf8");
const stylesSource = readFileSync(resolve(__dirname, "../styles.css"), "utf8");
const constantsStart = appSource.indexOf("const SUB_STATUS_LABELS");
const constantsEnd = appSource.indexOf("\nconst OPERATION_LOG_CATEGORIES", constantsStart);
const functionStart = appSource.indexOf("function subscriptionPollToast");
const functionEnd = appSource.indexOf("\nfunction openSubscriptionDetail", functionStart);

assert.notEqual(
  constantsStart,
  -1,
  "subscription display constants should start at SUB_STATUS_LABELS",
);
assert.notEqual(constantsEnd, -1, "subscription display constants should end before log constants");
assert.notEqual(
  functionStart,
  -1,
  "subscription display helpers should start at subscriptionPollToast",
);
assert.notEqual(functionEnd, -1, "subscription display helpers should end before route helpers");

const helpers = vm.runInNewContext(
  `${appSource.slice(constantsStart, constantsEnd)}
${appSource.slice(functionStart, functionEnd)}
({
  subscriptionCardMeta,
  subscriptionCardNotices,
});`,
);

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

function cssBlock(selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return stylesSource.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`))?.[1] ?? "";
}

function sourceBetween(start, end, description) {
  const startIndex = appSource.indexOf(start);
  assert.notEqual(startIndex, -1, `${description} should have a start marker`);
  const endIndex = appSource.indexOf(end, startIndex);
  assert.notEqual(endIndex, -1, `${description} should have an end marker`);
  return appSource.slice(startIndex, endIndex);
}

const failedRecord = {
  status: "failed",
  processing_stage: "error",
  stage_message: "M-Team API Key 未配置",
  next_action: "检查错误后重新轮询或手动重试",
  last_error: "M-Team API Key 未配置",
};

assert.deepEqual(
  plain(helpers.subscriptionCardNotices(failedRecord)),
  [
    {
      key: "error",
      kind: "error",
      text: "M-Team API Key 未配置；下一步：检查错误后重新轮询或手动重试",
    },
  ],
  "failed processing stages should render one state-driven error notice",
);

assert.deepEqual(
  plain(
    helpers.subscriptionCardNotices({
      status: "skipped",
      processing_stage: "skipped",
      stage_message: "initial_bootstrap_existing_wish",
      skip_reason: "initial_bootstrap_existing_wish",
    }),
  ),
  [{ key: "stage", kind: "stage", text: "历史想看，首次同步跳过" }],
  "skipped subscriptions should render the skip reason as a status notice, not an error",
);

assert.deepEqual(
  plain(
    helpers.subscriptionCardNotices({
      status: "failed",
      processing_stage: "pushing",
      stage_message: "正在推送到 qB",
      next_action: "等待 qB 接收任务",
      last_push: { status: "failed", error: "qB 添加失败" },
    }),
  ),
  [
    { key: "stage", kind: "stage", text: "正在推送到 qB；下一步：等待 qB 接收任务" },
    { key: "push-error", kind: "error", text: "qB 添加失败" },
  ],
  "distinct push failure state should render a separate error notice",
);

assert.deepEqual(
  plain(
    helpers.subscriptionCardMeta({
      douban_date: "2026-07-04",
      release_year: "2026",
      category_text: "电影",
      processing_stage: "skipped",
    }),
  ),
  ["豆瓣 2026-07-04", "2026", "电影"],
  "card meta should not duplicate the current stage/status",
);

assert.match(
  stylesSource,
  /\.subscription-list\s*\{[\s\S]*grid-auto-rows:\s*[^;]+;/,
  "subscription cards should render in stable grid rows",
);
{
  const rowHeight = Number(
    cssBlock(".subscription-list").match(/grid-auto-rows:\s*([\d.]+)px/)?.[1],
  );
  assert.ok(
    rowHeight >= 300,
    "subscription card rows should be tall enough for two-line titles, notices, and actions",
  );
}
assert.match(
  stylesSource,
  /\.subscription-card\s*\{[\s\S]*height:\s*100%;[\s\S]*overflow:\s*hidden;/,
  "subscription cards should keep a stable size and contain long text",
);
assert.match(
  stylesSource,
  /\.subscription-card\s*\{[\s\S]*display:\s*grid;[\s\S]*grid-template-rows:/,
  "subscription cards should use fixed row slots so text is not clipped by auto margins",
);
assert.match(
  cssBlock(".subscription-card-meta"),
  /flex-wrap:\s*wrap/,
  "subscription card metadata should wrap onto two lines instead of clipping long text",
);
assert.doesNotMatch(
  cssBlock(".subscription-card-meta"),
  /flex-wrap:\s*nowrap/,
  "subscription card metadata should not force all fields onto one clipped line",
);
{
  const metaHeight = Number(
    cssBlock(".subscription-card-meta").match(/max-height:\s*([\d.]+)rem/)?.[1],
  );
  assert.ok(
    metaHeight >= 2.6,
    "subscription card metadata should reserve enough height for two rows",
  );
}
assert.match(
  appSource,
  /class="subscription-card-notices"/,
  "subscription card notices should render inside a reserved notice slot",
);
assert.match(
  stylesSource,
  /\.subscription-card-notices\s*\{[\s\S]*min-height:\s*[^;]+;[\s\S]*overflow:\s*hidden;/,
  "subscription card notices should reserve enough full-line space before clipping",
);
assert.doesNotMatch(
  cssBlock(".subscription-card-actions"),
  /margin-top:\s*auto/,
  "subscription card actions should not push notice text into partial clipping",
);
assert.match(
  stylesSource,
  /\.subscription-card-notice\s*\{[\s\S]*-webkit-line-clamp:\s*2;/,
  "subscription card notices should be clamped instead of resizing cards",
);
{
  const cardSource = sourceBetween(
    'v-for="record in subscriptionRecords"',
    '<div class="subscription-card-actions"',
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
