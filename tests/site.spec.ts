import { expect, test } from "@playwright/test";

test("the public site loads its images and switches the product preview", async ({ page }) => {
  await page.goto("/site/");
  await expect(page.getByRole("heading", { level: 1 })).toContainText("随时调用");
  await expect(page.locator(".hero-shot img")).toHaveJSProperty("naturalWidth", 680);
  await page.getByRole("button", { name: "填写模板" }).click();
  await expect(page.locator("#product-view")).toHaveAttribute("src", "assets/template.png");
  await expect(page.getByRole("button", { name: "填写模板" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("#product-view")).toHaveJSProperty("naturalWidth", 960);
});

test("English navigation and downloads work at mobile width", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto("/site/");
  await page.getByRole("link", { name: "English" }).click();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await page.getByRole("link", { name: "Download Prompt Pocket" }).click();
  await expect(page).toHaveURL(/#install$/);
  await expect(page.getByRole("link", { name: /Windows/ })).toHaveAttribute("href", /releases\/download\/v[\d.]+\/[^/]+-setup\.exe$/);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(360);
});
