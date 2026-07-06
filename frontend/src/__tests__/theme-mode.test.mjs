import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(__dirname, "../App.vue"), "utf8");
const functionStart = appSource.indexOf("const THEME_MODES");
const functionEnd = appSource.indexOf("\n\nconst route = useRoute", functionStart);

assert.notEqual(functionStart, -1, "theme helpers should start at THEME_MODES");
assert.notEqual(functionEnd, -1, "theme helpers should end before route setup");

const helpers = vm.runInNewContext(
  `${appSource.slice(functionStart, functionEnd)}
({
  normalizeThemeMode,
  resolveThemeScheme,
  nextThemeMode,
  themeModeLabel,
});`,
);

assert.equal(helpers.normalizeThemeMode("system"), "system");
assert.equal(helpers.normalizeThemeMode("light"), "light");
assert.equal(helpers.normalizeThemeMode("dark"), "dark");
assert.equal(helpers.normalizeThemeMode("bad"), "system");
assert.equal(helpers.normalizeThemeMode(null), "system");

assert.equal(helpers.resolveThemeScheme("system", true), "dark");
assert.equal(helpers.resolveThemeScheme("system", false), "light");
assert.equal(helpers.resolveThemeScheme("light", true), "light");
assert.equal(helpers.resolveThemeScheme("dark", false), "dark");
assert.equal(helpers.resolveThemeScheme("bad", true), "dark");

assert.equal(helpers.nextThemeMode("system"), "light");
assert.equal(helpers.nextThemeMode("light"), "dark");
assert.equal(helpers.nextThemeMode("dark"), "system");
assert.equal(helpers.nextThemeMode("bad"), "light");

assert.equal(helpers.themeModeLabel("system"), "主题：跟随系统");
assert.equal(helpers.themeModeLabel("light"), "主题：浅色");
assert.equal(helpers.themeModeLabel("dark"), "主题：深色");
