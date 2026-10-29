import type { ModelMapping, ProviderInput, ProviderView as GeneratedProviderView, Protocol, Thinking } from "@/generated";

export type { ModelMapping };
export type ProviderView = GeneratedProviderView;
/** 前端的 `Provider` 即后端 `save_provider` 的入参 `ProviderInput`（生成类型 `Provider` 是存储层模型，不对外暴露）。 */
export type { ProviderInput as Provider };
export type ProviderType = Protocol;
export type ThinkingLevel = Thinking;

/** 加密配置导入或导出的结果。 */
export interface ProviderConfigTransferResult {
  path: string;
  providers: number;
  models: number;
  /** 导出：写入文件的标准模型数；导入：备份中包含的标准模型数。 */
  standard_models: number;
  /** 导入时被跳过的条目，需要告诉用户。 */
  terminal_config_warnings: string[];
}
