import { expect, test, type Page } from "@playwright/test";
import {
  appendLongSessionSteps,
  resetTrajectoryFixture,
  type TrajectoryFixtureOptions,
} from "./fixtures/trajectory";
import { installBackend } from "./mock-backend";

async function openSessions(page: Page, options: TrajectoryFixtureOptions = {}) {
  await installBackend(page, "light");
  resetTrajectoryFixture(options);
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "会话", exact: true }).click();
  await expect(page.locator(".sessions-page")).toBeVisible();
}

const rows = (page: Page) => page.locator("li[data-session-id]");
const row = (page: Page, id: string) => page.locator(`li[data-session-id="${id}"]`);
const group = (page: Page, label: string) =>
  page.locator("section.session-group", { has: page.getByRole("heading", { name: label, exact: true }) });

test("list groups by day and marks inferred sessions", async ({ page }) => {
  await openSessions(page);
  await expect(group(page, "今天").locator('li[data-session-id="session-long-codex"]')).toHaveCount(1);
  await expect(group(page, "昨天").locator('li[data-session-id="session-claude-sse"]')).toHaveCount(1);
  await expect(group(page, "更早").locator('li[data-session-id="session-chain"]')).toHaveCount(1);

  const inferred = row(page, "session-chain").locator(".session-marker.inferred");
  await expect(inferred).toHaveText("≈");
  await expect(inferred).toHaveAttribute("title", "根据对话历史推断");
  await expect(row(page, "session-claude-sse").locator(".session-marker.new")).toHaveCount(1);
  await expect(row(page, "session-long-codex").locator(".session-marker")).toHaveCount(0);
});

test("client filter and title search narrow the list", async ({ page }) => {
  await openSessions(page);
  await page.getByRole("combobox", { name: "终端", exact: true }).click();
  await page.getByRole("option", { name: "Codex CLI", exact: true }).click();
  await expect(rows(page)).toHaveCount(1);
  await expect(row(page, "session-long-codex")).toBeVisible();

  await page.getByRole("combobox", { name: "终端", exact: true }).click();
  await page.getByRole("option", { name: "全部终端", exact: true }).click();
  await expect(rows(page)).toHaveCount(3);

  await page.getByRole("searchbox", { name: "搜索会话标题", exact: true }).fill("SSE");
  await expect(rows(page)).toHaveCount(1);
  await expect(row(page, "session-claude-sse")).toBeVisible();
});

test("infinite scroll loads next page", async ({ page }) => {
  await openSessions(page, { manySessions: true });
  await expect(rows(page)).toHaveCount(50);
  await page.locator(".session-list-scroll").evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  await expect(rows(page)).toHaveCount(60);
});

