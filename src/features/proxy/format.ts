import type { MessageKey, Translate } from "@/shared/i18n";
import type { MonitorRoute, ServerSettings } from "./types";

export function formatBytes(value: number): string {
  if (value < 1024) return `${Math.round(value)} B`;
  if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KiB`;
  if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MiB`;
  return `${(value / 1024 ** 3).toFixed(2)} GiB`;
}

export function formatCount(value: number, language: string): string {
  return value.toLocaleString(language);
}

export function formatClock(timestamp: number, language: string): string {
  return new Date(timestamp).toLocaleTimeString(language, { hour12: false });
}

/** `now` 由调用方传入（如最近一次轮询时间），保持渲染纯净。 */
export function formatUptime(startedAt: number | null, now: number, t: Translate): string {
  if (!startedAt) return "--";
  const seconds = Math.max(0, Math.floor((now - startedAt) / 1000));
  if (seconds < 60) return t("proxy.uptime.seconds", { value: seconds });
  const minutes = Math.floor(seconds / 60);
  return minutes < 60 ? t("proxy.uptime.minutes", { m: minutes, s: seconds % 60 }) : t("proxy.uptime.hours", { h: Math.floor(minutes / 60), m: minutes % 60 });
}

export function formatListenAddress(settings: ServerSettings): string {
  return `${settings.host.includes(":") ? `[${settings.host}]` : settings.host}:${settings.port}`;
}

export const CLIENT_META: Record<string, { name?: string; label?: MessageKey; brand?: string }> = {
  codex: { name: "Codex CLI", brand: "openai" },
  "claude-code": { name: "Claude Code", brand: "claude" },
  opencode: { name: "OpenCode", brand: "opencode" },
  pi: { name: "Pi", brand: "pi" },
  "deepseek-harness": { name: "DeepSeek Harness", brand: "deepseek" },
  api: { label: "proxy.monitor.clients.api" },
  unknown: { label: "proxy.monitor.clients.unknown" },
};

/** 终端的展示名称：优先使用固定名，其次是多语言标签，最后回退到原始标识。 */
export function clientName(client: string, t: Translate): string {
  const meta = CLIENT_META[client];
  return meta?.label ? t(meta.label) : (meta?.name ?? client);
}

export interface ClientGroup {
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

/** 按终端聚合监控路由（入口统计）；请求数多的终端排在前面。 */
export function group(routes: readonly MonitorRoute[]): ClientGroup[] {
  const entries = new Map<string, ClientGroup>();
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

/** 耗时：一秒以内用毫秒，之后用秒保留一位小数，避免表格里出现 30000 ms 这样的长数字。 */
export function formatLatency(value: number | null): string {
  if (value === null) return "--";
  const ms = Math.max(0, value);
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
}

/** 每秒请求数换算为每分钟；低流量时保留一位小数，流量大时取整并压缩。 */
export function formatRequestsPerMinute(perSecond: number, language: string): string {
  const perMinute = Math.max(0, perSecond) * 60;
  return perMinute < 10 ? perMinute.toFixed(1) : formatCount(Math.round(perMinute), language);
}
