import { expect, test } from "@playwright/test";

test("browser entry point loads its WASM asset and serves all APIs", async ({ page }) => {
  await page.goto("/");
  const result = await page.evaluate(() => globalThis.mooLspTest);
  expect(result.diagnostics.some((item) => item.code === "missing-semicolon")).toBe(true);
  expect(result.formatted).toBe("if (x)\n  return;\nendif\n");
  expect(result.messages[0].id).toBe(1);
});
