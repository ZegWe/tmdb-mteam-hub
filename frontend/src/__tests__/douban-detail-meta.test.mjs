import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(__dirname, "../App.vue"), "utf8");
const functionStart = appSource.indexOf("function doubanMetaRows");
const functionEnd = appSource.indexOf("\n\nfunction imdbFromDetail", functionStart);

assert.notEqual(functionStart, -1, "douban meta helper should exist");
assert.notEqual(functionEnd, -1, "douban meta helper should end before external id helpers");

const helpers = vm.runInNewContext(
  `${appSource.slice(functionStart, functionEnd)}
({
  doubanMetaRows,
});`,
);

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

assert.deepEqual(
  plain(
    helpers.doubanMetaRows({
      original_title: "The Shawshank Redemption",
      aka: ["月黑高飞(港)", "刺激1995(台)"],
      languages: ["英语"],
      countries: ["美国"],
    }),
  ),
  [
    { label: "原名", value: "The Shawshank Redemption" },
    { label: "又名", value: "月黑高飞(港) · 刺激1995(台)" },
    { label: "国家/地区", value: "美国" },
    { label: "语言", value: "英语" },
  ],
  "douban rexxar title and language fields should render as detail metadata",
);
