import type { Client, SessionSummary } from "@/generated";
import type { Translate } from "@/shared/i18n";
import { TERMINALS } from "@/features/terminal";

export type DayGroup = "today" | "yesterday" | "week" | "earlier";

export interface DayGroupSection {
  key: DayGroup;
  items: SessionSummary[];
}

const DAY_GROUP_ORDER: DayGroup[] = ["today", "yesterday", "week", "earlier"];

/** 本地时间的 0 点，向后或向前偏移若干天。 */
function startOfDay(time: number, offsetDays = 0): number {
  const date = new Date(time);
  date.setHours(0, 0, 0, 0);
  date.setDate(date.getDate() + offsetDays);
  return date.getTime();
}

/** 本周从周一开始（本地时间）。 */
function startOfWeek(time: number): number {
  const sinceMonday = (new Date(time).getDay() + 6) % 7;
  return startOfDay(time, -sinceMonday);
}

/** 一个时间戳相对 `now` 所在的日历分组：今天、昨天、本周（周一起，不含昨天）、更早。 */
export function dayGroup(timestamp: number, now: number): DayGroup {
  if (timestamp >= startOfDay(now)) return "today";
  if (timestamp >= startOfDay(now, -1)) return "yesterday";
  if (timestamp >= startOfWeek(now)) return "week";
  return "earlier";
}

/** 按日分组，保持输入顺序（列表已按 `last_at` 倒序），跳过空分组。 */
export function groupSessionsByDay(items: readonly SessionSummary[], now: number): DayGroupSection[] {
  const buckets: Record<DayGroup, SessionSummary[]> = { today: [], yesterday: [], week: [], earlier: [] };
  for (const item of items) buckets[dayGroup(item.last_at, now)].push(item);
  return DAY_GROUP_ORDER.map((key) => ({ key, items: buckets[key] })).filter((section) => section.items.length > 0);
}

export function formatTokens(value: number, language: string): string {
  return new Intl.NumberFormat(language, { notation: "compact", maximumFractionDigits: 1 }).format(value);
}

/** 紧凑时长：`45s`、`18m`、`1h 2m`。 */
export function formatDuration(milliseconds: number): string {
  const seconds = Math.max(0, Math.round(milliseconds / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

const RELATIVE_UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["year", 365 * 86_400_000],
  ["month", 30 * 86_400_000],
  ["week", 7 * 86_400_000],
  ["day", 86_400_000],
  ["hour", 3_600_000],
  ["minute", 60_000],
];

/** 相对时间（如"3 分钟前"）；`now` 由调用方传入。 */
export function formatRelative(timestamp: number, now: number, language: string): string {
  const format = new Intl.RelativeTimeFormat(language, { numeric: "auto" });
  const diff = timestamp - now;
  for (const [unit, size] of RELATIVE_UNITS) {
    if (Math.abs(diff) >= size) return format.format(Math.round(diff / size), unit);
  }
  return format.format(0, "second");
}

/** 终端的展示名；无终端（经全局路由）显示为 api。 */
export function clientLabel(client: Client | null, t: Translate): string {
  if (client === null) return t("session.list.api");
  return TERMINALS.find((item) => item.id === client)?.name ?? client;
}

export function terminalBrand(client: Client | null): string | undefined {
  return client === null ? undefined : TERMINALS.find((item) => item.id === client)?.brand;
}

export function tokenTotal(item: Pick<SessionSummary, "input_tokens" | "output_tokens">): number {
  return item.input_tokens + item.output_tokens;
}
