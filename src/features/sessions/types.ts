import type { SessionListInput } from "@/generated";

export type {
  BlobView,
  ContextItem,
  KeySource,
  LivePhase,
  LiveRequest,
  LiveSnapshot,
  RecordKind,
  SessionChanges,
  SessionDetail,
  SessionListInput,
  SessionPage,
  SessionSummary,
  StepContext,
  ThreadKindView,
  ThreadView,
  TrajectoryInput,
  TrajectoryPage,
  TrajectoryRecord,
  TrajectorySettings,
  TrajectoryStatus,
  TrajectoryStep,
  TrajectoryTurn,
} from "@/generated";

/** 会话列表的筛选条件；游标与页大小由 hook 分页时注入。 */
export type SessionFilter = Omit<SessionListInput, "cursor" | "limit">;
