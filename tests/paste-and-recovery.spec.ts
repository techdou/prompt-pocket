import { expect, test } from "@playwright/test";

test("an existing recovery record is clickable at the minimum window size", async ({ page }) => {
  await page.goto("/?preview");
  await page.getByTitle("新建 (Ctrl+N)").click();
  await page.locator("#f-title").fill("恢复测试");
  await page.locator("#f-body").fill("第一版正文");
  await page.locator("#f-body").press("Control+s");

  await page.getByRole("button", { name: "编辑", exact: true }).click();
  await page.locator("#f-body").fill("第二版正文");
  await page.locator("#f-body").press("Control+s");
  await expect(page.getByText("第二版正文", { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "设置", exact: true }).click();
  const entry = page.getByRole("dialog").getByRole("article").filter({ hasText: "恢复测试.md" });
  await entry.getByRole("button", { name: "查看内容" }).click();
  await expect(page.getByRole("dialog").getByText("第一版正文", { exact: true })).toBeVisible();
});

test("a replaced clipboard prompts the user to copy the prompt again", async ({ page }) => {
  await page.goto("/?preview");
  const search = page.locator("#search-input");
  await search.fill("代码审查");
  await expect(page.getByRole("button", { name: "复制 ↵" })).toBeEnabled();

  await page.evaluate(() => {
    const ipc = window.__TAURI_INTERNALS__;
    const invoke = ipc.invoke;
    ipc.invoke = async (command, args, options) => {
      if (command === "copy_or_paste") return { status: "paste_failed", reason: "clipboard_changed" };
      if (command === "plugin:window|is_visible") return true;
      if (command === "notify_paste_failed") {
        window.dispatchEvent(new CustomEvent("test:notice", { detail: args }));
      }
      return invoke(command, args, options);
    };
  });
  const notice = page.evaluate(() => new Promise((resolve) => {
    window.addEventListener("test:notice", (event) => resolve((event as CustomEvent).detail), { once: true });
  }));
  await search.press("Enter");
  await expect(page.getByRole("status")).toContainText("请重新复制提示词");
  expect(await notice).toMatchObject({ body: expect.stringContaining("剪贴板内容已变化") });
});
