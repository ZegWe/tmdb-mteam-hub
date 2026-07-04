import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(__dirname, "../App.vue"), "utf8");
const functionStart = appSource.indexOf("function subscriptionOrderTimestamp");
const functionEnd = appSource.indexOf("\n\nconst subscriptionSummary", functionStart);

assert.notEqual(functionStart, -1, "subscription order timestamp helper should exist");
assert.notEqual(functionEnd, -1, "subscription summary marker should exist after sort helpers");

const compareSubscriptionRecords = vm.runInNewContext(
  `${appSource.slice(functionStart, functionEnd)}\ncompareSubscriptionRecords;`,
);

const records = [
  {
    title: "豆瓣第三",
    subject_id: "3",
    douban_return_order: 2,
    douban_sort_time: 3000,
    created_at: 3000,
  },
  {
    title: "豆瓣第一",
    subject_id: "1",
    douban_return_order: 0,
    douban_sort_time: 1000,
    created_at: 1000,
  },
  {
    title: "豆瓣第二",
    subject_id: "2",
    douban_return_order: 1,
    douban_sort_time: 2000,
    created_at: 2000,
  },
  {
    title: "本地旧记录",
    subject_id: "4",
    douban_sort_time: 9999,
    created_at: 9999,
  },
];

assert.deepEqual(
  [...records].sort(compareSubscriptionRecords).map((record) => record.title),
  ["豆瓣第一", "豆瓣第二", "豆瓣第三", "本地旧记录"],
);
