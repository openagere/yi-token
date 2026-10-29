import { expect, test } from "@playwright/test";
import { installBackend } from "./mock-backend";

test("provider editor styles the capability summary on first open", async ({ page }) => {
  await installBackend(page, "dark");
  await page.goto("/");
  // Open the editor straight from the provider list, without visiting 模型管理 first:
  // the models CSS used to load only with the model pages, so the shared summary
  // rendered unstyled (block-level chips, icons stacked above their labels).
  await page.getByRole("button", { name: "OpenAI" }).first().click();
  const summary = page.locator(".model-capability-summary").first();
  await expect(summary).toBeVisible();
  await expect.poll(async () => summary.evaluate((el) => getComputedStyle(el).display)).toBe("flex");
  const chipWidths = await summary
    .locator("> span")
    .evaluateAll((els) => els.map((el) => el.getBoundingClientRect().width));
  expect(chipWidths.length).toBeGreaterThan(0);
  for (const width of chipWidths) {
    expect(width, "capability chips must stay compact on the first open").toBeLessThan(200);
  }
});

test("non-standard switch sits after the standard model and persists per mapping", async ({
  page,
}, testInfo) => {
  await installBackend(page, "dark");
  await page.goto("/");
  await page.getByRole("button", { name: "OpenAI" }).first().click();
  const row = page.locator(".connection-model-row").first();
  const standard = row.locator(".provider-standard-model-select");
  const toggle = row.getByRole("checkbox", { name: "模型 1 非标准兼容" });
  const remove = row.getByRole("button", { name: "移除模型 1" });
  await expect(toggle).toBeVisible();
  await expect(page.locator(".provider-model-binding").first()).not.toContainText("gpt(oa)");
  const [standardBox, toggleBox, removeBox] = await Promise.all([
    standard.boundingBox(),
    toggle.boundingBox(),
    remove.boundingBox(),
  ]);
  expect(standardBox!.x + standardBox!.width).toBeLessThan(toggleBox!.x + 1);
  expect(toggleBox!.x + toggleBox!.width).toBeLessThan(removeBox!.x + 1);
  await expect(page.locator(".provider-model-compatibility")).toHaveCount(0);
  await toggle.check();
  await page.screenshot({ path: testInfo.outputPath("provider-non-standard.png") });
  await page.getByRole("button", { name: "确定" }).first().click();
  await page.getByRole("button", { name: "OpenAI" }).first().click();
  await expect(page.getByRole("checkbox", { name: "模型 1 非标准兼容" })).toBeChecked();
  await page.setViewportSize({ width: 1120, height: 760 });
  await expect(page.getByRole("checkbox", { name: "模型 1 非标准兼容" })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("provider-non-standard-1120.png") });
});

test("fetching provider models adds the chosen names", async ({ page }) => {
  await installBackend(page, "light");
  await page.goto("/");
  await page.getByRole("button", { name: "OpenAI" }).first().click();
  await page.getByRole("button", { name: "获取并添加" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "从接口获取模型" })).toBeVisible();
  await expect(dialog.locator("code")).toHaveText("https://api.openai.com/v1/models");
  await expect(dialog.getByRole("checkbox", { name: "gpt" })).toBeDisabled();
  await expect(dialog.getByRole("checkbox", { name: "gpt-3.5-turbo" })).toHaveCount(0);
  await dialog.getByRole("button", { name: /显示历史模型/ }).click();
  await expect(dialog.getByRole("checkbox", { name: "gpt-3.5-turbo" })).toBeVisible();
  await dialog.getByRole("button", { name: "收起历史模型" }).click();
  const catalog = dialog.locator(".provider-model-catalog");
  const scrolling = await catalog.evaluate((element) => {
    const style = getComputedStyle(element);
    return { overflow: style.overflowY, scrolls: element.scrollHeight > element.clientHeight };
  });
  expect(scrolling.overflow).toBe("auto");
  expect(scrolling.scrolls).toBe(true);
  await dialog.getByRole("searchbox", { name: "搜索模型" }).fill("o3");
  await dialog.getByRole("checkbox", { name: "o3", exact: true }).check();
  await dialog.getByRole("button", { name: "添加所选 · 1" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("textbox", { name: "模型名 2" })).toHaveValue("o3");
});

test("provider models page through long lists and new models land on the last page", async ({ page }) => {
  await installBackend(page, "dark");
  await page.goto("/");
  await page.getByRole("button", { name: "OpenAI" }).first().click();
  const rows = page.locator(".connection-model-row");
  const pager = page.getByRole("navigation", { name: "模型分页" });
  const add = page.getByRole("button", { name: "添加模型" });
  await page.locator("#provider-tab-models").click();
  // The mock OpenAI provider starts with one model; six more make seven, i.e. two pages.
  await expect(rows).toHaveCount(1);
  for (let i = 0; i < 6; i++) await add.click();
  await expect(pager).toBeVisible();
  await expect(pager).toContainText("2 / 2");
  await expect(rows).toHaveCount(2);
  await pager.getByRole("button", { name: "上一页" }).click();
  await expect(rows).toHaveCount(5);
  await expect(pager).toContainText("1 / 2");
});
