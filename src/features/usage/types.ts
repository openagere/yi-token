import type { Client, UsageDailyModel, UsageDay, UsageGroupBy, UsageGroupRow, UsageModel, UsageRecord, UsageReportView, UsageTotals } from "@/generated";

export interface UsageDateRange { startDate: string; endDate: string }
export type { Client, UsageDailyModel, UsageDay, UsageGroupBy, UsageGroupRow, UsageModel, UsageRecord, UsageTotals };
/** 后端 `get_usage` 的返回值：`totals` 与 `groups` 为窗口汇总与当前分组的聚合，其余为旧字段。 */
export type UsageSnapshot = UsageReportView;

export type TrendMode = "tokens" | "models";
