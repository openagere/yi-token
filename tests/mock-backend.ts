import type { Page } from "@playwright/test";
import { resetTrajectoryFixture, trajectoryCommand } from "./fixtures/trajectory";

const fixtureBoundPages = new WeakSet<Page>();

/** Browser-only IPC fixture. Production code and real local configuration stay untouched. */
export async function installBackend(
  page: Page,
  theme: "light" | "dark",
  empty = false,
  failure = false,
  options: { native?: boolean; fullPage?: boolean; manyModels?: boolean; pagedModels?: boolean; startFailure?: boolean } = {
    native: false,
    fullPage: false,
  },
) {
  // Session and trajectory commands are served from Node-side fixture data, since the init script runs in the page.
  // A page may install the backend more than once (reload with a new scenario), but the binding can be registered once.
  // A fresh page starts from the default fixture data; tests that need other data call `resetTrajectoryFixture` afterwards.
  if (!fixtureBoundPages.has(page)) {
    fixtureBoundPages.add(page);
    resetTrajectoryFixture();
    await page.exposeFunction("__trajectoryFixture", trajectoryCommand);
  }
  await page.addInitScript(
    ({ theme, empty, failure, options }) => {
      if (!localStorage.getItem("yi-token:settings"))
        localStorage.setItem(
          "yi-token:settings",
          JSON.stringify({ version: 1, settings: { theme, font: "system", language: "zh-CN" } }),
        );
      const capabilities = {
        input_modalities: ["text", "image"],
        output_modalities: ["text"],
        context_window: 200000,
        max_output_tokens: 32000,
        effort: { support: "supported", levels: ["low", "medium", "high"], default: "medium" },
      };
      let models = empty
        ? []
        : [
            {
              id: "gpt",
              name: "GPT-5",
              company: "openai",
              brand: "openai",
              capabilities,
              provider_count: 2,
            },
            {
              id: "claude",
              name: "Claude Sonnet",
              company: "anthropic",
              brand: "anthropic",
              capabilities,
              provider_count: 1,
            },
            {
              id: "deepseek",
              name: "DeepSeek V3",
              company: "openai",
              brand: "deepseek",
              capabilities: {
                ...capabilities,
                input_modalities: ["text"],
                effort: { support: "unsupported", levels: [], default: null },
              },
              provider_count: 1,
            },
            {
              id: "gemini",
              name: "Gemini Pro",
              company: "openai",
              brand: "gemini",
              capabilities,
              provider_count: 0,
            },
            {
              id: "kimi",
              name: "Kimi K2",
              company: "openai",
              brand: "kimi",
              capabilities,
              provider_count: 0,
            },
          ];
      let providers = empty
        ? []
        : [
            ["openai", "OpenAI", "oa", "responses", "https://api.openai.com/v1", "gpt"],
            ["anthropic", "Anthropic", "ac", "anthropic", "https://api.anthropic.com", "claude"],
            ["deepseek", "DeepSeek", "ds", "openai_chat", "https://api.deepseek.com/v1", "deepseek"],
            ["gateway", "Work Gateway", "work", "responses", "https://gateway.example.com/v1", "gpt"],
          ].map(([id, name, short_code, provider_type, base_url, modelId], index) => ({
            id,
            name,
            short_code,
            provider_type,
            base_url,
            api_key: "test-key",
            enabled: index !== 3,
            thinking: "medium",
            extra: {},
            is_default: index === 0,
            models: [
              {
                id: String(index + 1),
                provider_id: id,
                route_id: `${short_code}/${modelId}`,
                name: modelId,
                standard_model_id: modelId,
                capabilities,
              },
            ],
          }));
      if (options.manyModels && !empty) {
        providers[0].models.push(
          ...["gpt-mini", "gpt-large", "gpt-reasoning", "gpt-fast"].map((name, index) => ({
            ...structuredClone(providers[0].models[0]),
            id: String(100 + index),
            route_id: `oa/${name}`,
            name,
            standard_model_id: name,
          })),
        );
      }
      if (options.pagedModels && !empty) {
        providers[0].models.push(
          ...Array.from({ length: 14 }, (_, index) => ({
            ...structuredClone(providers[0].models[0]),
            id: String(200 + index),
            route_id: `oa/gpt-paged-${index + 1}`,
            name: `gpt-paged-${index + 1}`,
          })),
        );
      }
      if (options.fullPage && !empty) {
        providers.push(
          ...[4, 5].map((index) => ({
            ...structuredClone(providers[0]),
            id: `provider-${index}`,
            name: `Gateway ${index}`,
            short_code: `gw${index}`,
            is_default: false,
            models: [
              {
                ...structuredClone(providers[0].models[0]),
                id: `gw-${index}`,
                provider_id: `provider-${index}`,
                route_id: `gw${index}/gpt`,
              },
            ],
          })),
        );
        models.push({ ...structuredClone(models[0]), id: "extra", name: "Extra Model", provider_count: 0 });
      }
      let maximized = false;
      type IpcArgs = {
        action?: string;
        host?: string;
        port?: number;
        logLevel?: string;
        id?: string;
        passphrase?: string;
        title?: string;
        client?: string;
        modelId?: string | null;
        providerId?: string;
        model?: string | { id: string };
        provider?: { id: string };
        models?: unknown[];
        profile?: {
          client?: string;
          models?: unknown[];
          provider_id?: string;
          model_source?: string;
          model?: string | null;
          route_mode?: "fixed" | "auto";
        };
      };
      const nativeCommands: { command: string; args: IpcArgs }[] = [];
      /** 记录加密配置导入 / 导出的 IPC 调用，供 UI 测试断言参数。 */
      const transferCalls: { command: string; args: IpcArgs }[] = [];
      const ipcError = (code: string, message: string) => ({ code, message });
      Object.defineProperty(window, "isTauri", { value: options.native, configurable: true });
      Object.defineProperty(window, "__nativeCommands", { value: nativeCommands, configurable: true });
      Object.defineProperty(window, "__transferCalls", { value: transferCalls, configurable: true });
      let phase = options.startFailure ? "error" : "running";
      // Live registry: the in-flight request stays put; the second poll reveals one more completed request (incremental merge).
      let liveCalls = 0;
      const liveNow = Date.now();
      const liveInFlight = {
        seq: 0,
        step_id: "step-8",
        client: "codex",
        model: "gpt-5",
        provider_name: "OpenAI",
        started_at: liveNow - 4000,
        first_token_at: null,
        finished_at: null,
        phase: "waiting",
        status: null,
        input_tokens: 9000,
        output_tokens: 0,
        session_id: null,
      };
      const liveDone = {
        seq: 1,
        step_id: "step-2",
        client: "codex",
        model: "gpt-5",
        provider_name: "OpenAI",
        started_at: liveNow - 120000,
        first_token_at: liveNow - 119000,
        finished_at: liveNow - 112000,
        phase: "done",
        status: 200,
        input_tokens: 12000,
        output_tokens: 800,
        session_id: "session-long-codex",
      };
      const liveFailed = {
        seq: 2,
        step_id: "step-claude-2",
        client: "claude-code",
        model: "claude-sonnet-4",
        provider_name: "Anthropic",
        started_at: liveNow - 60000,
        first_token_at: null,
        finished_at: liveNow - 30000,
        phase: "error",
        status: 502,
        input_tokens: 3000,
        output_tokens: 0,
        session_id: "session-claude-sse",
      };
      type MockEngine = { client: string; model_id: string; exposed_name: string; provider_name: string };
      let activeEngines: MockEngine[] = [];
      let settings = { host: "127.0.0.1", port: 11435, log_level: "info" };
      const now = Date.now();
      const day = new Date().toLocaleDateString("en-CA");
      const usageModels = models.slice(0, 3).map((model, index) => ({
        model: model.name,
        requests: 25 + index * 10,
        input_tokens: 230000 + index * 70000,
        output_tokens: 30000 + index * 15000,
        cached_tokens: 80000,
        reasoning_tokens: 4000,
      }));
      const totals = usageModels.reduce(
        (total, item) => ({
          total_requests: total.total_requests + item.requests,
          input_tokens: total.input_tokens + item.input_tokens,
          output_tokens: total.output_tokens + item.output_tokens,
          cached_tokens: total.cached_tokens + item.cached_tokens,
          reasoning_tokens: total.reasoning_tokens + item.reasoning_tokens,
        }),
        { total_requests: 0, input_tokens: 0, output_tokens: 0, cached_tokens: 0, reasoning_tokens: 0 },
      );
      const usage = {
        ...totals,
        days: [{ day, ...totals }],
        daily_models: usageModels.map((item) => ({
          day,
          model: item.model,
          tokens: item.input_tokens + item.output_tokens,
        })),
        models: usageModels,
        recent: usageModels.flatMap((model, index) =>
          Array.from({ length: 4 }, (_, i) => ({
            id: String(index * 4 + i),
            requested_at: now - i * 90000,
            provider_name: providers[index]?.name,
            protocol: providers[index]?.provider_type,
            model: model.model,
            input_tokens: 12000,
            output_tokens: 900,
            cached_tokens: 2000,
            reasoning_tokens: 300,
          })),
        ),
      };
      // Aggregates in the shape of the backend `totals` and `groups` (UsageReportView), keyed by UsageGroupBy.
      const usageTotals = {
        requests: totals.total_requests,
        input_tokens: totals.input_tokens,
        output_tokens: totals.output_tokens,
        cached_tokens: totals.cached_tokens,
        reasoning_tokens: totals.reasoning_tokens,
        cache_hit_rate: totals.cached_tokens / totals.input_tokens,
        avg_ttft_ms: 1420,
        avg_duration_ms: 6800,
        timed_requests: 40,
      };
      const groupRow = (key: string, label: string, item: (typeof usageModels)[number]) => ({
        key,
        label,
        requests: item.requests,
        input_tokens: item.input_tokens,
        output_tokens: item.output_tokens,
        cached_tokens: item.cached_tokens,
        cache_hit_rate: item.cached_tokens / item.input_tokens,
        avg_duration_ms: 5200,
      });
      const usageGroups: Record<string, ReturnType<typeof groupRow>[]> = {
        model: usageModels.map((item) => groupRow(item.model, item.model, item)),
        client: [
          groupRow("codex", "codex", usageModels[0]),
          groupRow("claude-code", "claude-code", usageModels[1]),
          groupRow("", "", usageModels[2]),
        ],
        provider: [
          groupRow("OpenAI", "OpenAI", usageModels[0]),
          groupRow("Anthropic", "Anthropic", usageModels[1]),
          groupRow("DeepSeek", "DeepSeek", usageModels[2]),
        ],
      };
      const totalsMonitor = {
        connections: 3,
        accepted_connections: 142,
        peak_connections: 8,
        active_requests: 1,
        peak_requests: 4,
        requests: 105,
        completed: 104,
        errors: 1,
        received_bytes: 1590000,
        sent_bytes: 832000,
        duration_ms: 239000,
      };
      const profiles = ["codex", "claude-code", "opencode", "pi", "deepseek-harness"].map((client) => ({
        client,
        config_path: `C:/Users/user/.${client}/config.json`,
        exists: true,
        active: true,
        active_mode: "proxy",
        config_error: null,
        direct_profile: null,
        profile: {
          client,
          models: [{ provider_id: providers[0]?.id, model: providers[0]?.models[0].route_id }],
          default_model: providers[0]?.models[0].route_id ?? "",
          protocol:
            client === "pi" || client === "opencode" || client === "deepseek-harness" ? "openai_chat" : null,
          route_mode: "fixed" as "fixed" | "auto",
          active_engine: null as MockEngine | null,
        },
      }));
      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        value: {
          metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
          invoke: async (command: string, args: IpcArgs = {}) => {
            if (
              failure &&
              [
                "list_providers",
                "list_standard_models",
                "get_model_catalog_info",
                "get_proxy_runtime",
                "get_usage",
                "get_terminal_profiles",
              ].includes(command)
            )
              throw ipcError("internal", "Fixture: backend unavailable");
            if (command.startsWith("plugin:window|")) nativeCommands.push({ command, args });
            switch (command) {
              case "plugin:window|is_maximized":
                return maximized;
              case "plugin:window|toggle_maximize":
                maximized = !maximized;
                return;
              case "plugin:window|minimize":
                return;
              case "plugin:window|start_dragging":
                return;
              case "plugin:window|close":
                return;
              case "plugin:window|set_theme":
                return;
              case "plugin:window|set_title":
                return;
              case "list_providers":
                return structuredClone(providers);
              case "list_standard_models":
                return structuredClone(models);
              case "get_model_catalog_info":
                return {
                  source: "override",
                  path: "C:/Users/user/.yi-token/models.json",
                  version: "1",
                  count: models.length,
                  loaded_at: Math.floor(now / 1000),
                };
              case "get_proxy_runtime":
                return {
                  phase,
                  desired_running: phase === "running",
                  started_at: now - 3600000,
                  address: options.startFailure ? null : "127.0.0.1:11435",
                  last_error: options.startFailure ? "端口 11435 已被占用" : null,
                  active_requests: 1,
                  active_connections: 3,
                  settings,
                };
              case "control_proxy":
                phase = args.action === "stop" ? "stopped" : "running";
                return;
              case "get_proxy_monitor":
                return {
                  since: now - 3600000,
                  sampled_at: Date.now(),
                  totals: totalsMonitor,
                  rate_seconds: 5,
                  received_per_second: 3200,
                  sent_per_second: 1700,
                  requests_per_second: 0.8,
                  samples: Array.from({ length: 30 }, (_, i) => ({
                    timestamp: now - (30 - i) * 5000,
                    connections: 2 + (i % 4),
                    active_requests: i % 3,
                    requests: 3 + (i % 2),
                    errors: 0,
                    received_bytes: 12000 + i * 100,
                    sent_bytes: 5000 + i * 30,
                  })),
                  routes: [
                    { client: "codex", protocol: "responses", ...totalsMonitor },
                    { client: "claude-code", protocol: "anthropic", ...totalsMonitor, requests: 12, completed: 11, errors: 1 },
                  ],
                };
              case "get_live_requests": {
                liveCalls += 1;
                const revealed = liveCalls >= 2;
                const completed = [liveDone, ...(revealed ? [liveFailed] : [])].filter(
                  (item) => item.seq > (args.sinceSeq ?? 0),
                );
                return {
                  in_flight: [liveInFlight],
                  completed,
                  seq: revealed ? 2 : 1,
                };
              }
              case "get_settings":
                return settings;
              case "save_settings":
                settings = { host: args.host, port: args.port, log_level: args.logLevel };
                return;
              case "get_usage":
                return structuredClone({
                  ...usage,
                  totals: usageTotals,
                  groups: usageGroups[args.groupBy ?? "model"] ?? [],
                });
              case "get_logs":
                return {
                  lines: [
                    "2026-09-29T18:00:00.000Z INFO Proxy listening on 127.0.0.1:11435",
                    "2026-09-29T18:01:00.000Z INFO POST /v1/responses 200",
                    "2026-09-29T18:02:00.000Z WARN upstream responded slowly",
                    "2026-09-29T18:03:00.000Z ERROR upstream timeout",
                  ],
                };
              case "get_terminal_profiles":
                return structuredClone(profiles);
              case "get_active_engines":
                return structuredClone(activeEngines);
              case "set_active_engine": {
                const target = profiles.find((item) => item.client === args.client);
                const selected = providers
                  .flatMap((provider) => provider.models.map((model) => ({ provider, model })))
                  .find(({ model }) => model.id === args.modelId);
                activeEngines =
                  args.modelId && selected && target
                    ? [
                        ...activeEngines.filter((item) => item.client !== target.client),
                        {
                          client: target.client,
                          model_id: selected.model.id,
                          exposed_name: selected.model.route_id,
                          provider_name: selected.provider.name,
                        },
                      ]
                    : activeEngines.filter((item) => item.client !== args.client);
                if (target)
                  target.profile.active_engine =
                    activeEngines.find((item) => item.client === target.client) ?? null;
                return;
              }
              case "discover_provider_models": {
                const discovered = (id: string, extra: Record<string, unknown> = {}) => ({
                  id,
                  name: null,
                  display_name: null,
                  status: null,
                  domain: "LLM",
                  task_types: ["text_generation"],
                  context_window: null,
                  max_output_tokens: null,
                  input_modalities: ["text"],
                  output_modalities: ["text"],
                  tools: null,
                  created: null,
                  ...extra,
                });
                return [
                  discovered("gpt"),
                  discovered("o3"),
                  discovered("gpt-3.5-turbo"),
                  discovered("gpt-4o-2024-08-06"),
                  discovered("gpt-4.1-retired", { status: "Shutdown" }),
                  discovered("text-embedding-3", { domain: "Embedding", task_types: ["embedding"] }),
                  ...Array.from({ length: 22 }, (_, index) => discovered(`remote-${index + 1}`)),
                ];
              }
              case "preview_terminal_config":
                return {
                  config_path: profiles[0].config_path,
                  endpoint: "http://127.0.0.1:11435",
                  files: [
                    {
                      path: "config.toml",
                      content:
                        args.profile?.route_mode === "auto"
                          ? 'model = "yi-token"\nmodel_provider = "yi"'
                          : 'model = "oa/gpt"\nmodel_provider = "yi"',
                    },
                  ],
                };
              case "apply_terminal_config": {
                const target = profiles.find((item) => item.client === args.profile?.client);
                if (target && args.profile)
                  Object.assign(target, {
                    active: true,
                    active_mode: "proxy",
                    profile: {
                      ...structuredClone(args.profile),
                      route_mode: args.profile.route_mode ?? "fixed",
                      active_engine: activeEngines.find((item) => item.client === target.client) ?? null,
                    },
                  });
                return {
                  config_path: profiles[0].config_path,
                  endpoint: "http://127.0.0.1:11435",
                  backup_paths: [],
                  model_count: args.profile?.route_mode === "auto" ? 1 : (args.profile?.models?.length ?? 0),
                };
              }
              case "preview_terminal_direct_config": {
                const model =
                  args.profile?.model_source === "provider"
                    ? (args.profile.model ??
                      providers.find((provider) => provider.id === args.profile?.provider_id)?.models[0]
                        ?.name ??
                      null)
                    : null;
                return {
                  config_path: profiles[0].config_path,
                  endpoint: "https://api.openai.com/v1",
                  files: [
                    {
                      path: "config.toml",
                      content: `model_provider = "yi_direct_openai"\n${model ? `model = "${model}"\n` : ""}[model_providers.yi_direct_openai]\nbase_url = "https://api.openai.com/v1"\nexperimental_bearer_token = "••••••••"`,
                    },
                  ],
                };
              }
              case "disconnect_terminal": {
                const target = profiles.find((item) => item.client === args.client);
                if (target) Object.assign(target, { active: false, active_mode: "none" });
                return {
                  config_path: profiles[0].config_path,
                  endpoint: "",
                  backup_paths: [],
                  model_count: 0,
                };
              }
              case "apply_terminal_direct_config": {
                const target = profiles.find((item) => item.client === args.profile?.client);
                if (target && args.profile)
                  Object.assign(target, {
                    active: true,
                    active_mode: "direct",
                    direct_profile: structuredClone(args.profile),
                  });
                return {
                  config_path: profiles[0].config_path,
                  endpoint: "https://api.openai.com/v1",
                  backup_paths: [],
                  model_count: 1,
                };
              }
              case "save_provider": {
                const provider = args.provider as (typeof providers)[number];
                const existing = providers.findIndex((p) => p.id === provider.id);
                const next = { ...provider, models: args.models as (typeof providers)[number]["models"] };
                if (existing >= 0) providers[existing] = next;
                else providers.push(next);
                return next;
              }
              case "delete_provider":
                providers = providers.filter((p) => p.id !== args.id);
                return;
              case "save_standard_model":
                {
                  const model = args.model as (typeof models)[number];
                  models = [...models.filter((m) => m.id !== model.id), model];
                }
                return;
              case "delete_standard_model":
                models = models.filter((m) => m.id !== args.id);
                return;
              case "test_provider":
                return "OK";
              case "export_provider_config":
                transferCalls.push({ command, args });
                return {
                  path: "C:/Users/user/yi-token-backup.yitoken",
                  providers: providers.length,
                  models: providers.reduce((total, item) => total + item.models.length, 0),
                  standard_models: models.length,
                  terminal_config_warnings: [],
                };
              case "import_provider_config":
                transferCalls.push({ command, args });
                return {
                  path: "C:/Users/user/yi-token-backup.yitoken",
                  providers: 2,
                  models: 3,
                  standard_models: 1,
                  terminal_config_warnings:
                    args.filename === "warnings.yitoken" ? ["codex 配置未更新：请在终端接入中更新配置"] : [],
                };
              default: {
                const fixture = await (
                  window as unknown as {
                    __trajectoryFixture: (
                      command: string,
                      args: IpcArgs,
                    ) => Promise<{ handled: boolean; value?: unknown; error?: { code: string; message: string } }>;
                  }
                ).__trajectoryFixture(command, args);
                if (!fixture.handled) throw ipcError("not_found", `Unmocked IPC command: ${command}`);
                if (fixture.error) throw ipcError(fixture.error.code, fixture.error.message);
                return fixture.value;
              }
            }
          },
        },
        configurable: true,
      });
    },
    { theme, empty, failure, options },
  );
}
