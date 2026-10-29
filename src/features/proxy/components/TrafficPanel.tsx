import { useMemo } from "react";
import { Plug } from "lucide-react";
import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { useI18n } from "@/shared/i18n";
import { ProviderIcon } from "@/shared/ui";
import { CLIENT_META, clientName, formatClock, formatLatency, formatRequestsPerMinute, group } from "../format";
import type { ProxyMonitorSnapshot } from "../types";

interface Props {
  snapshot: ProxyMonitorSnapshot | undefined;
  stale: boolean;
  minutes: number;
}

/** Monitor sampling interval in seconds (see `INTERVAL_MS` in the backend). */
const SAMPLE_SECONDS = 5;
/** The chart draws at most this many bars, so each one stays wide enough to read. */
const MAX_BARS = 60;

/** 流量面板：窗口汇总、每分钟请求与错误，以及按时间分组的请求/错误柱状图和按终端的分布。 */
export function TrafficPanel({ snapshot, stale, minutes }: Props) {
  const { t, language } = useI18n();
  const { bars, seconds, errorsPerSecond } = useMemo(() => {
    if (!snapshot) return { bars: [], seconds: SAMPLE_SECONDS, errorsPerSecond: 0 };
    // Only buckets that have closed; the current one is still filling up.
    const closed = snapshot.samples.filter((sample) => sample.timestamp + SAMPLE_SECONDS * 1000 <= snapshot.sampled_at);
    // Same window as the backend's rate: the last two closed buckets.
    const recent = closed.slice(-2);
    const perBar = Math.max(1, Math.ceil(closed.length / MAX_BARS));
    const bars: { timestamp: number; requests: number; errors: number }[] = [];
    for (let index = 0; index < closed.length; index += perBar) {
      const chunk = closed.slice(index, index + perBar);
      bars.push({
        timestamp: chunk[0].timestamp,
        requests: chunk.reduce((sum, sample) => sum + sample.requests, 0),
        errors: chunk.reduce((sum, sample) => sum + sample.errors, 0),
      });
    }
    return {
      bars,
      seconds: perBar * SAMPLE_SECONDS,
      errorsPerSecond: recent.reduce((sum, sample) => sum + sample.errors, 0) / (recent.length * SAMPLE_SECONDS || 1),
    };
  }, [snapshot]);
  const groups = useMemo(() => group(snapshot?.routes ?? []), [snapshot]);
  const ready = snapshot && !stale;
  const totals = snapshot?.totals;
  const count = (value: number) => value.toLocaleString(language);
  const max = Math.max(1, ...groups.map((entry) => entry.requests));
  const stats = [
    { label: t("proxy.traffic.requests"), value: ready ? formatRequestsPerMinute(snapshot.requests_per_second, language) : "--", tone: "requests" },
    { label: t("proxy.traffic.errors"), value: ready ? formatRequestsPerMinute(errorsPerSecond, language) : "--", tone: "errors" },
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
            <span>
              {t("proxy.traffic.window", { minutes })}
              <small>{t("proxy.traffic.bucket", { seconds })}</small>
            </span>
            <div className="run-traffic-legend" aria-hidden="true">
              <span className="requests">{t("proxy.traffic.legendRequests")}</span>
              <span className="errors">{t("proxy.traffic.legendErrors")}</span>
            </div>
          </div>
          <div className="run-traffic-chart" aria-label={t("proxy.traffic.chartLabel")}>
            {ready && bars.length ? (
              <ResponsiveContainer width="100%" height="100%" minWidth={0} minHeight={0}>
                <BarChart data={bars} margin={{ top: 8, right: 12, bottom: 0, left: 0 }} barCategoryGap="25%">
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
                  <YAxis
                    tick={{ fontSize: 10, fill: "var(--chart-axis)" }}
                    tickLine={false}
                    axisLine={false}
                    width={36}
                    allowDecimals={false}
                    domain={[0, "auto"]}
                  />
                  <Tooltip
                    isAnimationActive={false}
                    cursor={{ fill: "var(--surface-subtle)" }}
                    labelFormatter={(value) => formatClock(Number(value), language)}
                    formatter={(value) => count(Number(value))}
                    contentStyle={{ border: "1px solid var(--border)", borderRadius: 6, background: "var(--surface)", fontSize: 12 }}
                  />
                  <Bar dataKey="requests" name={t("proxy.traffic.legendRequests")} fill="var(--success)" maxBarSize={12} isAnimationActive={false} />
                  <Bar dataKey="errors" name={t("proxy.traffic.legendErrors")} fill="var(--danger)" maxBarSize={12} isAnimationActive={false} />
                </BarChart>
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
