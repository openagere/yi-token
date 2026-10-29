import type { ReactNode } from "react";
import { Activity, ArrowDownToLine, ArrowUpFromLine, Database, Gauge, type LucideIcon } from "lucide-react";
import { useI18n } from "@/shared/i18n";
import { durationMs, ratePercent } from "../format";
import type { UsageSnapshot } from "../types";
import { UsageNumber } from "./UsageCommon";

/** 汇总卡：请求数、输入/输出 token、缓存命中率、平均 TTFT；数值全部取自后端 `totals`。 */
export function UsageSummary({ snapshot }: { snapshot: UsageSnapshot }) {
  const { t } = useI18n();
  const { totals } = snapshot;
  const showTtftNote = totals.timed_requests < totals.requests;
  return <section className="analytics-summary" aria-label={t("usage.summary.label")}>
    <Kpi icon={Activity} label={t("usage.summary.requestCount")}><UsageNumber value={totals.requests} /></Kpi>
    <Kpi icon={ArrowDownToLine} label={t("usage.metricTokens.input")}><UsageNumber value={totals.input_tokens} /></Kpi>
    <Kpi icon={ArrowUpFromLine} label={t("usage.metricTokens.output")}><UsageNumber value={totals.output_tokens} /></Kpi>
    <Kpi icon={Database} label={t("usage.summary.cacheHitRate")}><span className="analytics-kpi-value">{ratePercent(totals.cache_hit_rate)}</span></Kpi>
    <Kpi icon={Gauge} label={t("usage.summary.avgTtft")}>
      <span className="analytics-kpi-value">{durationMs(totals.avg_ttft_ms)}</span>
      {showTtftNote && <small className="analytics-kpi-note">{t("usage.summary.ttftNote", { count: totals.timed_requests })}</small>}
    </Kpi>
  </section>;
}

function Kpi({ icon: Icon, label, children }: { icon: LucideIcon; label: string; children: ReactNode }) {
  return <div className="analytics-kpi"><span className="analytics-kpi-label"><Icon size={14} aria-hidden="true" />{label}</span><strong className="analytics-kpi-figure">{children}</strong></div>;
}
