import { expect, test, type Page } from "@playwright/test";
import { installBackend } from "./mock-backend";

async function openUsage(page: Page) {
  await installBackend(page, "light");
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "模型用量", exact: true }).click();
  await expect(page.locator(".usage-page")).toBeVisible();
}

test("single filter bar without tabs", async ({ page }) => {
  await openUsage(page);
  await expect(page.getByRole("tablist")).toHaveCount(0);
  await expect(page.locator(".analytics-global-filters")).toBeVisible();
});

test("kpis include cache hit rate and ttft note", async ({ page }) => {
  await openUsage(page);
  const summary = page.getByRole("region", { name: "用量汇总", exact: true });
  await expect(summary).toContainText("缓存命中率");
  await expect(summary).toContainText("平均 TTFT");
  // The fixture reports fewer timed requests than total requests, so the note is shown.
  await expect(summary).toContainText(/基于 \d+ 次请求，自启用后统计/);
});

test("group switch changes breakdown rows", async ({ page }) => {
  await openUsage(page);
  const groups = page.getByRole("group", { name: "明细分组", exact: true });
  const table = page.getByRole("table", { name: "用量明细表格", exact: true });

  await groups.getByRole("button", { name: "模型", exact: true }).click();
  await expect(table.locator("tbody tr").first()).toContainText("GPT-5");

  await groups.getByRole("button", { name: "终端", exact: true }).click();
  await expect(table.locator("tbody tr").first()).toContainText("Codex CLI");

  await groups.getByRole("button", { name: "服务商", exact: true }).click();
  await expect(table.locator("tbody tr").first()).toContainText("OpenAI");
});

test("breakdown row links to filtered sessions", async ({ page }) => {
  await openUsage(page);
  const groups = page.getByRole("group", { name: "明细分组", exact: true });
  await groups.getByRole("button", { name: "终端", exact: true }).click();
  const table = page.getByRole("table", { name: "用量明细表格", exact: true });
  await table.locator("tbody tr").first().getByRole("button", { name: "→ 会话", exact: true }).click();

  await expect(page.locator(".sessions-page")).toBeVisible();
  await expect(page.getByRole("combobox", { name: "终端", exact: true })).toContainText("Codex CLI");
});
