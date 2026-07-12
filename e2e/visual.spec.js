import { mkdir, stat } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { expect, test } from "@playwright/test";

const themes = ["light", "dark"];
const routes = [
  {
    name: "search",
    path: "/#/",
    heading: "影视检索",
    headingSelector: "#page-main > .top h1",
    prepare: async (page) => {
      const search = page.getByRole("searchbox");
      await search.fill("browser fixture");
      await search.press("Enter");
      await expect(page.getByRole("button", { name: "打开详情 浏览器验收电影" })).toBeVisible();
    },
  },
  {
    name: "media-detail",
    path: "/#/detail/movie/42",
    heading: "浏览器验收电影详情",
    headingSelector: "#page-detail > .top h1",
    prepare: async (page) => {
      await expect(page.getByText("Browser.Acceptance.Movie.2160p")).toBeVisible();
    },
  },
  {
    name: "subscriptions",
    path: "/#/subscriptions",
    heading: "订阅",
    headingSelector: "#page-subscriptions > .top h1",
    prepare: async (page) => {
      await expect(page.getByRole("link", { name: "打开订阅 浏览器订阅" })).toBeVisible();
    },
  },
  {
    name: "subscription-detail",
    path: "/#/subscriptions/fixture-subscription",
    heading: "浏览器订阅",
    headingSelector: "#page-subscription-detail > .top h1",
    prepare: async (page) => {
      await expect(page.locator("[aria-label='下载进度 25%']")).toBeVisible();
    },
  },
  {
    name: "logs",
    path: "/#/logs",
    heading: "日志",
    headingSelector: "#page-logs > .top h1",
    prepare: async (page) => {
      await expect(page.getByText("M-Team 种子搜索完成：1 条候选")).toBeVisible();
    },
  },
  {
    name: "settings",
    path: "/#/settings",
    heading: "设置",
    headingSelector: "#page-settings > .top h1",
    prepare: async (page) => {
      await expect(page.getByRole("button", { name: "保存设置" })).toBeEnabled();
    },
  },
];

test.beforeEach(async ({ page }) => {
  await page.goto("/__fixture__/health");
  await page.evaluate(async () => {
    const response = await fetch("/__fixture__/reset", { method: "POST" });
    if (!response.ok) throw new Error(`fixture reset failed: ${response.status}`);
  });
});

for (const theme of themes) {
  for (const route of routes) {
    test(`@visual ${route.name} ${theme}`, async ({ page }, testInfo) => {
      await page.evaluate((mode) => {
        localStorage.setItem("tmdb-mteam-theme-mode", mode);
      }, theme);
      await page.goto(route.path);
      await route.prepare(page);

      const heading = page.locator(route.headingSelector);
      await expect(heading).toBeVisible();
      await expect(heading).toHaveText(route.heading);
      await expect(page.locator("body")).toHaveAttribute(
        "data-theme",
        theme === "dark" ? "mediahub-dark" : "mediahub",
      );

      const layout = await page.evaluate(() => ({
        viewportWidth: document.documentElement.clientWidth,
        contentWidth: document.documentElement.scrollWidth,
      }));
      expect(layout.contentWidth, "page must not overflow horizontally").toBeLessThanOrEqual(
        layout.viewportWidth + 1,
      );

      const screenshotPath = resolve(
        "test-results",
        "visual-evidence",
        testInfo.project.name,
        theme,
        `${route.name}.png`,
      );
      await mkdir(dirname(screenshotPath), { recursive: true });
      const screenshot = await page.screenshot({
        path: screenshotPath,
        fullPage: true,
        animations: "disabled",
      });
      expect(screenshot.byteLength, "screenshot buffer must be non-empty").toBeGreaterThan(1_000);
      expect(
        (await stat(screenshotPath)).size,
        "screenshot file must be non-empty",
      ).toBeGreaterThan(1_000);
    });
  }
}
