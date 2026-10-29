import { expect, test } from "@playwright/test";
import { installBackend } from "./mock-backend";

test("proxy keeps yi-token fixed and switches a saved model without a config preview", async ({ page }) => {
  // Narrow enough for two card columns, so the first card's detail can reach the next row.
  await page.setViewportSize({ width: 720, height: 760 });
  await installBackend(page, "light");
  await page.goto("/");
  await expect(page.locator(".connection-list-summary")).toContainText("/ 4");
  await page.locator(".sidebar").getByRole("button", { name: "终端管理", exact: true }).click();
  await page.locator(".sidebar").getByRole("button", { name: "Codex CLI", exact: true }).click();

  await expect(page.getByRole("combobox", { name: "Provider" })).toBeVisible();
  await expect(page.locator(".terminal-header-status")).toContainText("未选择");
  await expect(page.locator(".terminal-preview")).toHaveCount(0);
  await expect(page.locator(".terminal-direct-footer")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "确认接入" })).toHaveCount(0);

  const gpt = page.locator(".terminal-engine-option", { hasText: "gpt" }).first();
  await gpt.click();
  await expect(page.locator(".app-toast")).toHaveCount(0);
  await expect(gpt).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".terminal-header-status")).toContainText("oa/gpt");
  await expect(page.locator(".terminal-preview")).toHaveCount(0);

  await page.getByRole("combobox", { name: "Provider" }).click();
  await page.getByRole("option", { name: "DeepSeek" }).click();
  await expect(page.locator(".terminal-engine-option")).toHaveCount(1);
  await expect(page.locator(".terminal-engine-option")).toContainText("deepseek");

  await page.getByRole("combobox", { name: "Provider" }).click();
  await page.getByRole("option", { name: "全部 Provider" }).click();
  const card = page.locator(".terminal-engine-card", { hasText: "deepseek" });
  const detail = card.locator(".terminal-engine-detail");
  await expect(detail).toBeHidden();
  await card.locator(".terminal-engine-option .terminal-engine-radio").hover();
  await page.waitForTimeout(600);
  await expect(detail).toBeHidden();
  await card.locator(".terminal-engine-option strong").hover();
  await expect(detail).toBeVisible();
  await expect(detail).toContainText("路由名称");
  await expect(detail).toContainText("上下文窗口");

  const first = page.locator(".terminal-engine-card").first();
  await first.locator(".terminal-engine-option strong").hover();
  await expect(first.locator(".terminal-engine-detail")).toBeVisible();
  const overlap = await page.evaluate(() => {
    const detail = document.querySelector(".terminal-engine-card:hover .terminal-engine-detail");
    // The card straight below the first one starts the next grid row.
    const board = document.querySelector(".terminal-model-list");
    const columns = board ? getComputedStyle(board).gridTemplateColumns.split(" ").filter(Boolean).length : 2;
    const below = document.querySelectorAll(".terminal-engine-card")[columns];
    if (!(detail instanceof HTMLElement) || !(below instanceof HTMLElement)) return { overlap: false, hitDetail: false };
    const d = detail.getBoundingClientRect();
    const b = below.getBoundingClientRect();
    const x = Math.min(Math.max(d.left, b.left) + 16, d.right - 8, b.right - 8);
    const y = Math.min(Math.max(d.top, b.top) + 16, d.bottom - 8, b.bottom - 8);
    const overlaps = d.bottom > b.top && d.top < b.bottom && d.right > b.left && d.left < b.right;
    return {
      overlap: overlaps,
      hitDetail: document.elementFromPoint(x, y)?.closest(".terminal-engine-detail") === detail,
    };
  });
  expect(overlap.overlap, "Detail should reach the card on the next row").toBe(true);
  expect(overlap.hitDetail, "Hover detail should stay above the card it overlaps").toBe(true);
});

test("disconnect keeps the selected proxy model and offers connect again", async ({ page }) => {
  await installBackend(page, "light");
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "终端管理", exact: true }).click();
  await page.locator(".sidebar").getByRole("button", { name: "Codex CLI", exact: true }).click();

  const gpt = page.locator(".terminal-engine-option", { hasText: "gpt" }).first();
  await gpt.click();
  await expect(page.locator(".terminal-header-status")).toContainText("oa/gpt");
  const actions = page.locator(".terminal-header-status");
  await actions.getByRole("button", { name: "断开", exact: true }).click();
  await expect(actions).toContainText("未接入");
  await expect(actions).not.toContainText("oa/gpt");
  await expect(gpt).toHaveAttribute("aria-pressed", "true");
  await expect(actions.getByRole("button", { name: "连接", exact: true })).toBeEnabled();
  await expect(actions.getByRole("button", { name: "断开", exact: true })).toHaveCount(0);
  await expect(page.getByRole("tab", { name: "代理接入" })).toHaveAttribute("aria-selected", "true");
});

