import { defineConfig, devices } from "@playwright/test";

function withLoopbackNoProxy(value = "") {
  const entries = value
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean);
  return [...new Set([...entries, "127.0.0.1", "localhost"])].join(",");
}

process.env.NO_PROXY = withLoopbackNoProxy(process.env.NO_PROXY);
process.env.no_proxy = withLoopbackNoProxy(process.env.no_proxy);

const port = Number(process.env.E2E_PORT || 4174);
const baseURL = `http://127.0.0.1:${port}`;
const localChromium = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE;
const projects = [
  {
    name: "chromium-desktop",
    use: { ...devices["Desktop Chrome"] },
  },
  {
    name: "chromium-mobile",
    use: { ...devices["Pixel 7"] },
  },
];

if (process.env.PLAYWRIGHT_CROSS_BROWSER === "1") {
  projects.push({
    name: "firefox-desktop",
    use: { ...devices["Desktop Firefox"] },
  });
}

export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/*.spec.js",
  fullyParallel: false,
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  timeout: 30_000,
  expect: { timeout: 7_000 },
  reporter: process.env.CI
    ? [["line"], ["html", { open: "never", outputFolder: "playwright-report" }]]
    : "list",
  use: {
    baseURL,
    launchOptions: localChromium ? { executablePath: localChromium } : {},
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: localChromium ? "off" : "retain-on-failure",
  },
  webServer: {
    command: "node e2e/fixture-server.mjs",
    url: `${baseURL}/__fixture__/health`,
    reuseExistingServer: !process.env.CI,
    timeout: 15_000,
    env: { E2E_PORT: String(port) },
  },
  projects,
});
