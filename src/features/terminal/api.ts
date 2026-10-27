import { invoke } from "@/shared/api";
import type {
  TerminalActiveEngine,
  TerminalApplyResult,
  TerminalClient,
  TerminalDirectProfile,
  TerminalPreview,
  TerminalProfile,
  TerminalStatus,
} from "./types";

declare module "@/shared/api/tauri" {
  interface CommandMap {
    get_terminal_profiles: { args: void; result: TerminalStatus[] };
    preview_terminal_config: { args: { profile: TerminalProfile }; result: TerminalPreview };
    apply_terminal_config: { args: { profile: TerminalProfile }; result: TerminalApplyResult };
    preview_terminal_direct_config: { args: { profile: TerminalDirectProfile }; result: TerminalPreview };
    apply_terminal_direct_config: { args: { profile: TerminalDirectProfile }; result: TerminalApplyResult };
    disconnect_terminal: { args: { client: TerminalClient }; result: TerminalApplyResult };
    set_active_engine: { args: { client: TerminalClient; modelId: string | null }; result: void };
    get_active_engines: { args: void; result: TerminalActiveEngine[] };
    select_terminal_model: {
      args: { client: TerminalClient; providerId: string; model: string };
      result: TerminalActiveEngine;
    };
  }
}

export const terminalApi = {
  profiles: () => invoke("get_terminal_profiles"),
  preview: (profile: TerminalProfile) => invoke("preview_terminal_config", { profile }),
  apply: (profile: TerminalProfile) => invoke("apply_terminal_config", { profile }),
  previewDirect: (profile: TerminalDirectProfile) => invoke("preview_terminal_direct_config", { profile }),
  applyDirect: (profile: TerminalDirectProfile) => invoke("apply_terminal_direct_config", { profile }),
  disconnect: (client: TerminalClient) => invoke("disconnect_terminal", { client }),
  activeEngines: () => invoke("get_active_engines"),
  setActiveEngine: (client: TerminalClient, modelId: string | null) =>
    invoke("set_active_engine", { client, modelId }),
  selectTerminalModel: (client: TerminalClient, providerId: string, model: string) =>
    invoke("select_terminal_model", { client, providerId, model }),
};