test("hover detail stays above the cards and pager it overlaps", async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 680 });
  await installBackend(page, "light", false, false, { pagedModels: true });
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "终端管理", exact: true }).click();
  await page.locator(".sidebar").getByRole("button", { name: "Codex CLI", exact: true }).click();
  const cards = page.locator(".terminal-engine-card");
  await expect(cards.nth(3)).toBeVisible();

  const card = cards.first();
  await card.locator(".terminal-engine-option strong").hover();
  await expect(card.locator(".terminal-engine-detail")).toBeVisible();
  const covered = await card.locator(".terminal-engine-detail").evaluate((detail) => {
    const box = detail.getBoundingClientRect();
    const samples = [
      [box.left + box.width / 2, box.top + 20],
      [box.left + box.width / 2, Math.min(box.bottom - 8, window.innerHeight - 2)],
    ];
    return samples
      .flatMap(([x, y]) => {
        const el = document.elementFromPoint(x, y);
        if (el?.closest(".terminal-engine-detail") === detail) return [];
        return [el?.className || el?.tagName || "none"];
      })
      .join(" | ");
  });
  expect(covered, "Hover detail should stay above overlapping cards and the pager").toBe("");

  await page.locator("h1").hover();
  const lower = cards.last();
  await lower.scrollIntoViewIfNeeded();
  await lower.locator(".terminal-engine-option strong").hover();
  await expect(lower.locator(".terminal-engine-detail")).toBeVisible();
  const lowerCovered = await lower.locator(".terminal-engine-detail").evaluate((detail) => {
    const box = detail.getBoundingClientRect();
    const y = Math.max(box.top + 12, 2);
    const el = document.elementFromPoint(box.left + box.width / 2, y);
    return el?.closest(".terminal-engine-detail") === detail ? "" : el?.className || el?.tagName || "none";
  });
  expect(lowerCovered, "Upward hover detail should stay above the cards above it").toBe("");
});

test("proxy model grid pages through many saved models", async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 680 });
  await installBackend(page, "light", false, false, { pagedModels: true });
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "终端管理", exact: true }).click();
  await page.locator(".sidebar").getByRole("button", { name: "Codex CLI", exact: true }).click();
  const cards = page.locator(".terminal-engine-card");
  await expect(cards.first()).toBeVisible();
  const pager = page.getByRole("navigation", { name: "模型分页" });
  await expect(pager).toBeVisible();
  await expect(pager).toContainText(/1 \/ [2-9]/);
  // A page holds exactly what fits: every card sits inside the board and the page does not scroll.
  const measure = () => page.evaluate(() => {
    const board = document.querySelector(".terminal-model-list")!.getBoundingClientRect();
    const panel = document.querySelector(".main-panel")!;
    const cardsInside = [...document.querySelectorAll(".terminal-engine-card")].every(
      (card) => card.getBoundingClientRect().bottom <= board.bottom + 1,
    );
    return { cardsInside, overflow: panel.scrollHeight - panel.clientHeight };
  });
  await expect.poll(async () => (await measure()).overflow).toBeLessThanOrEqual(1);
  expect((await measure()).cardsInside).toBe(true);
  const first = await cards.first().textContent();
  await pager.getByRole("button", { name: "下一页" }).click();
  await expect(cards.first()).not.toHaveText(first ?? "");
});

test("proxy model grid adds columns and rows as the window grows", async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 680 });
  await installBackend(page, "light", false, false, { pagedModels: true });
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "终端管理", exact: true }).click();
  await page.locator(".sidebar").getByRole("button", { name: "Codex CLI", exact: true }).click();
  const cards = page.locator(".terminal-engine-card");
  await expect(cards.first()).toBeVisible();
  const columns = () =>
    page.locator(".terminal-model-list").evaluate((el) => getComputedStyle(el).gridTemplateColumns.split(" ").length);
  const small = { columns: await columns(), cards: await cards.count() };

  await page.setViewportSize({ width: 1600, height: 1000 });
  await expect.poll(columns).toBeGreaterThan(small.columns);
  await expect.poll(() => cards.count()).toBeGreaterThan(small.cards);
});
