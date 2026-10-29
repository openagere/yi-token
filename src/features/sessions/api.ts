import { invoke } from "@/shared/api";
import type {
  BlobView,
  Client,
  LiveSnapshot,
  SessionChanges,
  SessionDetail,
  SessionListInput,
  SessionPage,
  StepContext,
  TrajectoryInput,
  TrajectoryPage,
  TrajectorySettings,
  TrajectoryStatus,
} from "@/generated";

declare module "@/shared/api/tauri" {
  interface CommandMap {
    list_sessions: { args: { input: SessionListInput }; result: SessionPage };
    get_session: { args: { id: string }; result: SessionDetail };
    get_trajectory: { args: { input: TrajectoryInput }; result: TrajectoryPage };
    get_step_context: { args: { stepId: string }; result: StepContext };
    get_blob: { args: { hash: string }; result: BlobView };
    get_session_changes: { args: { sinceSeq: number }; result: SessionChanges };
    delete_session: { args: { id: string }; result: void };
    clear_sessions: { args: { client: Client | null }; result: number };
    get_trajectory_status: { args: void; result: TrajectoryStatus };
    save_trajectory_settings: { args: { settings: TrajectorySettings }; result: void };
    rebuild_trajectory_db: { args: void; result: void };
    get_live_requests: { args: { sinceSeq: number }; result: LiveSnapshot };
  }
}

export const sessionsApi = {
  list: (input: SessionListInput) => invoke("list_sessions", { input }),
  get: (id: string) => invoke("get_session", { id }),
  trajectory: (input: TrajectoryInput) => invoke("get_trajectory", { input }),
  stepContext: (stepId: string) => invoke("get_step_context", { stepId }),
  blob: (hash: string) => invoke("get_blob", { hash }),
  changes: (sinceSeq: number) => invoke("get_session_changes", { sinceSeq }),
  remove: (id: string) => invoke("delete_session", { id }),
  clear: (client: Client | null) => invoke("clear_sessions", { client }),
  status: () => invoke("get_trajectory_status"),
  saveSettings: (settings: TrajectorySettings) => invoke("save_trajectory_settings", { settings }),
  rebuild: () => invoke("rebuild_trajectory_db"),
  live: (sinceSeq: number) => invoke("get_live_requests", { sinceSeq }),
};