test("thread chips switch the trajectory", async ({ page }) => {
  await openSessions(page);
  await row(page, "session-long-codex").getByText("重构用量页").click();
  const chips = page.getByRole("group", { name: "线程", exact: true });
  await expect(chips.getByRole("button", { name: "主线程", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.locator(".trajectory-view")).toHaveAttribute("data-thread-id", "thread-main");

  await chips.getByRole("button", { name: "子 Agent ×1", exact: true }).click();
  await expect(chips.getByRole("button", { name: "子 Agent ×1", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.locator(".trajectory-view")).toHaveAttribute("data-thread-id", "thread-sub");
});

test("delete session requires confirmation", async ({ page }) => {
  await openSessions(page);
  const target = row(page, "session-claude-sse");
  await target.hover();
  await target.getByRole("button", { name: "删除会话", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("删除这个会话？");
  await dialog.getByRole("button", { name: "取消", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(target).toHaveCount(1);

  await target.hover();
  await target.getByRole("button", { name: "删除会话", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: "删除", exact: true }).click();
  await expect(target).toHaveCount(0);
  await expect(rows(page)).toHaveCount(2);
});

test("first visit shows privacy notice once", async ({ page }) => {
  await installBackend(page, "light");
  resetTrajectoryFixture({ noticeSeen: false });
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "会话", exact: true }).click();
  const notice = page.locator(".trajectory-notice");
  await expect(notice).toContainText("trajectory.db");
  await notice.getByRole("button", { name: "知道了", exact: true }).click();
  await expect(notice).toHaveCount(0);

  await page.reload();
  await page.locator(".sidebar").getByRole("button", { name: "会话", exact: true }).click();
  await expect(rows(page).first()).toBeVisible();
  await expect(notice).toHaveCount(0);
});

test("unavailable trajectory store shows settings link", async ({ page }) => {
  await openSessions(page, { available: false });
  const pane = page.locator(".sessions-detail-pane");
  await expect(pane).toContainText("轨迹库不可用");
  await pane.getByRole("button", { name: "前往设置", exact: true }).click();
  await expect(page.locator(".sessions-page")).toHaveCount(0);
});

test("unavailable store keeps the settings link reachable in the narrow layout", async ({ page }) => {
  await openSessions(page, { available: false });
  await page.setViewportSize({ width: 700, height: 800 });
  const settings = page
    .locator(".sessions-detail-pane")
    .getByRole("button", { name: "前往设置", exact: true });
  await expect(settings).toBeVisible();
  await settings.click();
  await expect(page.locator(".sessions-page")).toHaveCount(0);
});

test("sidebar session badge shows the in-flight count", async ({ page }) => {
  await installBackend(page, "light");
  await page.goto("/");
  const badge = page
    .locator(".sidebar")
    .getByRole("button", { name: "会话", exact: true })
    .locator(".nav-count");
  await expect(badge).toHaveText("1");
});

const LONG = "session-long-codex";

/** Opens the long codex session on its main thread. Extra steps are appended after the reset, before the row is clicked. */
async function openLongSession(page: Page, options: TrajectoryFixtureOptions = {}, extraSteps = 0) {
  await openSessions(page, options);
  if (extraSteps > 0) appendLongSessionSteps(extraSteps, false);
  await row(page, LONG).getByText("重构用量页").click();
  await expect(page.locator("[data-trajectory-scroll]")).toBeVisible();
}

const ledgerRow = (page: Page, key: string) =>
  page.locator(`tr[data-trajectory-row-key="${encodeURIComponent(key)}"]`);
const requestMarkers = (page: Page) => page.locator("button[data-request-status]");

/** Key and offset (top edge relative to the ledger pane) of the first record row that starts inside the pane. */
async function firstFullyVisibleRow(page: Page) {
  return page.locator("[data-trajectory-scroll]").evaluate((pane) => {
    const frame = pane.getBoundingClientRect();
    const found = [...pane.querySelectorAll("tr[data-trajectory-row-key]")].find((element) => {
      const box = element.getBoundingClientRect();
      return box.top >= frame.top && box.bottom <= frame.bottom;
    });
    if (!found) return null;
    return {
      key: found.getAttribute("data-trajectory-row-key"),
      offset: found.getBoundingClientRect().top - frame.top,
    };
  });
}

test("ledger shows kind tags, turn labels and request markers", async ({ page }) => {
  await openLongSession(page);
  await expect(page.locator(".ledger-turn-label")).toHaveCount(3);
  // The main thread holds 7 of the fixture's 8 steps; the eighth belongs to the subagent thread.
  await expect(requestMarkers(page)).toHaveCount(7);
  await expect(page.locator('button[data-request-status="error"]')).toHaveCount(1);
  await expect(page.locator('button[data-request-status="running"]')).toHaveCount(1);
  await expect(ledgerRow(page, "call:call_read_1").locator('[data-role-kind="tool"]')).toContainText("工具");
  await expect(ledgerRow(page, "step-1:req:0").locator('[data-role-kind="system"]')).toBeVisible();

  const chips = page.getByRole("group", { name: "线程", exact: true });
  await chips.getByRole("button", { name: "子 Agent ×1", exact: true }).click();
  await expect(requestMarkers(page)).toHaveCount(1);
  await expect(page.locator(".ledger-turn-label")).toHaveCount(1);
});

test("tool call and result render on one row", async ({ page }) => {
  await openLongSession(page);
  const callRow = ledgerRow(page, "call:call_read_1");
  await expect(callRow).toHaveCount(1);
  await expect(callRow).toHaveAttribute("data-kind", "tool");
  await expect(callRow).toContainText("shell");
  await expect(callRow).toContainText("→");
  await expect(callRow).toContainText("34 行");
});

test("double-click turn header folds and unfolds the turn", async ({ page }) => {
  await openLongSession(page);
  const turnHeader = ledgerRow(page, "step-1:req:1");
  const summaries = page.locator('tr[data-collapsed-summary="turn"]');
  await turnHeader.dblclick();
  await expect(summaries).toHaveCount(1);
  await expect(summaries).toContainText(/\d+ 步 · \d+ 次工具调用/);
  await turnHeader.dblclick();
  await expect(summaries).toHaveCount(0);
});

test("toolbar collapses all calls", async ({ page }) => {
  await openLongSession(page);
  const calls = page.locator('tr[data-collapsed-summary="assistant"]');
  await page.getByRole("button", { name: "折叠全部调用", exact: true }).click();
  // Three assistant rows own tool calls: step 1 (shell), step 3 (subagent) and step 5 (fetch).
  await expect(calls).toHaveCount(3);
  await page.getByRole("button", { name: "展开全部调用", exact: true }).click();
  await expect(calls).toHaveCount(0);
});

test("search filters loaded records", async ({ page }) => {
  await openLongSession(page);
  await page.getByRole("searchbox", { name: "搜索轨迹记录", exact: true }).fill("shell");
  await expect(page.locator("tr[data-trajectory-row-key]")).toHaveCount(1);
  await expect(ledgerRow(page, "call:call_read_1")).toBeVisible();
});

test("load older keeps the viewport anchored", async ({ page }) => {
  await openLongSession(page, {}, 30);
  // Native scroll anchoring would also move the viewport. Turning it off for this test isolates the ledger's own
  // scrollTop compensation, which is the behaviour under test.
  await page.addStyleTag({ content: "[data-trajectory-scroll] { overflow-anchor: none !important; }" });
  const pane = page.locator("[data-trajectory-scroll]");
  // Scroll down past the "load earlier" row, which then sits above the viewport.
  await pane.evaluate((element) => {
    element.scrollTop = 100;
  });
  const heightBefore = await pane.evaluate((element) => element.scrollHeight);
  const before = await firstFullyVisibleRow(page);
  expect(before).not.toBeNull();
  // A real click would scroll the button into view first and move the viewport, so the click is dispatched directly.
  await page
    .getByRole("button", { name: "加载更早的回合", exact: true })
    .evaluate((button: HTMLElement) => button.click());
  // The older turn is prepended and the load row disappears once the second page is in.
  await expect(page.getByRole("button", { name: "加载更早的回合", exact: true })).toHaveCount(0);
  expect(await pane.evaluate((element) => element.scrollHeight)).toBeGreaterThan(heightBefore);
  const after = await firstFullyVisibleRow(page);
  expect(after?.key).toBe(before?.key);
  expect(Math.abs((after?.offset ?? 0) - (before?.offset ?? 0))).toBeLessThanOrEqual(2);
});

test("follow tail pauses when scrolled up", async ({ page }) => {
  await installBackend(page, "light");
  resetTrajectoryFixture();
  const lastStep = appendLongSessionSteps(30, false);
  await page.goto("/");
  await page.locator(".sidebar").getByRole("button", { name: "会话", exact: true }).click();
  await expect(page.locator(".sessions-page")).toBeVisible();
  await row(page, LONG).getByText("重构用量页").click();
  const pane = page.locator("[data-trajectory-scroll]");
  // Opening the session follows the tail: the newest step is in view.
  await expect(ledgerRow(page, `${lastStep}:resp:0`)).toBeVisible();

  await pane.evaluate((element) => {
    element.scrollTop = 0;
  });
  const scrolledTop = await pane.evaluate((element) => element.scrollTop);
  const heightBefore = await pane.evaluate((element) => element.scrollHeight);
  // A new step arrives through the change poll (every 2 s).
  appendLongSessionSteps(1, true);
  await expect
    .poll(() => pane.evaluate((element) => element.scrollHeight), { timeout: 15_000 })
    .toBeGreaterThan(heightBefore);
  expect(await pane.evaluate((element) => element.scrollTop)).toBe(scrolledTop);

  await pane.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  const newest = ledgerRow(page, `step-${Number(lastStep.slice(5)) + 1}:resp:0`);
  await expect(newest).toBeVisible();
  const inView = await newest.evaluate((element) => {
    const frame = element.closest("[data-trajectory-scroll]")?.getBoundingClientRect();
    const box = element.getBoundingClientRect();
    return frame !== undefined && box.top >= frame.top && box.bottom <= frame.bottom;
  });
  expect(inView).toBe(true);
});

test("subtool expands to the subagent thread", async ({ page }) => {
  await openLongSession(page);
  const spawn = ledgerRow(page, "call:call_spawn_1");
  await expect(spawn).toHaveAttribute("data-kind", "subtool");
  await spawn.getByRole("button", { name: "展开子 Agent", exact: true }).click();
  const chips = page.getByRole("group", { name: "线程", exact: true });
  await expect(chips.getByRole("button", { name: "子 Agent ×1", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.locator(".trajectory-view")).toHaveAttribute("data-thread-id", "thread-sub");
});

const timelineTrack = (page: Page) => page.locator(".trajectory-timeline-track");
const timelineSpans = (page: Page) => page.locator("[data-timeline-span]");
const outsideRows = (page: Page) => page.locator('tr[data-timeline-focus="outside"]');

/** Centre of the track in page coordinates, with the requested horizontal fraction. */
async function trackPoint(page: Page, fraction: number) {
  const box = await timelineTrack(page).boundingBox();
  if (box === null) throw new Error("timeline track is not laid out");
  return { x: box.x + box.width * fraction, y: box.y + box.height / 2 };
}

async function dragTimeline(page: Page, from: number, to: number) {
  const start = await trackPoint(page, from);
  const end = await trackPoint(page, to);
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 8 });
  await page.mouse.up();
}

test("timeline renders three lanes and turn boundaries", async ({ page }) => {
  await openLongSession(page);
  await expect(page.locator("[data-timeline-lane]")).toHaveCount(3);
  // The main thread has three turns, so three boundary lines.
  await expect(page.locator("[data-turn-boundary]")).toHaveCount(3);
});

test("drag selection focuses ledger rows", async ({ page }) => {
  await openLongSession(page);
  await expect(outsideRows(page)).toHaveCount(0);
  await dragTimeline(page, 0.05, 0.45);
  await expect(outsideRows(page).first()).toBeAttached();
  await expect(
    page.locator('tr[data-trajectory-row-key]:not([data-timeline-focus="outside"])').first(),
  ).toBeAttached();
  await expect(ledgerRow(page, "step-8:resp:0")).toHaveAttribute("data-timeline-focus", "outside");
});

test("right click clears selection", async ({ page }) => {
  await openLongSession(page);
  await dragTimeline(page, 0.05, 0.45);
  await expect(outsideRows(page).first()).toBeAttached();
  const center = await trackPoint(page, 0.5);
  await page.mouse.click(center.x, center.y, { button: "right" });
  await expect(page.locator("[data-timeline-focus]")).toHaveCount(0);
  await expect(page.locator(".trajectory-timeline-selection")).toHaveCount(0);
});

test("mode switch changes span widths", async ({ page }) => {
  await openLongSession(page);
  const widths = () =>
    timelineSpans(page).evaluateAll((elements) =>
      elements.map((element) => Number(element.getAttribute("width")).toFixed(3)),
    );
  const equal = await widths();
  expect(equal.length).toBeGreaterThan(1);
  expect(new Set(equal).size).toBe(1);

  await page.getByRole("button", { name: "时长：等宽", exact: true }).click();
  await expect(page.getByRole("button", { name: "时长：实际", exact: true })).toBeVisible();
  const actual = await widths();
  expect(new Set(actual).size).toBeGreaterThan(1);
});

test("assistant span shows ttft segment", async ({ page }) => {
  await openLongSession(page);
  const assistant = page.locator('[data-timeline-span="message"][data-assistant-timing="true"]');
  expect(await assistant.count()).toBeGreaterThan(0);
  expect(await page.locator("[data-timeline-generation]").count()).toBeGreaterThan(0);
});

test("clicking a span selects the ledger record", async ({ page }) => {
  await openLongSession(page);
  const toolSpan = page.locator('[data-timeline-span="tool"]').first();
  const box = await toolSpan.boundingBox();
  if (box === null) throw new Error("tool span is not laid out");
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await expect(ledgerRow(page, "call:call_read_1")).toHaveAttribute("data-selected", "true");
});

test("hover shows the timing tooltip after 500 ms", async ({ page }) => {
  await openLongSession(page);
  const messageSpan = page.locator('[data-timeline-span="message"]').first();
  const box = await messageSpan.boundingBox();
  if (box === null) throw new Error("message span is not laid out");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await expect(page.getByRole("tooltip")).toContainText("首 token 1,200 ms · 生成 7,200 ms");
});

test("wheel zooms the timeline around the pointer", async ({ page }) => {
  await openLongSession(page);
  const center = await trackPoint(page, 0.5);
  await page.mouse.move(center.x, center.y);
  await page.mouse.wheel(0, -400);
  await expect(timelineTrack(page)).toHaveAttribute("data-zoomed", "true");
  await page.mouse.wheel(0, 2000);
  await expect(timelineTrack(page)).not.toHaveAttribute("data-zoomed", "true");
});

test("actual time cuts idle gaps that duration keeps", async ({ page }) => {
  await openLongSession(page);
  // Message spans in ledger order: step 1, step 2, step 3, ... The step 3 request follows a 51.6 s idle gap.
  const messageX = (position: number) =>
    page
      .locator('[data-timeline-span="message"]')
      .nth(position)
      .evaluate((element) => Number(element.getAttribute("x")));
  await page.getByRole("button", { name: "时长：等宽", exact: true }).click();
  await expect(page.getByRole("button", { name: "时长：实际", exact: true })).toBeVisible();
  // Duration keeps the gap: the window runs 60 s to 428 s, so step 3 starts near 326 of 1000.
  const kept = await messageX(2);
  await page.getByRole("switch", { name: "实际时间", exact: true }).click();
  await expect(page.getByRole("switch", { name: "实际时间", exact: true })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  // Actual time cuts the gap: the window shrinks to 265 s and step 3 starts near 258 of 1000.
  const cut = await messageX(2);
  expect(cut).toBeLessThan(kept - 40);
});

const INSPECTOR_WIDTH_KEY = "yi-token:trajectory-inspector-width";
const inspector = (page: Page) => page.locator(".trajectory-inspector");
const inspectorTabs = (page: Page) => inspector(page).getByRole("tab");
const inspectorTab = (page: Page, name: string) => inspector(page).getByRole("tab", { name, exact: true });
const requestMarker = (page: Page, number: number) =>
  page.getByRole("button", { name: `请求 #${number}`, exact: true });

test("inspector tabs depend on record kind", async ({ page }) => {
  await openLongSession(page);
  // The drawer opens a moment after a record click, and the assertions wait for the tab set they expect.
  await ledgerRow(page, "step-1:req:0").click();
  await expect(inspectorTabs(page)).toHaveText(["系统提示词", "工具"]);

  await ledgerRow(page, "step-1:resp:0").click();
  await expect(inspectorTabs(page)).toHaveText(["概述", "预览", "原始内容"]);

  await ledgerRow(page, "call:call_read_1").click();
  await expect(inspectorTabs(page)).toHaveText(["概述", "参数", "结果", "Schema", "计时"]);

  await requestMarker(page, 1).click();
  await expect(inspectorTabs(page)).toHaveText(["用量", "选项", "计时", "完整上下文"]);

  await ledgerRow(page, "step-6:req:0").click();
  await expect(inspectorTabs(page)).toHaveText(["系统提示词", "工具", "差异"]);
});

test("user row does not present the request total as its duration", async ({ page }) => {
  await openLongSession(page);
  // The request total belongs to the request marker's timing tab, not to a user row.
  await ledgerRow(page, "step-1:req:1").click();
  await expect(inspectorTabs(page)).toHaveText(["概述", "预览", "原始内容"]);
  await expect(inspector(page).locator("dt", { hasText: "时长" })).toHaveCount(0);

  await ledgerRow(page, "call:call_read_1").click();
  await expect(inspector(page).locator("dt", { hasText: "时长" })).toHaveCount(1);
});

test("preview renders markdown without raw html", async ({ page }) => {
  await openLongSession(page);
  await ledgerRow(page, "step-2:resp:0").click();
  await inspectorTab(page, "预览").click();
  const preview = inspector(page).locator(".markdown-block");
  // The script tag stays text: no element is created, and the text is visible.
  await expect(preview).toContainText("<script>window.__trajectoryXss = true</script>");
  await expect(preview.locator("script")).toHaveCount(0);
  await expect(preview.locator("strong")).toHaveText("加粗");
  const link = preview.getByRole("link", { name: "参考链接", exact: true });
  await expect(link).toHaveAttribute("target", "_blank");
  await expect(link).toHaveAttribute("rel", "noreferrer");
});

test("truncated body loads full text on demand", async ({ page }) => {
  await openLongSession(page);
  await ledgerRow(page, "step-1:resp:0").click();
  await expect(inspector(page)).toContainText("已截断，原文 4.0 KiB");
  await expect(inspector(page)).not.toContainText("完整正文");
  await inspector(page).getByRole("button", { name: "加载全文", exact: true }).click();
  await expect(inspector(page)).toContainText("完整正文 · blob-step-1");
});

test("system diff shows added tool", async ({ page }) => {
  await openLongSession(page);
  await ledgerRow(page, "step-6:req:0").click();
  await inspectorTab(page, "差异").click();
  // Step 6 adds the fetch tool compared with step 5; the added line sits in the tools section of the diff.
  await expect(
    inspector(page).locator(".prompt-diff-line.is-added", { hasText: '"name": "fetch"' }),
  ).toHaveCount(1);
});

test("full context lists reconstructed items", async ({ page }) => {
  await openLongSession(page);
  await requestMarker(page, 1).click();
  await inspectorTab(page, "完整上下文").click();
  await expect(inspector(page)).toContainText("共 1 条，回溯 2 层");
  await expect(inspector(page).locator("[data-context-item]")).toHaveCount(1);
  await expect(inspector(page)).toContainText("你是 Codex，一个编码助手。");
});

test("inspector width persists", async ({ page }) => {
  await openLongSession(page);
  await ledgerRow(page, "step-1:resp:0").click();
  await expect(inspector(page)).toBeVisible();
  const width = () => inspector(page).evaluate((element) => element.getBoundingClientRect().width);
  expect(Math.round(await width())).toBe(420);

  await inspector(page).getByRole("separator", { name: "调整检查器宽度", exact: true }).focus();
  for (let step = 0; step < 5; step++) await page.keyboard.press("ArrowLeft");
  expect(Math.round(await width())).toBe(500);
  await expect
    .poll(() => page.evaluate((key) => window.localStorage.getItem(key), INSPECTOR_WIDTH_KEY))
    .toBe("500");

  await page.reload();
  await page.locator(".sidebar").getByRole("button", { name: "会话", exact: true }).click();
  await row(page, "session-long-codex").getByText("重构用量页").click();
  await ledgerRow(page, "step-1:resp:0").click();
  await expect(inspector(page)).toBeVisible();
  expect(Math.round(await width())).toBe(500);
});

test("escape closes the inspector", async ({ page }) => {
  await openLongSession(page);
  await ledgerRow(page, "step-1:resp:0").click();
  await expect(inspector(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(inspector(page)).toHaveCount(0);
});
