import { expect, test, type Page } from "@playwright/test";
import { installBackend } from "./mock-backend";
import { resetTrajectoryFixture } from "./fixtures/trajectory";

async function openSettings(page: Page, tab?: string) {
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "设置", exact: true }).click();
  await expect(page.locator("h1:visible")).toHaveText("设置");
  if (tab) await page.getByRole("tab", { name: tab, exact: true }).click();
}

test("listen settings moved to settings page", async ({ page }) => {
  await installBackend(page, "light");
  await openSettings(page, "代理监听");
  const listen = page.locator("#settings-listen");
  await expect(listen).toBeVisible();
  await expect(listen.getByLabel("监听地址")).toHaveValue("127.0.0.1");
  await expect(listen.getByLabel("端口")).toHaveValue("11435");
  await expect(page.locator("#proxy-listen-settings")).toHaveCount(0);
  await listen.getByLabel("端口").fill("11436");
  await listen.getByRole("button", { name: "保存设置", exact: true }).click();
  await expect(listen.getByRole("status")).toContainText("所有修改已保存");
});

test("general api switches sections and saves exposed models", async ({ page }) => {
  await installBackend(page, "light");
  await openSettings(page, "通用 API");
  const api = page.locator("#settings-api");
  const nav = api.getByRole("navigation", { name: "通用 API 子分区" });
  const section = (name: RegExp) => nav.getByRole("button", { name });

  // Endpoints: one base URL, protocol paths and a folded example.
  await expect(api.locator(".api-base-url")).toHaveText("http://127.0.0.1:11435/v1");
  await expect(api).toContainText("/v1/chat/completions");
  await expect(api).toContainText("/v1/messages");
  await expect(api.locator("pre")).toBeHidden();
  await api.getByText("调用示例", { exact: true }).click();
  await expect(api.locator("pre")).toContainText('"model":"oa/gpt"');

  // Models: disabled providers are not listed.
  await section(/开放模型/).click();
  await expect(nav).toContainText("已开放 3 / 3");
  const list = api.getByRole("list", { name: "开放模型" });
  await expect(list).not.toContainText("work/gpt");
  await expect(list.getByRole("checkbox")).toHaveCount(0);
  await api.getByRole("button", { name: "仅选中的模型", exact: true }).click();
  // The selection starts from every model, so nothing is hidden until unchecked.
  await expect(list.getByRole("checkbox")).toHaveCount(3);
  await list.getByRole("checkbox", { name: "oa/gpt" }).uncheck();
  await expect(nav).toContainText("已开放 2 / 3");
  await expect(api.getByRole("status")).toContainText("有未保存的修改");

  // Fallback: shares the draft with the models pane.
  await section(/转发规则/).click();
  await expect(api).toContainText("当前默认服务商：OpenAI");
  await api.getByRole("switch", { name: "转发未知模型名" }).click();
  await expect(nav).toContainText("已关闭");
  await api.getByRole("button", { name: "保存设置", exact: true }).click();
  await expect(api.getByRole("status")).toContainText("所有修改已保存");

  await page.getByRole("tab", { name: "代理监听", exact: true }).click();
  await page.getByRole("tab", { name: "通用 API", exact: true }).click();
  await section(/开放模型/).click();
  await expect(list.getByRole("checkbox", { name: "oa/gpt" })).not.toBeChecked();
  await expect(list.getByRole("checkbox", { name: "ac/claude" })).toBeChecked();
  await section(/转发规则/).click();
  await expect(api.getByRole("switch", { name: "转发未知模型名" })).toHaveAttribute("aria-checked", "false");
});

test("trajectory toggles save", async ({ page }) => {
  await installBackend(page, "light");
  await openSettings(page, "会话轨迹");
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
  await page.getByRole("tab", { name: "会话轨迹", exact: true }).click();
  await expect(page.getByRole("switch", { name: "Codex CLI", exact: true })).toHaveAttribute("aria-checked", "false");
  await expect(page.getByRole("switch", { name: "api", exact: true })).toHaveAttribute("aria-checked", "false");
});

test("clear all requires confirmation and shows result", async ({ page }) => {
  await installBackend(page, "light");
  await openSettings(page, "会话轨迹");
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
  await openSettings(page, "会话轨迹");
  const trajectory = page.locator("#settings-trajectory");
  await expect(trajectory).toContainText("轨迹库无法打开：磁盘已满");
  await expect(trajectory).toContainText("旧文件会被重命名保留");
  await trajectory.getByRole("button", { name: "重建轨迹库", exact: true }).click();
  await expect(trajectory.getByRole("button", { name: "重建轨迹库", exact: true })).toHaveCount(0);
});

test("settings areas are tabs and each fits without page scrolling", async ({ page }) => {
  await installBackend(page, "light");
  await page.setViewportSize({ width: 1280, height: 800 });
  await openSettings(page);
  await expect(page.getByRole("tab", { name: "通用", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".preference-options").first()).toBeVisible();
  await expect(page.locator("#settings-listen")).not.toBeVisible();
  for (const tab of ["通用", "代理监听", "会话轨迹"]) {
    await page.getByRole("tab", { name: tab, exact: true }).click();
    const overflow = await page.locator(".main-panel").evaluate((el) => el.scrollHeight - el.clientHeight);
    expect(overflow, tab + " should not scroll the page").toBeLessThanOrEqual(1);
  }
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
