import type { UsageDailyModel, UsageDay, UsageModel, UsageRecord, UsageReportView } from "@/generated";

export interface UsageDateRange { startDate: string; endDate: string }
export type { UsageDailyModel, UsageDay, UsageModel, UsageRecord };
/** 后端 `get_usage` 的返回值（含按客户端聚合 `by_client`）。 */
export type UsageSnapshot = UsageReportView;

export type TrendMode = "tokens" | "models";
