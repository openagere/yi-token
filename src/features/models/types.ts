import type { CatalogInfo, Capabilities, Modality, StandardModelView, Support } from "@/generated";

export type { Modality };
export type EffortSupport = Support;
export type ModelCapabilities = Capabilities;
/** 前端的 `StandardModel` 带 `provider_count`，即后端 `StandardModelView`。 */
export type StandardModel = StandardModelView;

/** 目录信息：`path` 无来源文件（内置目录）时为空串，由 api.ts 适配。`version` 保持后端的字符串。 */
export type ModelCatalogInfo = Omit<CatalogInfo, "path"> & { path: string };
