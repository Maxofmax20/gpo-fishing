import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests/browser", timeout: 30000, workers: 1,
  reporter: [["list"]],
  use: { headless: true, launchOptions: process.env.GPO_TEST_BROWSER ? { executablePath: process.env.GPO_TEST_BROWSER } : {} },
});
