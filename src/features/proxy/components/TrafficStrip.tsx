import { useMemo, useState } from "react";
import { ChevronDown, ChevronUp, Gauge } from "lucide-react";
import { Line, LineChart, ResponsiveContainer, XAxis, YAxis } from "recharts";
import { useI18n } from "@/shared/i18n";
import type { ProxyMonitorSnapshot } from "../types";

const STORAGE_KEY = "yi-token:run-traffic";

/** 收起状态只是本机偏好：读写失败（隐私模式、存储禁用）时退回展开。 */
function readCollapsed(): boolean {
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "collapsed";
  } catch {
    return false;
  }
}

function writeCollapsed(collapsed: boolean) {
  try {
    window.localStorage.setItem(STORAGE_KEY, collapsed ? "collapsed" : "expanded");
  } catch {
    // The preference is best-effort; the strip still works without it.
  }
}

interface Props {
  snapshot: ProxyMonitorSnapshot | undefined;
  stale: boolean;
}

/** 流量迷你折线：请求/秒与错误/秒，数据来自监控快照的完整采样区间。 */
export function TrafficStrip({ snapshot, stale }: Props) {
  const { t } = useI18n();
  const [collapsed, setCollapsed] = useState(readCollapsed);
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
  const last = series.at(-1);
  const summary = snapshot && !stale
    ? t("proxy.traffic.summary", {
        requests: snapshot.requests_per_second.toFixed(2),
        errors: (last?.errors ?? 0).toFixed(2),
      })
    : "--";

  function toggle() {
    const next = !collapsed;
    setCollapsed(next);
    writeCollapsed(next);
  }

  return (
    <section className="run-traffic" aria-label={t("proxy.traffic.title")}>
      <div className="run-traffic-head">
        <div className="run-traffic-title">
          <Gauge size={15} />
          <h2>{t("proxy.traffic.title")}</h2>
        </div>
        <span className="run-traffic-summary">{summary}</span>
        <div className="run-traffic-legend" aria-hidden="true">
          <span className="requests">{t("proxy.traffic.requests")}</span>
          <span className="errors">{t("proxy.traffic.errors")}</span>
        </div>
        <button
          type="button"
          className="icon-button run-traffic-toggle"
          aria-expanded={!collapsed}
          aria-label={collapsed ? t("proxy.traffic.expand") : t("proxy.traffic.collapse")}
          title={collapsed ? t("proxy.traffic.expand") : t("proxy.traffic.collapse")}
          onClick={toggle}
        >
          {collapsed ? <ChevronDown size={15} /> : <ChevronUp size={15} />}
        </button>
      </div>
      {!collapsed && (
        <div className="run-traffic-chart" aria-label={t("proxy.traffic.chartLabel")}>
          {snapshot && !stale && series.length ? (
            <ResponsiveContainer width="100%" height={64} minWidth={0}>
              <LineChart data={series} margin={{ top: 6, right: 6, bottom: 6, left: 6 }}>
                <XAxis dataKey="timestamp" type="number" domain={["dataMin", "dataMax"]} hide />
                <YAxis hide domain={[0, "auto"]} />
                <Line dataKey="requests" name={t("proxy.traffic.requests")} stroke="var(--success)" strokeWidth={1.75} dot={false} isAnimationActive={false} />
                <Line dataKey="errors" name={t("proxy.traffic.errors")} stroke="var(--danger)" strokeWidth={1.75} dot={false} isAnimationActive={false} />
              </LineChart>
            </ResponsiveContainer>
          ) : (
            <div className="proxy-loading">{stale ? t("proxy.monitor.dataUnavailable") : snapshot ? t("proxy.monitor.waiting") : t("proxy.loading.monitor")}</div>
          )}
        </div>
      )}
    </section>
  );
}
