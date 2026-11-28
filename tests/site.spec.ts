import { expect, test } from "@playwright/test";

test("the gallery loads crisp native captures and follows keyboard selection", async ({ page }) => {
  await page.goto("/site/");
  await expect(page.getByRole("heading", { level: 1 })).toContainText("随手取用");
  const screen = page.locator("#product-view");
  await expect(screen).toHaveJSProperty("naturalWidth", 1020);

  const template = page.getByRole("button", { name: "填写模板" });
  await template.click();
  await expect(screen).toHaveJSProperty("naturalWidth", 1440);
  await expect(template).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("#original-link")).toHaveAttribute("href", "assets/template.png");

  await template.press("ArrowRight");
  const organize = page.getByRole("button", { name: "整理", exact: true });
  await expect(organize).toBeFocused();
  await expect(organize).toHaveAttribute("aria-pressed", "true");
  await expect(screen).toHaveAttribute("src", "assets/manage.png");
  await expect(page.locator("#capture-link")).toHaveAttribute("href", "assets/manage.png");
});

test("English mobile uses native English screenshots and keeps download links reachable", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto("/site/");
  await page.getByRole("link", { name: "English" }).click();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page.locator("#product-view")).toHaveAttribute("src", "assets/quick-search-en.png");
  await page.getByRole("button", { name: "Fill a template" }).click();
  await expect(page.locator("#original-link")).toHaveAttribute("href", "assets/template-en.png");
  await page.getByRole("link", { name: /Download Prompt Pocket/ }).click();
  await expect(page).toHaveURL(/#install$/);
  await expect(page.getByRole("link", { name: /Windows/ })).toHaveAttribute("href", /releases\/download\/v[\d.]+\/[^/]+-setup\.exe$/);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(360);
});
