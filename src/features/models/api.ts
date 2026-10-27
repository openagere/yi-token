import { invoke } from "@/shared/api";
import type { CatalogInfo } from "@/generated";
import type { ModelCatalogInfo, StandardModel } from "./types";

declare module "@/shared/api/tauri" {
  interface CommandMap {
    list_standard_models: { args: void; result: StandardModel[] };
    get_model_catalog_info: { args: void; result: CatalogInfo };
    save_standard_model: { args: { model: StandardModel }; result: StandardModel };
    delete_standard_model: { args: { id: string }; result: void };
  }
}

/** 后端 `path` 可为 null（内置目录），前端按空串展示。 */
export function toModelCatalogInfo(info: CatalogInfo): ModelCatalogInfo {
  return { ...info, path: info.path ?? "" };
}

export const modelApi = {
  list: () => invoke("list_standard_models"),
  catalogInfo: async () => toModelCatalogInfo(await invoke("get_model_catalog_info")),
  save: async (model: StandardModel): Promise<void> => {
    await invoke("save_standard_model", { model });
  },
  remove: (id: string) => invoke("delete_standard_model", { id }),
};
