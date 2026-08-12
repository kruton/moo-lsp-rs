import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "test/browser",
  use: { baseURL: "http://127.0.0.1:4173" },
  webServer: {
    command: "npm run prepare:test && npm run build:test && npm run serve:test",
    cwd: "test/browser",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: !process.env.CI,
  },
});
