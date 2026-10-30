import type { ActiveEngineView, Client, DirectProfile, RouteMode } from "@/generated";
import type { ProviderType } from "@/shared/lib";

export type TerminalRouteMode = RouteMode;
export type TerminalActiveEngine = ActiveEngineView;

export type TerminalClient = Client;
export interface TerminalSelection {
  provider_id: string;
  model: string;
}
export type TerminalDirectProfile = DirectProfile;
export interface TerminalProfile {
  client: TerminalClient;
  models: TerminalSelection[];
  default_model: string;
  protocol: ProviderType | null;
  route_mode: TerminalRouteMode;
  active_engine: TerminalActiveEngine | null;
}
export interface TerminalStatus {
  client: TerminalClient;
  config_path: string;
  exists: boolean;
  active: boolean;
  active_mode?: "none" | "proxy" | "direct";
  profile: TerminalProfile | null;
  direct_profile?: TerminalDirectProfile | null;
  config_error: string | null;
}
export interface TerminalPreviewFile {
  path: string;
  content: string;
}
export interface TerminalPreview {
  config_path: string;
  endpoint: string;
  files: TerminalPreviewFile[];
}
export interface TerminalApplyResult {
  config_path: string;
  endpoint: string;
  backup_paths: string[];
  model_count: number;
}
