import { currentLanguage } from "@/shared/i18n";
import type { UsageDateRange } from "./types";

const numberFormat = (locale = currentLanguage()) => new Intl.NumberFormat(locale, { maximumFractionDigits: 2 });
const integerFormat = (locale = currentLanguage()) => new Intl.NumberFormat(locale, { maximumFractionDigits: 0 });
const units = [{ value: 1e9, label: "b" }, { value: 1e6, label: "m" }, { value: 1e3, label: "k" }];

export function compactNumber(value: number, locale = currentLanguage()): string {
  let index = units.findIndex((item) => Math.abs(value) >= item.value);
  if (index > 0 && Math.round(Math.abs(value) / units[index].value * 100) / 100 >= 1000) index -= 1;
  const unit = units[index];
  return unit ? `${numberFormat(locale).format(value / unit.value)}${unit.label}` : integerFormat(locale).format(value);
}

/** 比率（0–1）格式化为百分比；无分母时为 null，显示为破折号。 */
export function ratePercent(rate: number | null): string {
  return rate === null ? "—" : `${numberFormat().format(rate * 100)}%`;
}

/** 毫秒耗时；无记录时为 null，显示为破折号。 */
export function durationMs(ms: number | null): string {
  return ms === null ? "—" : `${integerFormat().format(Math.round(ms))} ms`;
}

export function localDay(date: Date): string { return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`; }
export function shortDay(day: string): string { return `${Number(day.slice(5, 7))}/${Number(day.slice(8, 10))}`; }
export function dateRange(range: UsageDateRange): string {
  const start = range.startDate.replace(/-/g, "/");
  const end = range.endDate.replace(/-/g, "/");
  return range.startDate === range.endDate ? start : `${start} ~ ${end}`;
}
export function rangeDays(range: UsageDateRange): number {
  return Math.round((Date.parse(range.endDate) - Date.parse(range.startDate)) / 86400000) + 1;
}
