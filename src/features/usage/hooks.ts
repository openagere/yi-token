import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { Client, UsageGroupBy } from "@/generated";
import { useStandardModels, type StandardModel } from "@/features/models";
import { useProviders, type ProviderView } from "@/features/providers";
import { usageApi } from "./api";
import { createUsageBrandResolver, createUsageModelFormatter } from "./modelNames";
import type { UsageDateRange } from "./types";

export const usageKeys = {
  all: ["usage"] as const,
  snapshot: (range: UsageDateRange, model: string, client: Client | null, groupBy: UsageGroupBy) => ["usage", range.startDate, range.endDate, model, client, groupBy] as const,
};

/** 读取用量报告（范围、模型、终端、分组维度）；切换筛选时保留上一份数据用于占位。 */
export function useUsage(range: UsageDateRange, model: string, client: Client | null, groupBy: UsageGroupBy, enabled = true) {
  return useQuery({
    queryKey: usageKeys.snapshot(range, model, client, groupBy),
    queryFn: () => usageApi.get(range, model, client, groupBy),
    enabled,
    placeholderData: keepPreviousData,
    staleTime: 0,
    refetchOnWindowFocus: false,
    meta: { errorPrefix: "usage.page.loadFailedPrefix" },
  });
}

const NO_PROVIDERS: ProviderView[] = [];
const NO_STANDARD_MODELS: StandardModel[] = [];

/** 路由 ID 的展示名与品牌图标解析器，依赖 Provider 与标准模型数据。 */
export function useUsageModelNames() {
  const providers = useProviders().data ?? NO_PROVIDERS;
  const standardModels = useStandardModels().data ?? NO_STANDARD_MODELS;
  const formatModel = useMemo(() => createUsageModelFormatter(providers), [providers]);
  const resolveBrand = useMemo(() => createUsageBrandResolver(providers, standardModels), [providers, standardModels]);
  return { formatModel, resolveBrand };
}
