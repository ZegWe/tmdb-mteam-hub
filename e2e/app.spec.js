import { expect, test } from "@playwright/test";

async function fixtureState(page) {
  return page.evaluate(async () => {
    const response = await fetch("/__fixture__/state", { cache: "no-store" });
    if (!response.ok) throw new Error(`fixture state failed: ${response.status}`);
    return response.json();
  });
}

test.beforeEach(async ({ page }) => {
  await page.goto("/__fixture__/health");
  await page.evaluate(async () => {
    const response = await fetch("/__fixture__/reset", { method: "POST" });
    if (!response.ok) throw new Error(`fixture reset failed: ${response.status}`);
  });
});

test("search detail renders before optional provider and Back preserves results", async ({
  page,
}) => {
  await page.goto("/#/");

  const search = page.getByRole("searchbox");
  await search.fill("browser fixture");
  await search.press("Enter");
  const result = page.getByRole("button", { name: "打开详情 浏览器验收电影" });
  await expect(result).toBeVisible();

  await result.click();
  await expect(page).toHaveURL(/#\/detail\/movie\/42/);
  await expect(page.getByRole("heading", { level: 1, name: "浏览器验收电影详情" })).toBeVisible();
  await expect(page.getByText("主详情不等待可选 M-Team 请求即可渲染。")).toBeVisible();
  expect((await fixtureState(page)).mteamRequests).toBe(0);
  await page.getByRole("button", { name: "搜索种子", exact: true }).click();
  await expect(page.getByRole("status").filter({ hasText: "正在加载 M-Team" })).toBeVisible();
  await expect(page.getByText("Browser.Acceptance.Movie.2160p")).toBeVisible();

  await page.goBack();
  await expect(page).toHaveURL(/#\/(?:\?.*)?$/);
  await expect(search).toHaveValue("browser fixture");
  await expect(result).toBeVisible();
});

test("subscription detail observes one polling stream and exposes no legacy side effects", async ({
  page,
}) => {
  await page.goto("/#/subscriptions");
  const card = page.getByRole("link", { name: "打开订阅 浏览器订阅" });
  await expect(card).toBeVisible();
  await card.click();

  await expect(page).toHaveURL(/#\/subscriptions\/fixture-subscription/);
  await expect(page.locator("[aria-label='下载进度 25%']")).toBeVisible();
  const downloadTask = page.locator(".subscription-task-panel").first();
  await downloadTask.locator("summary").click();
  await expect(downloadTask.locator("a").first()).toBeVisible();
  await expect(downloadTask.locator("a").first()).toHaveAttribute(
    "href",
    "https://kp.m-team.cc/detail/fixture-torrent",
  );
  const candidates = page.locator(".subscription-saved-candidates");
  await expect(candidates).not.toHaveAttribute("open", "");
  await candidates.locator("summary").click();
  await expect(candidates.locator("a")).toHaveAttribute(
    "href",
    "https://kp.m-team.cc/detail/fixture-torrent",
  );
  await expect(
    candidates.getByText("大小 1.00 GB · 做种 8 · 下载中 2 · 发布 2026-10-13 10:00:00"),
  ).toBeVisible();
  expect((await fixtureState(page)).mteamRequests).toBe(0);
  await expect(page.getByRole("button", { name: /重试|重新执行|立即执行/ })).toHaveCount(0);

  await expect(page.getByRole("heading", { level: 1, name: "浏览器订阅（已更新）" })).toBeVisible({
    timeout: 8_000,
  });
  await expect(page.locator("[aria-label='下载进度 100%']")).toBeVisible();

  await page.goBack();
  await expect(page).toHaveURL(/#\/subscriptions$/);
  const before = await fixtureState(page);
  await page.waitForTimeout(5_500);
  const after = await fixtureState(page);
  expect(after.listRequests - before.listRequests).toBe(1);
  expect(after.pollRequests).toBe(0);
});

test("authenticated settings save returns redacted state", async ({ page }) => {
  await page.goto("/#/settings");
  await page
    .getByRole("navigation", { name: "设置子页面" })
    .getByRole("button", { name: "API 密钥" })
    .click();
  const mteamKey = page.getByRole("textbox", { name: "M-Team OpenAPI Key", exact: true });
  await expect(mteamKey).toHaveValue("");
  await mteamKey.fill("browser-fixture-replacement-secret");
  await page.getByRole("button", { name: "保存全部设置" }).click();

  await expect(page.locator("#settings-save-status")).toHaveText("设置已保存");
  await expect(mteamKey).toHaveValue("");
  await expect(mteamKey).toHaveAttribute("placeholder", /已配置/);
  await expect(page.locator("body")).not.toContainText("browser-fixture-replacement-secret");

  const state = await fixtureState(page);
  expect(state.settingsWrites).toBe(1);
  expect(state.lastSettingsPayload).toMatchObject({
    expected_revision: 7,
    mteam_api_key: "browser-fixture-replacement-secret",
  });
});

test("direct media-detail deep link survives a full reload", async ({ page }) => {
  await page.goto("/#/detail/movie/42");
  await expect(page.getByRole("heading", { level: 1, name: "浏览器验收电影详情" })).toBeVisible();

  await page.reload();
  await expect(page).toHaveURL(/#\/detail\/movie\/42$/);
  await expect(page.getByRole("heading", { level: 1, name: "浏览器验收电影详情" })).toBeVisible();
  await expect(page.getByText("主详情不等待可选 M-Team 请求即可渲染。")).toBeVisible();
  expect((await fixtureState(page)).mteamRequests).toBe(0);
});

test("settings subpages preserve drafts and reveal invalid hidden fields before saving", async ({
  page,
}) => {
  await page.goto("/#/settings");
  const navigation = page.getByRole("navigation", { name: "设置子页面" });
  await expect(page.locator("#settings-automation")).toBeVisible();
  for (const name of [
    "订阅自动化",
    "订阅分类",
    "种子匹配规则",
    "qBittorrent",
    "API 密钥",
    "豆瓣登录",
    "管理访问",
  ]) {
    await navigation.getByRole("button", { name, exact: true }).click();
    await expect(page.locator(".settings-section:visible")).toHaveCount(1);
    await expect(page.locator(".settings-section:visible h2")).toHaveText(name);
  }
  const token = page.locator("#settings-access input[type=password]");
  await token.fill("short");
  await navigation.getByRole("button", { name: "API 密钥" }).click();
  await page.getByRole("button", { name: "保存全部设置" }).click();
  await expect(page.locator("#settings-access")).toBeVisible();
  await expect(token).toBeFocused();
  expect((await fixtureState(page)).settingsWrites).toBe(0);
  await token.fill("");

  await navigation.getByRole("button", { name: "qBittorrent", exact: true }).click();
  await page.getByRole("button", { name: "添加服务器", exact: true }).click();
  const serverName = page.getByLabel("显示名", { exact: true });
  await serverName.fill("跨页草稿");
  await page.getByLabel("Web UI 根地址", { exact: true }).fill("http://127.0.0.1:8080");
  await navigation.getByRole("button", { name: "订阅分类", exact: true }).click();
  await page.getByRole("button", { name: "添加分类", exact: true }).click();
  await expect(
    page.getByRole("combobox", { name: "qB 服务器", exact: true }).locator("option:checked"),
  ).toContainText("跨页草稿");
  await page.getByLabel("分类名", { exact: true }).fill("电影");
  await page.getByLabel("想看文本", { exact: true }).fill("电影");
  await page.getByLabel("qB 下载分类", { exact: true }).fill("movie");
  await page.getByLabel("qB 保存目录名", { exact: true }).fill("movies");
  await page.getByLabel("真实下载目录", { exact: true }).fill("/downloads/movies");
  await page.getByLabel("硬链接目标目录", { exact: true }).fill("/media/movies");
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await navigation.getByRole("button", { name: "qBittorrent", exact: true }).click();
  await expect(serverName).toHaveValue("跨页草稿");
  await navigation.getByRole("button", { name: "API 密钥", exact: true }).click();
  await page.getByRole("button", { name: "保存全部设置" }).click();
  await expect(page.locator("#settings-save-status")).toHaveText("设置已保存");
  expect((await fixtureState(page)).lastSettingsPayload.qb_servers[0].name).toBe("跨页草稿");
});

test("subscription poster filters keep attention states accessible without automation alerts", async ({
  page,
}) => {
  await page.goto("/#/subscriptions");
  const filters = page.getByRole("group", { name: "订阅状态筛选" });
  await expect(page.locator(".subscription-card")).toHaveCount(1);
  await expect(page.locator(".subscription-watcher-banner")).not.toHaveClass(/alert/);
  await filters.getByRole("button", { name: "需关注" }).click();
  await expect(page.locator(".subscription-card")).toHaveCount(0);
  await expect(page.getByText("该状态下暂无订阅")).toBeVisible();
  await filters.getByRole("button", { name: "全部", exact: true }).click();
  await expect(page.locator(".subscription-card")).toHaveCount(1);
  await page.locator(".subscription-card").press("Enter");
  await expect(page.locator("#page-subscription-detail")).toBeVisible();
  await expect(page.locator(".subscription-watcher-banner")).toHaveCount(0);
});
