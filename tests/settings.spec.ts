import { expect, test, type Page } from "@playwright/test";
import { installBackend } from "./mock-backend";
import { resetTrajectoryFixture } from "./fixtures/trajectory";

async function openSettings(page: Page) {
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "设置", exact: true }).click();
  await expect(page.locator("h1:visible")).toHaveText("设置");
}

test("listen settings moved to settings page", async ({ page }) => {
  await installBackend(page, "light");
  await openSettings(page);
  const listen = page.locator("#settings-listen");
  await expect(listen).toBeVisible();
  await expect(listen.getByLabel("监听地址")).toHaveValue("127.0.0.1");
  await expect(listen.getByLabel("端口")).toHaveValue("11435");
  await expect(page.locator("#proxy-listen-settings")).toHaveCount(0);
  await listen.getByLabel("端口").fill("11436");
  await listen.getByRole("button", { name: "保存设置", exact: true }).click();
  await expect(listen.getByRole("status")).toContainText("所有修改已保存");
});

test("trajectory toggles save", async ({ page }) => {
  await installBackend(page, "light");
  await openSettings(page);
  const trajectory = page.locator("#settings-trajectory");
  const codex = trajectory.getByRole("switch", { name: "Codex CLI", exact: true });
  await expect(codex).toHaveAttribute("aria-checked", "true");
  await codex.click();
  await expect(codex).toHaveAttribute("aria-checked", "false");
  // The whole settings object is written back, so the retention days survive the toggle.
  await expect(trajectory.getByLabel("保留天数")).toHaveValue("30");
  // The api (global route) switch is a real toggle too, saved as `disabled_global`.
  const api = trajectory.getByRole("switch", { name: "api", exact: true });
  await expect(api).toHaveAttribute("aria-checked", "true");
  await api.click();
  await expect(api).toHaveAttribute("aria-checked", "false");
  await expect(trajectory.getByLabel("保留天数")).toHaveValue("30");
  await page.reload();
  await page.locator(".sidebar").getByRole("button", { name: "设置", exact: true }).click();
  await expect(page.getByRole("switch", { name: "Codex CLI", exact: true })).toHaveAttribute("aria-checked", "false");
  await expect(page.getByRole("switch", { name: "api", exact: true })).toHaveAttribute("aria-checked", "false");
});

test("clear all requires confirmation and shows result", async ({ page }) => {
  await installBackend(page, "light");
  await openSettings(page);
  const trajectory = page.locator("#settings-trajectory");
  await trajectory.getByRole("button", { name: "清空全部", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("不可恢复");
  await dialog.getByRole("button", { name: "取消", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(trajectory).not.toContainText("0 个会话");

  await trajectory.getByRole("button", { name: "清空全部", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: "清空全部", exact: true }).click();
  await expect(trajectory.getByRole("status")).toContainText("已清空");
  await expect(trajectory).toContainText("0 个会话");
});

test("rebuild shown when store unavailable", async ({ page }) => {
  await installBackend(page, "light");
  resetTrajectoryFixture({ available: false });
  await openSettings(page);
  const trajectory = page.locator("#settings-trajectory");
  await expect(trajectory).toContainText("轨迹库无法打开：磁盘已满");
  await expect(trajectory).toContainText("旧文件会被重命名保留");
  await trajectory.getByRole("button", { name: "重建轨迹库", exact: true }).click();
  await expect(trajectory.getByRole("button", { name: "重建轨迹库", exact: true })).toHaveCount(0);
});

test("terminal page links to its sessions", async ({ page }) => {
  await installBackend(page, "light");
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "终端管理", exact: true }).click();
  await page.locator(".sidebar").getByRole("button", { name: "Codex CLI", exact: true }).click();
  await page.getByRole("button", { name: "查看会话", exact: true }).click();
  await expect(page.locator(".sessions-page")).toBeVisible();
  await expect(page.locator(".session-terminal-select")).toContainText("Codex CLI");
});
