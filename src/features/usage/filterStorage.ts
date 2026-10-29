import type { Client } from "@/generated";
import { TERMINALS } from "@/features/terminal";
import { presetRange, usageDatePresets, validateRange, type UsageDatePreset } from "./dateRange";
import type { UsageDateRange } from "./types";

/** 用量页筛选条件的本地存储键；日期预设与自定义范围一并保存。 */
export const USAGE_FILTER_STORAGE_KEY = "yi-token:usage-filter";

export interface UsageFilter {
  preset: UsageDatePreset;
  range: UsageDateRange;
  /** 终端筛选；空字符串表示全部终端。 */
  client: Client | "";
  /** 模型筛选（路由 ID）；空字符串表示全部模型。 */
  model: string;
}

const PRESET_VALUES = new Set<string>(usageDatePresets.map((item) => item.value));
const CLIENT_IDS = new Set<string>(TERMINALS.map((item) => item.id));

export function defaultUsageFilter(): UsageFilter {
  return { preset: "7", range: presetRange("7"), client: "", model: "" };
}

/** 读取已保存的筛选条件；存储不可用或内容无效时返回 null（调用方使用默认值）。 */
export function readUsageFilter(): UsageFilter | null {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(USAGE_FILTER_STORAGE_KEY) ?? "null");
    if (!stored || typeof stored !== "object") return null;
    const value = stored as Record<string, unknown>;
    const savedRange = (value.range && typeof value.range === "object" ? value.range : {}) as Record<string, unknown>;
    const preset = typeof value.preset === "string" && PRESET_VALUES.has(value.preset) ? (value.preset as UsageDatePreset) : "7";
    const client = typeof value.client === "string" && CLIENT_IDS.has(value.client) ? (value.client as Client) : "";
    const model = typeof value.model === "string" ? value.model : "";
    if (preset === "custom") {
      const custom: UsageDateRange = { startDate: String(savedRange.startDate ?? ""), endDate: String(savedRange.endDate ?? "") };
      if (!validateRange(custom)) return { preset, range: custom, client, model };
      return { preset: "7", range: presetRange("7"), client, model };
    }
    return { preset, range: presetRange(preset), client, model };
  } catch {
    return null;
  }
}

/** 保存筛选条件；写入失败（如隐私模式）时静默忽略。 */
export function writeUsageFilter(filter: UsageFilter): void {
  try {
    window.localStorage.setItem(USAGE_FILTER_STORAGE_KEY, JSON.stringify(filter));
  } catch {
    // Storage is a convenience only; the page works without it.
  }
}
