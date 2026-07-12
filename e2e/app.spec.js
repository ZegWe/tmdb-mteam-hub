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
  const mteamKey = page.getByRole("textbox", { name: "M-Team OpenAPI Key", exact: true });
  await expect(mteamKey).toHaveValue("");
  await mteamKey.fill("browser-fixture-replacement-secret");
  await page.getByRole("button", { name: "保存设置" }).click();

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
});
