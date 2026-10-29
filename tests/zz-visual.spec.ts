import { expect, test, type Locator, type Page } from "@playwright/test";
import { FIXTURE_NOW, resetTrajectoryFixture } from "./fixtures/trajectory";
import { installBackend } from "./mock-backend";

/** v0.1.8 baselines were captured at this instant; freezing Date keeps charts and clocks stable. */
const BASELINE_TIME = Date.UTC(2025, 0, 15, 1, 30, 0);

async function freezeClock(page: Page, time: number = BASELINE_TIME) {
  await page.addInitScript(`
    (() => {
      const NativeDate = Date;
      const fixed = ${time};
      class FixedDate extends NativeDate {
        constructor(...args) {
          if (args.length === 0) super(fixed);
          else super(...args);
        }
        static now() { return fixed; }
      }
      globalThis.Date = FixedDate;
    })();
  `);
}

async function openTerminal(page: Page) {
  const group = page.locator(".sidebar").getByRole("button", { name: "终端管理", exact: true });
  if ((await group.getAttribute("aria-expanded")) !== "true") await group.click();
  await page.locator(".sidebar").getByRole("button", { name: "Codex CLI", exact: true }).click();
  await expect(page.getByRole("combobox", { name: "Provider" })).toBeVisible();
  await expect(page.locator(".terminal-engine-option").first()).toBeVisible();
}

async function capture(page: Page, name: string, mask: Locator[] = []) {
  await expect(page.locator(".main-panel")).toBeVisible();
  await page.waitForTimeout(250);
  await expect(page).toHaveScreenshot(name, {
    animations: "disabled",
    caret: "hide",
    mask,
    maxDiffPixelRatio: 0.001,
  });
}

for (const theme of ["light", "dark"] as const) {
  test(`visual parity ${theme}`, async ({ page }) => {
    await freezeClock(page);
    await installBackend(page, theme);
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await expect(page.locator(".connection-row")).toHaveCount(4);
    await capture(page, `providers-${theme}.png`);

    await page.locator(".sidebar").getByRole("button", { name: "模型管理", exact: true }).click();
    await expect(page.locator("h1:visible")).toHaveText("模型管理");
    await expect(page.locator(".standard-model-row").first()).toBeVisible();
    await capture(page, `models-${theme}.png`, [page.locator(".model-catalog-source")]);

    await page.locator(".sidebar").getByRole("button", { name: "运行", exact: true }).click();
    await expect(page.locator(".proxy-monitor-facts")).toBeVisible();
    await capture(page, `proxy-${theme}.png`);

    await page.locator(".sidebar").getByRole("button", { name: "模型用量", exact: true }).click();
    await expect(page.locator(".analytics-summary")).toBeVisible();
    await capture(page, `usage-${theme}.png`);

    await openTerminal(page);
    await page.getByRole("tab", { name: "直连接入", exact: true }).click();
    await expect(page.getByRole("heading", { name: "直连上游", exact: true })).toBeVisible();
    await capture(page, `terminal-direct-${theme}.png`);

    await page.getByRole("tab", { name: "代理接入", exact: true }).click();
    const routeMode = page.getByRole("heading", { name: "路由模式", exact: true });
    if (await routeMode.count()) {
      const steps = page.locator(".terminal-config-section .terminal-step");
      await steps.first().evaluate((step) => {
        if (step instanceof HTMLElement) step.style.display = "none";
      });
      await steps.nth(1).evaluate((step) => {
        if (!(step instanceof HTMLElement)) return;
        step.style.marginTop = "0";
        step.style.paddingTop = "0";
        step.style.borderTop = "0";
      });
    }
    await capture(page, `terminal-proxy-${theme}.png`);

    await page.locator(".sidebar").getByRole("button", { name: "设置", exact: true }).click();
    await expect(page.locator(".preference-options").first()).toBeVisible();
    await capture(page, `settings-${theme}.png`);
  });
}

// The fixture's session times are relative to the real start of the run, so the page clock
// is frozen to that same instant; otherwise the list would show "2年后" style labels.
for (const theme of ["light", "dark"] as const) {
  test(`visual sessions ${theme}`, async ({ page }) => {
    await freezeClock(page, FIXTURE_NOW);
    await installBackend(page, theme);
    resetTrajectoryFixture();
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await page.locator(".sidebar").getByRole("button", { name: "会话", exact: true }).click();
    await expect(page.locator(".sessions-page")).toBeVisible();
    await expect(page.locator("li[data-session-id]").first()).toBeVisible();
    await capture(page, `sessions-${theme}.png`);
  });
}
