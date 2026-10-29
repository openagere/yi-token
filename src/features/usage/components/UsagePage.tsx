import { useEffect, useState } from "react";
import { Boxes, LoaderCircle, RefreshCw, Terminal, X } from "lucide-react";
import "@/styles/usage.css";
import { TERMINALS } from "@/features/terminal";
import type { Client, UsageGroupBy } from "@/generated";
import { useI18n } from "@/shared/i18n";
import { useNavigation, useNavigationStore } from "@/shared/navigation";
import { notify } from "@/shared/notify";
import { PageHeader, ProviderIcon, Select, type SelectOption } from "@/shared/ui";
import { defaultUsageFilter, readUsageFilter, writeUsageFilter, type UsageFilter } from "../filterStorage";
import { presetRange } from "../dateRange";
import { dateRange } from "../format";
import { useUsage, useUsageModelNames } from "../hooks";
import type { TrendMode, UsageDateRange } from "../types";
import { UsageBreakdown } from "./UsageBreakdown";
import { UsageDateFilter } from "./UsageDateFilter";
import { UsageEmpty } from "./UsageCommon";
import { UsageOverview } from "./UsageOverview";

/** 用量页（spec 8.4）：一条筛选栏（日期、终端、模型、刷新），汇总卡、趋势图与明细表。 */
export function UsagePage() {
  const { t, language } = useI18n();
  const { back, close } = useNavigation();
  const openSessions = useNavigationStore((state) => state.openSessions);
  const { formatModel, resolveBrand } = useUsageModelNames();
  const [filter, setFilter] = useState<UsageFilter>(() => readUsageFilter() ?? defaultUsageFilter());
  const [trendMode, setTrendMode] = useState<TrendMode>("tokens");
  const [groupBy, setGroupBy] = useState<UsageGroupBy>("model");
  const { range, preset, client, model } = filter;
  const clientFilter: Client | null = client || null;

  useEffect(() => { writeUsageFilter(filter); }, [filter]);

  // The model dropdown lists the models present in the chosen range and terminal; it ignores the model filter itself.
  const choices = useUsage(range, "", clientFilter, "model");
  const active = useUsage(range, model, clientFilter, groupBy);
  const snapshot = active.isPlaceholderData ? undefined : active.data;
  const waiting = active.isFetching && !snapshot;
  const failed = active.isError && !snapshot;
  const stale = active.isError && Boolean(snapshot);
  const modelChoices = [...new Set([...(choices.data?.groups.map((item) => item.key) ?? []), ...(model ? [model] : [])])].sort((a, b) => formatModel(a).localeCompare(formatModel(b)) || a.localeCompare(b));
  const terminalOptions: SelectOption[] = [
    { value: "", label: t("usage.filter.allClients"), icon: Terminal },
    ...TERMINALS.map((item) => ({ value: item.id, label: item.name, leading: <ProviderIcon brand={item.brand} size={16} /> })),
  ];

  function refresh() {
    if (preset !== "custom") setFilter((current) => ({ ...current, range: presetRange(preset) }));
    void active.refetch().then((result) => { if (!result.isError) notify(t("usage.page.refreshed")); });
  }
  function changeRange(nextRange: UsageDateRange, nextPreset: UsageFilter["preset"]) {
    setFilter((current) => ({ ...current, range: nextRange, preset: nextPreset }));
  }

  return <section className="usage-page analytics-workspace">
    <PageHeader navigationInToolbar eyebrow={t("usage.page.eyebrow")} title={t("usage.page.title")} subtitle={t("pageDescriptions.usage")} onBack={back} onClose={close} closeLabel={t("usage.page.close")} />
    <div className="analytics-global-filters">
      <UsageDateFilter range={range} preset={preset} onChange={changeRange} />
      <Select className="analytics-client-select" icon={Terminal} label={t("usage.filter.clientSelect")} value={client} onValueChange={(value) => setFilter((current) => ({ ...current, client: value as Client | "" }))} options={[...terminalOptions]} />
      <Select className="analytics-model-select" icon={Boxes} label={t("usage.filter.modelSelect")} value={model} onValueChange={(value) => setFilter((current) => ({ ...current, model: value }))} disabled={waiting && !choices.data} options={[{ value: "", label: t("usage.filter.allModels"), icon: Boxes }, ...modelChoices.map((name) => ({ value: name, label: formatModel(name), leading: <ProviderIcon brand={resolveBrand(name)} size={16} /> }))]} />
      {model && <button type="button" className="icon-button analytics-clear-model" title={t("usage.filter.clearModel")} aria-label={t("usage.filter.clearModel")} onClick={() => setFilter((current) => ({ ...current, model: "" }))}><X size={14} /></button>}
      <button type="button" className="icon-button analytics-refresh" title={t("usage.page.refresh")} aria-label={t("usage.page.refresh")} disabled={waiting} onClick={refresh}><RefreshCw size={17} className={waiting ? "spinning" : ""} /></button>
    </div>
    <div className={`analytics-content ${active.isFetching && snapshot ? "is-refreshing" : ""}`} aria-busy={waiting}>
      {snapshot ? <>
        <UsageOverview snapshot={snapshot} range={range} mode={trendMode} onMode={setTrendMode} formatModel={formatModel} resolveBrand={resolveBrand} />
        <UsageBreakdown snapshot={snapshot} groupBy={groupBy} onGroupBy={setGroupBy} onOpenSessions={openSessions} formatModel={formatModel} resolveBrand={resolveBrand} />
      </> : waiting ? <div className="analytics-loading" role="status"><div className="analytics-summary-skeleton">{[0, 1, 2, 3].map((index) => <span key={index}><i /><i /><i /></span>)}</div><div><LoaderCircle size={20} className="spinning" /><span>{t("usage.page.loading")}</span></div></div> : <UsageEmpty title={t("usage.page.loadFailed")} action={refresh} actionLabel={t("usage.page.reload")} />}
    </div>
    <footer className="analytics-page-footer"><span className={stale || failed ? "analytics-stale" : ""}>{waiting ? <><LoaderCircle size={12} className="spinning" />{t("usage.page.updating")}</> : stale ? t("usage.page.staleFailed") : failed ? t("usage.page.readFailed") : <><span className="analytics-updated-dot" />{t("usage.page.updatedAt", { time: new Date(active.dataUpdatedAt).toLocaleTimeString(language, { hour12: false }) })}</>}</span><span>{model ? formatModel(model) : t("usage.filter.allModels")} · {dateRange(range)}</span></footer>
  </section>;
}
