import { invoke } from "@/shared/api";
import type { LiveSnapshot, LogsView } from "@/generated";
import type { LogLevelSetting, ProxyAction, ProxyMonitorSnapshot, ProxyRuntime, ServerSettings } from "./types";

declare module "@/shared/api/tauri" {
  interface CommandMap {
    get_settings: { args: void; result: ServerSettings };
    save_settings: { args: { host: string; port: number; logLevel: LogLevelSetting }; result: void };
    get_proxy_runtime: { args: void; result: ProxyRuntime };
    get_proxy_monitor: { args: { minutes: number }; result: ProxyMonitorSnapshot };
    control_proxy: { args: { action: ProxyAction }; result: void };
    get_logs: { args: void; result: LogsView };
    get_live_requests: { args: { sinceSeq: number }; result: LiveSnapshot };
  }
}

export const proxyApi = {
  settings: () => invoke("get_settings"),
  saveSettings: (settings: ServerSettings) => invoke("save_settings", { host: settings.host, port: settings.port, logLevel: settings.log_level }),
  runtime: () => invoke("get_proxy_runtime"),
  monitor: (minutes: number) => invoke("get_proxy_monitor", { minutes }),
  control: (action: ProxyAction) => invoke("control_proxy", { action }),
  /** 后端返回 `{ lines }`，前端按整段文本消费。 */
  logs: async () => (await invoke("get_logs")).lines.join("\n"),
  /** 实时请求：返回在途请求和 `seq` 大于 `sinceSeq` 的已完成请求。 */
  live: (sinceSeq: number) => invoke("get_live_requests", { sinceSeq }),
};
