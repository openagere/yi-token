import { expect, test, type Page } from "@playwright/test";
import { installBackend } from "./mock-backend";

async function openRunPage(page: Page, options: { startFailure?: boolean } = {}) {
  await installBackend(page, "light", false, false, { native: false, fullPage: false, ...options });
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "运行", exact: true }).click();
  await expect(page.locator(".proxy-monitor-facts")).toBeVisible();
}

const liveRows = (page: Page) => page.locator(".run-live-table tbody tr");

test("status strip shows listen address and counts", async ({ page }) => {
  await openRunPage(page);
  const facts = page.locator(".proxy-monitor-facts");
  await expect(facts).toContainText("127.0.0.1:11435");
  await expect(facts).toContainText("连接数");
  await expect(facts).toContainText("在途请求");
  await expect(page.getByRole("button", { name: "停止代理", exact: true })).toBeVisible();
});

test("live list puts in-flight first and links to session", async ({ page }) => {
  await openRunPage(page);
  await expect(liveRows(page).first()).toHaveClass(/in-flight/);
  await expect(liveRows(page).first()).toContainText("等待首 token");
  await page.locator('tr[data-step-id="step-2"] button', { hasText: "→ 会话" }).click();
  await expect(page.locator(".sessions-page")).toBeVisible();
});

test("live row opens the step inspector in the session page", async ({ page }) => {
  // The session page selects the linked step, so its placeholder shows the step's "请求 #N" label.
  await openRunPage(page);
  await page.locator('tr[data-step-id="step-2"] button', { hasText: "→ 会话" }).click();
  await expect(page.locator(".sessions-page")).toContainText("请求 #");
});

test("client chips filter the list", async ({ page }) => {
  await openRunPage(page);
  await page.getByRole("button", { name: /Claude Code/ }).click();
  await expect(liveRows(page)).toHaveCount(1);
  await expect(liveRows(page)).toContainText("claude-sonnet-4");
  await page.getByRole("button", { name: /^全部/ }).click();
  await expect(liveRows(page)).toHaveCount(3);
});

test("errors-only toggle", async ({ page }) => {
  await openRunPage(page);
  const toggle = page.getByRole("button", { name: "仅错误", exact: true });
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await expect(liveRows(page)).toHaveCount(1);
  await expect(page.locator('tr[data-step-id="step-claude-2"]')).toBeVisible();
  await toggle.click();
  await expect(liveRows(page)).toHaveCount(3);
});

test("log drawer is collapsed by default and shows issue count", async ({ page }) => {
  await openRunPage(page);
  const drawer = page.locator(".run-log-drawer");
  await expect(drawer).toContainText("日志");
  await expect(drawer).toContainText("1 警告");
  await expect(drawer).toContainText("1 错误");
  await expect(page.locator(".proxy-logs")).not.toBeVisible();
  await drawer.getByRole("button", { name: "展开日志", exact: true }).click();
  await expect(page.locator(".proxy-logs")).toBeVisible();
});

test("start failure offers settings link", async ({ page }) => {
  await openRunPage(page, { startFailure: true });
  await expect(page.getByRole("alert")).toContainText("代理启动失败");
  await expect(page.getByRole("alert")).toContainText("端口 11435 已被占用");
  await page.getByRole("button", { name: "去设置", exact: true }).click();
  await expect(page.locator(".preference-options").first()).toBeVisible();
});
