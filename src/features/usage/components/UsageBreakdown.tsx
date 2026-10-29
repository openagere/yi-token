import { TERMINALS } from "@/features/terminal";
import type { Client, UsageGroupBy, UsageGroupRow } from "@/generated";
import { useI18n, type MessageKey } from "@/shared/i18n";
import type { SessionTarget } from "@/shared/navigation";
import { ProviderIcon } from "@/shared/ui";
import { durationMs, ratePercent } from "../format";
import type { UsageModelBrandResolver, UsageModelFormatter } from "../modelNames";
import type { UsageSnapshot } from "../types";
import { UsageEmpty, UsageNumber } from "./UsageCommon";

const GROUP_OPTIONS: { value: UsageGroupBy; labelKey: MessageKey }[] = [
  { value: "model", labelKey: "usage.breakdown.group.model" },
  { value: "client", labelKey: "usage.breakdown.group.client" },
  { value: "provider", labelKey: "usage.breakdown.group.provider" },
];

/** 明细分组切换：放在用量页标签栏右侧。 */
export function UsageGroupControl({ groupBy, onGroupBy }: { groupBy: UsageGroupBy; onGroupBy: (groupBy: UsageGroupBy) => void }) {
  const { t } = useI18n();
  return <div className="analytics-mode-control" role="group" aria-label={t("usage.breakdown.groupLabel")}>
    {GROUP_OPTIONS.map(({ value, labelKey }) => <button type="button" key={value} aria-pressed={groupBy === value} className={groupBy === value ? "selected" : ""} onClick={() => onGroupBy(value)}>{t(labelKey)}</button>)}
  </div>;
}

interface UsageBreakdownProps {
  snapshot: UsageSnapshot;
  groupBy: UsageGroupBy;
  onOpenSessions: (target: SessionTarget) => void;
  formatModel: UsageModelFormatter;
  resolveBrand: UsageModelBrandResolver;
}

/** 明细表（表头固定、表体滚动）：按模型 / 终端 / 服务商分组（数据来自后端 `groups`），行内"→ 会话"带筛选跳转会话页。 */
export function UsageBreakdown({ snapshot, groupBy, onOpenSessions, formatModel, resolveBrand }: UsageBreakdownProps) {
  const { t } = useI18n();
  const rows = snapshot.groups;

  function nameOf(row: UsageGroupRow): { name: string; brand?: string } {
    if (groupBy === "model") return { name: formatModel(row.key), brand: resolveBrand(row.key) };
    if (groupBy === "client") {
      const terminal = row.key ? TERMINALS.find((item) => item.id === row.key) : undefined;
      return terminal ? { name: terminal.name, brand: terminal.brand } : { name: row.key || t("usage.breakdown.apiRoute") };
    }
    return { name: row.key || row.label };
  }

  // Providers have no session filter, so only model and terminal rows link to sessions.
  function sessionTarget(row: UsageGroupRow): SessionTarget | null {
    if (groupBy === "model") return { model: row.key };
    if (groupBy === "client") return row.key ? { client: row.key as Client } : {};
    return null;
  }

  return <section className="analytics-breakdown-section" aria-label={t("usage.breakdown.title")}>
    {rows.length ? <div className="analytics-table-scroll" tabIndex={0}>
      <table className="analytics-table analytics-breakdown-table" aria-label={t("usage.breakdown.table")}>
        <thead><tr>
          <th scope="col">{t("usage.breakdown.name")}</th>
          <th scope="col">{t("usage.breakdown.requests")}</th>
          <th scope="col">{t("usage.metricTokens.input")}</th>
          <th scope="col">{t("usage.metricTokens.output")}</th>
          <th scope="col">{t("usage.breakdown.cacheRate")}</th>
          <th scope="col">{t("usage.breakdown.avgDuration")}</th>
          <th scope="col" aria-label={t("usage.breakdown.toSessions")} />
        </tr></thead>
        <tbody>
          {rows.map((row) => {
            const { name, brand } = nameOf(row);
            const target = sessionTarget(row);
            return <tr key={row.key || "api"}>
              <td><div className="analytics-model-identity"><ProviderIcon brand={brand} size={18} /><span>{name}</span></div></td>
              <td><UsageNumber value={row.requests} /></td>
              <td><UsageNumber value={row.input_tokens} /></td>
              <td><UsageNumber value={row.output_tokens} /></td>
              <td>{ratePercent(row.cache_hit_rate)}</td>
              <td>{durationMs(row.avg_duration_ms)}</td>
              <td>{target
                ? <button type="button" className="analytics-row-action" onClick={() => onOpenSessions(target)}>{t("usage.breakdown.toSessions")}</button>
                : <button type="button" className="analytics-row-action" disabled title={t("usage.breakdown.providerNoSessions")}>{t("usage.breakdown.toSessions")}</button>}
              </td>
            </tr>;
          })}
        </tbody>
      </table>
    </div> : <UsageEmpty />}
  </section>;
}
