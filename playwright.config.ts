import { defineConfig } from "@playwright/test";

const url = "http://127.0.0.1:1422";

export default defineConfig({
  testDir: "./tests",
  use: { baseURL: url, viewport: { width: 640, height: 440 }, channel: process.env.PLAYWRIGHT_CHROMIUM_CHANNEL },
  webServer: { command: "npm run dev -- --host 127.0.0.1 --port 1422", url },
});
