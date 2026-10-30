import type { ProviderType, ProviderView } from "@/features/providers";
import type {
  TerminalActiveEngine,
  TerminalClient,
  TerminalDirectProfile,
  TerminalProfile,
  TerminalRouteMode,
  TerminalSelection,
} from "./types";

export interface TerminalDefinition {
  id: TerminalClient;
  name: string;
  /** 客户端可访问本地代理的协议；列表顺序把首个协议定为默认（主）协议。 */
  protocols: readonly ProviderType[];
  brand: string;
}

export const TERMINALS = [
  { id: "codex", name: "Codex CLI", protocols: ["responses"], brand: "openai" },
  { id: "claude-code", name: "Claude Code", protocols: ["anthropic"], brand: "claude" },
  {
    id: "opencode",
    name: "OpenCode",
    protocols: ["openai_chat", "responses", "anthropic"],
    brand: "opencode",
  },
  { id: "pi", name: "Pi", protocols: ["openai_chat", "responses", "anthropic"], brand: "pi" },
  {
    id: "deepseek-harness",
    name: "DeepSeek Harness",
    protocols: ["openai_chat", "responses", "anthropic"],
    brand: "deepseek",
  },
] as const satisfies readonly TerminalDefinition[];

/** 未选择或选择非法时回落到首个（主）协议。 */
export function protocolFor(terminal: TerminalDefinition, protocol: string | null | undefined): ProviderType {
  const protocols = terminal.protocols as readonly ProviderType[];
  return protocols.includes(protocol as ProviderType) ? (protocol as ProviderType) : protocols[0];
}

/** 草稿中记录的协议：多协议终端记录具体协议，单协议终端保持为空。 */
export function draftProtocol(
  terminal: TerminalDefinition,
  protocol: string | null | undefined,
): ProviderType | null {
  return terminal.protocols.length > 1 ? protocolFor(terminal, protocol) : null;
}

export const terminalName = (client: TerminalClient): string =>
  TERMINALS.find((item) => item.id === client)?.name ?? client;

export const AUTO_MODEL = "yi-token";

const emptyProfile = (client: TerminalClient, protocol: TerminalProfile["protocol"]): TerminalProfile => ({
  client,
  models: [],
  default_model: AUTO_MODEL,
  protocol,
  route_mode: "fixed",
  active_engine: null,
});

export const EMPTY_PROFILES: Record<TerminalClient, TerminalProfile> = {
  codex: emptyProfile("codex", null),
  "claude-code": emptyProfile("claude-code", null),
  opencode: emptyProfile("opencode", "openai_chat"),
  pi: emptyProfile("pi", "openai_chat"),
  "deepseek-harness": emptyProfile("deepseek-harness", "openai_chat"),
};

/** 保留已启用、协议可用且模型能被该终端使用的 Provider。 */
const DIRECT_PROTOCOLS: Record<TerminalClient, readonly ProviderType[]> = {
  codex: ["responses"],
  "claude-code": ["anthropic"],
  opencode: ["openai_chat", "responses", "anthropic"],
  pi: ["anthropic", "openai_chat", "responses"],
  "deepseek-harness": ["anthropic", "openai_chat", "responses"],
};

/** 直连时模型选择器由 yi-token 接管的终端：始终同步 Provider 维护的上游模型。 */
const PROVIDER_MODEL_SOURCE_CLIENTS: readonly TerminalClient[] = ["opencode", "pi", "deepseek-harness"];

export const usesProviderModels = (client: TerminalClient) => PROVIDER_MODEL_SOURCE_CLIENTS.includes(client);

/** Direct access must match the upstream native protocol, not a translated client protocol. */
export function directCompatibleProviders(
  providers: readonly ProviderView[],
  client: TerminalClient,
): ProviderView[] {
  return providers
    .filter((provider) => provider.enabled && DIRECT_PROTOCOLS[client].includes(provider.provider_type))
    .map((provider) => ({
      ...provider,
      models: provider.models.filter((model) => !model.protocol || model.protocol === provider.provider_type),
    }));
}

export function emptyDirectProfile(client: TerminalClient, providerId = ""): TerminalDirectProfile {
  return {
    client,
    provider_id: providerId,
    model_source: usesProviderModels(client) ? "provider" : "native",
    model: null,
  };
}

/** 生成新草稿；默认模型不在集合中时回落到第一个模型，无已选模型时回落到 Auto。 */
export function buildProfile(
  client: TerminalClient,
  models: TerminalSelection[],
  defaultModel: string,
  protocol: ProviderType | null,
  routeMode: TerminalRouteMode = "fixed",
  activeEngine: TerminalActiveEngine | null = null,
): TerminalProfile {
  return {
    client,
    models,
    default_model:
      defaultModel === AUTO_MODEL || models.some((model) => model.model === defaultModel)
        ? defaultModel
        : (models[0]?.model ?? AUTO_MODEL),
    protocol,
    route_mode: routeMode,
    active_engine: activeEngine,
  };
}
