import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(__dirname, "../App.vue"), "utf8");
const stylesSource = readFileSync(resolve(__dirname, "../styles.css"), "utf8");
const functionStart = appSource.indexOf("function cardKey");
const functionEnd = appSource.indexOf("\n\nfunction setSearchSource", functionStart);

assert.notEqual(functionStart, -1, "search card helpers should start at cardKey");
assert.notEqual(functionEnd, -1, "search card helpers should end before source setter");

function helpersForView(view) {
  return vm.runInNewContext(
    `const currentView = { value: ${JSON.stringify(view)} };
${appSource.slice(functionStart, functionEnd)}
({
  cardSubtitle,
});`,
  );
}

assert.equal(
  helpersForView("search").cardSubtitle({
    source: "douban",
    abstract_text: "美国 / 剧情 犯罪 / 弗兰克·德拉邦特",
    abstract_2: "1994",
    rating: { value: 9.7 },
  }),
  "1994 · ★ 9.7",
  "douban search cards should only show year and rating in the subtitle",
);

assert.equal(
  helpersForView("search").cardSubtitle({
    media_type: "movie",
    release_date: "1994-09-10",
    vote_average: 9.3,
  }),
  "1994-09-10 · ★ 9.3",
  "tmdb cards should keep date and rating subtitles",
);

assert.doesNotMatch(
  appSource,
  /douban-library|loadDoubanLibrary|豆瓣列表/,
  "search page should not expose the separate Douban library list option",
);

assert.match(
  appSource,
  /@click="setSearchSource\('douban'\)"[\s\S]*>\s*豆瓣\s*<\/button>/,
  "search page should keep Douban as a search source",
);

assert.match(
  appSource,
  /class="layout search-layout"/,
  "search page should use the full-width search layout directly",
);

assert.match(
  stylesSource,
  /\.layout\.search-layout \{[\s\S]*grid-template-columns:\s*1fr;/,
  "search view should collapse the two-column desktop layout into one column",
);

assert.match(
  stylesSource,
  /\.media-card-search \{[\s\S]*width:\s*100%;[\s\S]*flex-direction:\s*column;/,
  "search cards should keep the poster above the metadata",
);

assert.doesNotMatch(
  stylesSource,
  /\.media-card-search \{[^}]*flex-direction:\s*row;/,
  "search cards should not use the horizontal detail-list layout",
);

assert.match(
  stylesSource,
  /\.media-card-search \.title,[\s\S]*white-space:\s*nowrap;[\s\S]*text-overflow:\s*ellipsis;/,
  "search titles should ellipsize instead of wrapping to two lines",
);
