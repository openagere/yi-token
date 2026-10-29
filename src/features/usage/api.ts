import type { Client, UsageGroupBy } from "@/generated";
import { invoke } from "@/shared/api";
import type { UsageDateRange, UsageSnapshot } from "./types";

declare module "@/shared/api/tauri" {
  interface CommandMap {
    get_usage: { args: { days: number | null; startDate: string; endDate: string; model: string | null; client: Client | null; groupBy: UsageGroupBy }; result: UsageSnapshot };
  }
}

export const usageApi = {
  get: (range: UsageDateRange, model: string, client: Client | null, groupBy: UsageGroupBy) => invoke("get_usage", {
    days: null,
    startDate: range.startDate,
    endDate: range.endDate,
    model: model || null,
    client,
    groupBy,
  }),
};
