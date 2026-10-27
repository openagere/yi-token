import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  workers: 2,
  // Vite pre-bundles dependencies on the first page load; on a cold cache that
  // can take far longer than a normal test and is not a product timeout.
  timeout: 120_000,
  use: {
    baseURL: "http://127.0.0.1:1425",
    viewport: { width: 1440, height: 960 },
    channel: process.env.PLAYWRIGHT_CHANNEL || undefined,
    trace: "retain-on-failure",
  },
  webServer: {
    command: "node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 1425",
    url: "http://127.0.0.1:1425",
    timeout: 120_000,
    reuseExistingServer: !process.env.CI,
  },
});
