import { defineConfig, devices } from "@playwright/test";

/**
 * Smoke tests of every page, against a production build served locally and the mock API (the recorded
 * testnet snapshots in tests/fixtures/api). PLAYWRIGHT_BASE_URL runs them against a deployed site instead.
 */
const external = process.env.PLAYWRIGHT_BASE_URL;
const port = 3100;

export default defineConfig({
  testDir: "tests/e2e",
  timeout: 45_000,
  fullyParallel: false,
  workers: 2,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: external ?? `http://127.0.0.1:${port}`,
    trace: "retain-on-failure",
  },
  projects: [
    { name: "desktop-light", use: { ...devices["Desktop Chrome"], colorScheme: "light", viewport: { width: 1366, height: 900 } } },
    { name: "desktop-dark", use: { ...devices["Desktop Chrome"], colorScheme: "dark", viewport: { width: 1366, height: 900 } } },
    { name: "mobile", use: { ...devices["Pixel 7"], colorScheme: "light" } },
  ],
  webServer: external
    ? undefined
    : [
        { command: "node tests/mock-api.mjs 4010", port: 4010, reuseExistingServer: !process.env.CI },
        {
          command: `npx next start -p ${port}`,
          port,
          reuseExistingServer: !process.env.CI,
          timeout: 120_000,
        },
      ],
});
