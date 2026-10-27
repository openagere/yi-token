import { Activity, Boxes, Plug } from "lucide-react";
import { useI18n, type MessageKey } from "@/shared/i18n";
import { PROTOCOLS_BY_ID, type ProviderType } from "@/shared/lib";
import { ProviderIcon } from "@/shared/ui";
import { formatBytes, formatCount, formatDateTime } from "../format";
import { useProxyMonitor } from "../hooks";
import type { MonitorRoute } from "../types";

const CLIENTS: Record<string, { name?: string; label?: MessageKey; brand?: string }> = {
  codex: { name: "Codex CLI", brand: "openai" },
  "claude-code": { name: "Claude Code", brand: "claude" },
  opencode: { name: "OpenCode", brand: "opencode" },
  pi: { name: "Pi", brand: "pi" },
  "deepseek-harness": { name: "DeepSeek Harness", brand: "deepseek" },
  api: { label: "proxy.monitor.clients.api" },
  unknown: { label: "proxy.monitor.clients.unknown" },
};

interface Entry {
  client: string;
  routes: MonitorRoute[];
  requests: number;
  active: number;
  errors: number;
  completed: number;
  duration: number;
  received: number;
  sent: number;
}

function group(routes: readonly MonitorRoute[]): Entry[] {
  const entries = new Map<string, Entry>();
  for (const route of routes) {
    const entry = entries.get(route.client) ?? {
      client: route.client,
      routes: [],
      requests: 0,
      active: 0,
      errors: 0,
      completed: 0,
      duration: 0,
      received: 0,
      sent: 0,
    };
    entry.routes.push(route);
    entry.requests += route.requests;
    entry.active += route.active_requests;
    entry.errors += route.errors;
    entry.completed += route.completed;
    entry.duration += route.duration_ms;
    entry.received += route.received_bytes;
    entry.sent += route.sent_bytes;
    entries.set(route.client, entry);
  }
  return [...entries.values()]
    .map((entry) => ({ ...entry, routes: [...entry.routes].sort((a, b) => b.requests - a.requests) }))
    .sort((a, b) => b.requests - a.requests);
}

/** 入口统计：每个终端或 API 入口一张卡片，先看请求量与健康度，再看各协议的占比。 */
export function MonitorEntries({ visible }: { visible: boolean }) {
  const { t, language } = useI18n();
  // 入口统计按代理启动以来累计，与监控图表的时间窗口无关。
  const query = useProxyMonitor(15, { active: visible });
  const snapshot = query.data;
  const stale = query.isError;
  const entries = stale || !snapshot ? [] : group(snapshot.routes);
  const total = entries.reduce((sum, entry) => sum + entry.requests, 0);
  const count = (value: number) => formatCount(value, language);
  const average = (duration: number, completed: number) =>
    completed ? `${Math.round(duration / completed)} ms` : "--";

  return (
    <section className="monitor-entries" aria-label={t("proxy.monitorRoutes")}>
      <div className="monitor-entries-heading">
        <div className="proxy-section-title">
          <Boxes size={16} />
          <h2>{t("proxy.monitorRoutes")}</h2>
          <span>
            {t("proxy.monitor.routes.summary", { entries: entries.length, requests: count(total) })}
          </span>
        </div>
        <span className="monitor-entries-scope" title={t("proxy.monitor.routes.payloadHint")}>
          {t("proxy.monitor.scope", { since: snapshot ? formatDateTime(snapshot.since, language) : "--" })}
        </span>
      </div>
      {entries.length === 0 ? (
        <div className="monitor-entries-empty">
          <Activity size={20} />
          <span>{stale ? t("proxy.monitor.unavailable") : t("proxy.monitor.routes.empty")}</span>
        </div>
      ) : (
        <div className="monitor-entry-grid">
          {entries.map((entry) => {
            const meta = CLIENTS[entry.client] ?? {};
            const name = meta.label ? t(meta.label) : (meta.name ?? entry.client);
            const share = total ? Math.round((entry.requests / total) * 100) : 0;
            const errorRate = entry.completed ? (entry.errors / entry.completed) * 100 : 0;
            return (
              <article className="monitor-entry" key={entry.client}>
                <header className="monitor-entry-head">
                  <span className="monitor-entry-mark">
                    {meta.brand ? <ProviderIcon brand={meta.brand} size={20} /> : <Plug size={17} />}
                  </span>
                  <div className="monitor-entry-name">
                    <strong>{name}</strong>
                    <small>{t("proxy.monitor.routes.share", { value: share })}</small>
                  </div>
                  <div className="monitor-entry-total">
                    <strong>{count(entry.requests)}</strong>
                    <small>{t("proxy.monitor.routes.requests")}</small>
                  </div>
                </header>
                <div className="monitor-entry-share" aria-hidden="true">
                  <span style={{ width: `${share}%` }} />
                </div>
                <dl className="monitor-entry-facts">
                  <div>
                    <dt>{t("proxy.monitor.routes.active")}</dt>
                    <dd className={entry.active ? "live" : ""}>{entry.active}</dd>
                  </div>
                  <div>
                    <dt>{t("proxy.monitor.routes.failed")}</dt>
                    <dd className={entry.errors ? "bad" : ""}>
                      {entry.errors}
                      <small>{errorRate.toFixed(1)}%</small>
                    </dd>
                  </div>
                  <div>
                    <dt>{t("proxy.monitor.avgDuration")}</dt>
                    <dd>{average(entry.duration, entry.completed)}</dd>
                  </div>
                  <div>
                    <dt>{t("proxy.monitor.routes.traffic")}</dt>
                    <dd>
                      <span>↑ {formatBytes(entry.received)}</span>
                      <span>↓ {formatBytes(entry.sent)}</span>
                    </dd>
                  </div>
                </dl>
                <ul className="monitor-entry-protocols">
                  {entry.routes.map((route) => {
                    const protocol = PROTOCOLS_BY_ID[route.protocol as ProviderType];
                    const label =
                      protocol?.name ??
                      (route.protocol === "models"
                        ? t("proxy.monitor.routes.modelList")
                        : t("proxy.monitor.routes.other"));
                    const part = entry.requests ? (route.requests / entry.requests) * 100 : 0;
                    return (
                      <li key={route.protocol}>
                        <span className="monitor-entry-protocol">
                          {protocol ? (
                            <ProviderIcon brand={protocol.brand} size={13} />
                          ) : (
                            <span className="monitor-entry-protocol-dot" />
                          )}
                          {label}
                        </span>
                        <span className="monitor-entry-bar" aria-hidden="true">
                          <span style={{ width: `${part}%` }} />
                        </span>
                        <span className="monitor-entry-count">
                          {count(route.requests)}
                          {route.errors > 0 && (
                            <em>{t("proxy.monitor.routes.errorsShort", { count: route.errors })}</em>
                          )}
                        </span>
                      </li>
                    );
                  })}
                </ul>
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}
