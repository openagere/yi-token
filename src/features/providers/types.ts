import type { ModelMapping, ProviderInput, ProviderView as GeneratedProviderView, Protocol, ProtocolSupport, Thinking } from "@/generated";

export type { ModelMapping, ProtocolSupport };
/** 草稿（新建）没有后端告警，故 `terminal_config_warnings` 对前端可选。 */
export type ProviderView = Omit<GeneratedProviderView, "terminal_config_warnings"> & { terminal_config_warnings?: string[] };
/** 前端的 `Provider` 即后端 `save_provider` 的入参 `ProviderInput`（生成类型 `Provider` 是存储层模型，不对外暴露）。 */
export type { ProviderInput as Provider };
export type ProviderType = Protocol;
export type ThinkingLevel = Thinking;

/** 新的 Provider 对全部客户端协议开放，由 yi-token 负责转换。 */
export const ALL_PROTOCOLS: ProtocolSupport = { anthropic: true, openai_chat: true, responses: true };

export interface ProviderSaveResult { terminal_config_warnings: string[] }
/** 加密配置导入或导出的结果。 */
export interface ProviderConfigTransferResult {
  path: string;
  providers: number;
  models: number;
  /** 导出：写入文件的标准模型数；导入：备份中包含的标准模型数。 */
  standard_models: number;
  terminal_config_warnings: string[];
}
