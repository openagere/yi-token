import { useMemo } from "react";
import { Plug } from "lucide-react";
import { CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { useI18n } from "@/shared/i18n";
import { ProviderIcon } from "@/shared/ui";
import { CLIENT_META, clientName, formatClock, formatLatency, group } from "../format";
import type { ProxyMonitorSnapshot } from "../types";

interface Props {
  snapshot: ProxyMonitorSnapshot | undefined;
  stale: boolean;
  minutes: number;
}

/** 流量面板：窗口汇总、请求/秒与错误/秒折线（完整采样区间），以及按终端的请求分布。 */
export function TrafficPanel({ snapshot, stale, minutes }: Props) {
  const { t, language } = useI18n();
  const series = useMemo(() => {
    if (!snapshot) return [];
    const seconds = snapshot.rate_seconds || 5;
    return snapshot.samples
      .filter((sample) => sample.timestamp + seconds * 1000 <= snapshot.sampled_at)
      .map((sample) => ({
        timestamp: sample.timestamp,
        requests: sample.requests / seconds,
        errors: sample.errors / seconds,
      }));
  }, [snapshot]);
  const groups = useMemo(() => group(snapshot?.routes ?? []), [snapshot]);
  const ready = snapshot && !stale;
  const last = series.at(-1);
  const totals = snapshot?.totals;
  const count = (value: number) => value.toLocaleString(language);
  const max = Math.max(1, ...groups.map((entry) => entry.requests));
  const stats = [
    { label: t("proxy.traffic.requests"), value: ready ? snapshot.requests_per_second.toFixed(2) : "--", tone: "requests" },
    { label: t("proxy.traffic.errors"), value: ready ? (last?.errors ?? 0).toFixed(2) : "--", tone: "errors" },
    { label: t("proxy.traffic.windowRequests"), value: ready && totals ? count(totals.requests) : "--" },
    { label: t("proxy.traffic.windowErrors"), value: ready && totals ? count(totals.errors) : "--" },
    {
      label: t("proxy.traffic.avgDuration"),
      value: ready && totals && totals.completed ? formatLatency(totals.duration_ms / totals.completed) : "--",
    },
  ];

  return (
    <section className="run-traffic" aria-label={t("proxy.traffic.title")}>
      <dl className="run-traffic-stats">
        {stats.map((item) => (
          <div key={item.label} className={item.tone}>
            <dt>{item.label}</dt>
            <dd>{item.value}</dd>
          </div>
        ))}
      </dl>
      <div className="run-traffic-body">
        <div className="run-traffic-chart-card">
          <div className="run-traffic-chart-head">
            <span>{t("proxy.traffic.window", { minutes })}</span>
            <div className="run-traffic-legend" aria-hidden="true">
              <span className="requests">{t("proxy.traffic.requests")}</span>
              <span className="errors">{t("proxy.traffic.errors")}</span>
            </div>
          </div>
          <div className="run-traffic-chart" aria-label={t("proxy.traffic.chartLabel")}>
            {ready && series.length ? (
              <ResponsiveContainer width="100%" height="100%" minWidth={0} minHeight={0}>
                <LineChart data={series} margin={{ top: 8, right: 12, bottom: 0, left: 0 }}>
                  <CartesianGrid stroke="var(--chart-grid)" strokeDasharray="3 4" vertical={false} />
                  <XAxis
                    dataKey="timestamp"
                    type="number"
                    domain={["dataMin", "dataMax"]}
                    tickFormatter={(value: number) => formatClock(value, language).slice(0, 5)}
                    tick={{ fontSize: 10, fill: "var(--chart-axis)" }}
                    tickLine={false}
                    axisLine={false}
                    minTickGap={40}
                  />
                  <YAxis tick={{ fontSize: 10, fill: "var(--chart-axis)" }} tickLine={false} axisLine={false} width={36} domain={[0, "auto"]} />
                  <Tooltip
                    isAnimationActive={false}
                    labelFormatter={(value) => formatClock(Number(value), language)}
                    formatter={(value) => Number(value).toFixed(2)}
                    contentStyle={{ border: "1px solid var(--border)", borderRadius: 6, background: "var(--surface)", fontSize: 12 }}
                  />
                  <Line dataKey="requests" name={t("proxy.traffic.requests")} stroke="var(--success)" strokeWidth={1.75} dot={false} isAnimationActive={false} />
                  <Line dataKey="errors" name={t("proxy.traffic.errors")} stroke="var(--danger)" strokeWidth={1.75} dot={false} isAnimationActive={false} />
                </LineChart>
              </ResponsiveContainer>
            ) : (
              <div className="proxy-loading">{stale ? t("proxy.monitor.dataUnavailable") : snapshot ? t("proxy.monitor.waiting") : t("proxy.loading.monitor")}</div>
            )}
          </div>
        </div>
        <section className="run-traffic-clients" aria-label={t("proxy.traffic.byClient")}>
          <h3>{t("proxy.traffic.byClient")}</h3>
          {ready && groups.length ? (
            <ul>
              {groups.map((entry) => {
                const brand = CLIENT_META[entry.client]?.brand;
                return (
                  <li key={entry.client}>
                    <span className="run-client">
                      {brand ? <ProviderIcon brand={brand} size={14} /> : <Plug size={14} />}
                      {clientName(entry.client, t)}
                    </span>
                    <span className="run-traffic-count">
                      {count(entry.requests)}
                      {entry.errors > 0 && <em>{t("proxy.traffic.errorCount", { count: count(entry.errors) })}</em>}
                    </span>
                    <span className="run-traffic-share" aria-hidden="true">
                      <span style={{ width: `${(entry.requests / max) * 100}%` }} />
                    </span>
                  </li>
                );
              })}
            </ul>
          ) : (
            <p className="run-traffic-empty">{ready ? t("proxy.live.empty") : "--"}</p>
          )}
        </section>
      </div>
    </section>
  );
}
