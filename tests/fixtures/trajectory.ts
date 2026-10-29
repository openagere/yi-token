import type {
  BlobView,
  LiveSnapshot,
  SessionChanges,
  SessionDetail,
  SessionListInput,
  SessionPage,
  SessionSummary,
  StepContext,
  ThreadView,
  TrajectoryInput,
  TrajectoryPage,
  TrajectoryRecord,
  TrajectorySettings,
  TrajectoryStatus,
  TrajectoryStep,
  TrajectoryTurn,
} from "../../src/generated";

/** Browser-side fixture for the session trajectory views. Every shape is a generated backend DTO. */

/** Times are relative to the run, so the day groups ("today", "yesterday", "earlier") are stable on any date. */
const NOW = Date.now();
/** The instant the fixture's times are relative to; a visual test freezes the page clock here. */
export const FIXTURE_NOW = NOW;
const DAY = 86_400_000;
const TODAY = (() => {
  const day = new Date(NOW);
  day.setHours(0, 0, 0, 0);
  return day.getTime();
})();
const BASE = NOW - 20 * 60_000;
const LONG_ID = "session-long-codex";
const MAIN_THREAD = "thread-main";
const SUB_THREAD = "thread-sub";

function record(
  partial: Pick<TrajectoryRecord, "id" | "kind" | "step_id" | "text"> & Partial<TrajectoryRecord>,
): TrajectoryRecord {
  return {
    body: null,
    body_blob: null,
    truncated: false,
    thinking: null,
    call_id: null,
    tool_name: null,
    args: null,
    result: null,
    result_body: null,
    result_blob: null,
    result_step_id: null,
    is_error: false,
    started_at: null,
    duration_ms: null,
    duration_estimated: false,
    sub_thread_id: null,
    system_changed: false,
    ...partial,
  };
}

function step(
  partial: Pick<TrajectoryStep, "id" | "number" | "thread_id" | "turn_no" | "records"> &
    Partial<TrajectoryStep>,
): TrajectoryStep {
  const requested = BASE + partial.number * 60_000;
  return {
    requested_at: requested,
    first_token_at: requested + 1_200,
    finished_at: requested + 8_400,
    status: 200,
    stop_reason: "stop",
    error: null,
    model: "gpt-5",
    upstream_model: "gpt-5",
    provider_name: "OpenAI",
    inbound_protocol: "responses",
    upstream_protocol: "responses",
    route_mode: "fixed",
    input_tokens: 12_000,
    output_tokens: 800,
    cached_tokens: 4_000,
    reasoning_tokens: 300,
    capture: "full",
    live_phase: null,
    ...partial,
  };
}

/** Long codex session: 3 turns and 8 steps (7 on the main thread, 1 on a subagent thread), with one system change, one failed step and one in-flight placeholder. */
const longSteps: TrajectoryStep[] = [
  step({
    id: "step-1",
    number: 1,
    thread_id: MAIN_THREAD,
    turn_no: 1,
    records: [
      record({ id: "step-1:req:0", kind: "system", step_id: "step-1", text: "系统提示词 · 2.1 KB" }),
      record({ id: "step-1:req:1", kind: "user", step_id: "step-1", text: "把记录 tab 合并到会话页" }),
      record({
        id: "step-1:resp:0",
        kind: "message",
        step_id: "step-1",
        text: "好的，先看看现有结构。",
        thinking: "需要先读取用量页的记录列表。",
        // The body summary is cut: the full text lives in the blob, which the inspector loads on demand.
        body: "好的，先看看现有结构。\n\n这是被截断的正文摘要。",
        body_blob: "blob-step-1",
        truncated: true,
      }),
      record({
        id: "call:call_read_1",
        kind: "tool",
        step_id: "step-1",
        text: "shell rg usage",
        call_id: "call_read_1",
        tool_name: "shell",
        args: '{"cmd":"rg usage"}',
        result: "34 行",
        result_step_id: "step-2",
        duration_ms: 420,
        duration_estimated: true,
      }),
    ],
  }),
  step({
    id: "step-2",
    number: 2,
    thread_id: MAIN_THREAD,
    turn_no: 1,
    records: [
      record({
        id: "step-2:resp:0",
        kind: "message",
        step_id: "step-2",
        text: "找到 3 处引用，开始整理。",
        // Markdown with raw HTML and a link: the preview must show the script as text, never as an element.
        body: "**加粗** 说明\n\n<script>window.__trajectoryXss = true</script>\n\n[参考链接](https://example.com/usage)",
      }),
    ],
  }),
  step({
    id: "step-3",
    number: 3,
    thread_id: MAIN_THREAD,
    turn_no: 2,
    records: [
      record({ id: "step-3:req:0", kind: "user", step_id: "step-3", text: "顺便检查旧接口是否还在用" }),
      record({ id: "step-3:resp:0", kind: "message", step_id: "step-3", text: "派一个子 Agent 检查。" }),
      record({
        id: "call:call_spawn_1",
        kind: "subtool",
        step_id: "step-3",
        text: "spawn_agent 检查旧接口",
        call_id: "call_spawn_1",
        tool_name: "spawn_agent",
        sub_thread_id: SUB_THREAD,
        result: "子 Agent 已返回",
        duration_estimated: true,
      }),
    ],
  }),
  step({
    id: "step-4",
    number: 4,
    thread_id: SUB_THREAD,
    turn_no: 1,
    records: [
      record({
        id: "step-4:req:0",
        kind: "system",
        step_id: "step-4",
        text: "子 Agent 系统提示词 · 1.4 KB",
        system_changed: true,
      }),
      record({
        id: "step-4:req:1",
        kind: "user",
        step_id: "step-4",
        text: "检查 /api/usage 是否仍读取 records 表",
      }),
      record({
        id: "step-4:resp:0",
        kind: "message",
        step_id: "step-4",
        text: "旧接口仍在读取 records 表。",
      }),
    ],
  }),
  step({
    id: "step-5",
    number: 5,
    thread_id: MAIN_THREAD,
    turn_no: 2,
    status: 502,
    stop_reason: "error",
    error: "upstream timeout",
    finished_at: BASE + 5 * 60_000 + 30_000,
    records: [
      record({ id: "step-5:resp:0", kind: "message", step_id: "step-5", text: "请求上游失败。" }),
      record({
        id: "call:call_fetch_1",
        kind: "tool",
        step_id: "step-5",
        text: "fetch /api/usage",
        call_id: "call_fetch_1",
        tool_name: "fetch",
        result: "502 Bad Gateway",
        is_error: true,
        result_step_id: "step-5",
        duration_ms: 30_000,
        duration_estimated: true,
      }),
    ],
  }),
  step({
    id: "step-6",
    number: 6,
    thread_id: MAIN_THREAD,
    turn_no: 3,
    records: [
      // The prompt changed since step 5 (the fetch tool was added), so this system row carries a diff.
      record({
        id: "step-6:req:0",
        kind: "system",
        step_id: "step-6",
        text: "系统提示词已变更",
        system_changed: true,
      }),
      record({ id: "step-6:req:1", kind: "user", step_id: "step-6", text: "继续，把记录合并进去" }),
      record({ id: "step-6:resp:0", kind: "message", step_id: "step-6", text: "已完成合并。" }),
    ],
  }),
  step({
    id: "step-7",
    number: 7,
    thread_id: MAIN_THREAD,
    turn_no: 3,
    records: [
      record({ id: "step-7:req:0", kind: "user", step_id: "step-7", text: "再核对一下 token 汇总" }),
      record({ id: "step-7:resp:0", kind: "message", step_id: "step-7", text: "汇总与记录一致。" }),
    ],
  }),
  step({
    id: "step-8",
    number: 8,
    thread_id: MAIN_THREAD,
    turn_no: 3,
    status: null,
    stop_reason: "",
    finished_at: null,
    first_token_at: null,
    live_phase: "streaming",
    records: [record({ id: "step-8:resp:0", kind: "message", step_id: "step-8", text: "正在生成…" })],
  }),
];

const longThreads: ThreadView[] = [
  {
    id: MAIN_THREAD,
    kind: "main",
    parent_thread_id: null,
    spawn_call_id: null,
    reset: false,
    step_count: 7,
    started_at: BASE,
  },
  {
    id: SUB_THREAD,
    kind: "subagent",
    parent_thread_id: MAIN_THREAD,
    spawn_call_id: "call_spawn_1",
    reset: false,
    step_count: 1,
    started_at: BASE + 3 * 60_000,
  },
];

const claudeSteps: TrajectoryStep[] = [
  step({
    id: "step-claude-1",
    number: 1,
    thread_id: "thread-claude",
    turn_no: 1,
    model: "claude-sonnet-4",
    upstream_model: "claude-sonnet-4",
    provider_name: "Anthropic",
    inbound_protocol: "anthropic",
    upstream_protocol: "anthropic",
    records: [
      record({ id: "step-claude-1:req:0", kind: "user", step_id: "step-claude-1", text: "修复 SSE 断流" }),
      record({
        id: "step-claude-1:resp:0",
        kind: "message",
        step_id: "step-claude-1",
        text: "已定位到缓冲区未刷新。",
      }),
    ],
  }),
  step({
    id: "step-claude-2",
    number: 2,
    thread_id: "thread-claude",
    turn_no: 1,
    model: "claude-sonnet-4",
    upstream_model: "claude-sonnet-4",
    provider_name: "Anthropic",
    inbound_protocol: "anthropic",
    upstream_protocol: "anthropic",
    records: [
      record({ id: "step-claude-2:resp:0", kind: "message", step_id: "step-claude-2", text: "修复已提交。" }),
    ],
  }),
];

const chainSteps: TrajectoryStep[] = [
  step({
    id: "step-chain-1",
    number: 1,
    thread_id: "thread-chain",
    turn_no: 1,
    model: "deepseek-v3",
    upstream_model: "deepseek-chat",
    provider_name: "DeepSeek",
    inbound_protocol: "openai_chat",
    upstream_protocol: "openai_chat",
    route_mode: "auto",
    records: [
      record({ id: "step-chain-1:req:0", kind: "user", step_id: "step-chain-1", text: "继续上一次的对话" }),
    ],
  }),
];

function summary(
  partial: Pick<SessionSummary, "id" | "client" | "title" | "key_source" | "models" | "step_count"> &
    Partial<SessionSummary>,
): SessionSummary {
  return {
    started_at: BASE,
    last_at: BASE + 10 * 60_000,
    input_tokens: 24_000,
    output_tokens: 1_600,
    bytes: 52_000,
    live: false,
    ...partial,
  };
}

const baseSessions: SessionSummary[] = [
  summary({
    id: LONG_ID,
    client: "codex",
    title: "重构用量页",
    key_source: "client",
    models: ["gpt-5"],
    step_count: 8,
    live: true,
    last_at: NOW - 60_000,
  }),
  summary({
    id: "session-claude-sse",
    client: "claude-code",
    title: "修复 SSE 断流",
    key_source: "new",
    models: ["claude-sonnet-4"],
    step_count: 2,
    started_at: TODAY - 12 * 3_600_000 - 600_000,
    last_at: TODAY - 12 * 3_600_000,
  }),
  summary({
    id: "session-chain",
    client: null,
    title: "全局路由的续接对话",
    key_source: "chain",
    models: ["deepseek-v3"],
    step_count: 1,
    started_at: NOW - 30 * DAY,
    last_at: NOW - 30 * DAY + 60_000,
  }),
];

/** One turn older than the long session's first page (turn 0): three records, so prepending it changes the height. */
const OLDER_LONG_STEPS: TrajectoryStep[] = [
  step({
    id: "step-old-1", number: 0, thread_id: MAIN_THREAD, turn_no: 0,
    records: [
      record({ id: "step-old-1:req:0", kind: "user", step_id: "step-old-1", text: "更早的第一条提问" }),
      record({ id: "step-old-1:resp:0", kind: "message", step_id: "step-old-1", text: "更早的第一条回复。" }),
      record({ id: "step-old-1:resp:1", kind: "message", step_id: "step-old-1", text: "更早的补充说明。" }),
    ],
  }),
];

/** Steps per session. Reset by `resetTrajectoryFixture`; tests may append to the long session. */
let stepsBySession: Record<string, TrajectoryStep[]> = {};
/** Session changes queued for `get_session_changes`, in ascending `seq`. */
let changeLog: { seq: number; sessionId: string }[] = [];
let changeSeq = 0;

/** Mutable per-process state: a test resets it through `resetTrajectoryFixture`. */
interface FixtureState {
  sessions: SessionSummary[];
  settings: TrajectorySettings;
  available: boolean;
}

export interface TrajectoryFixtureOptions {
  /** Whether the one-time privacy notice has been dismissed. Defaults to true (no banner). */
  noticeSeen?: boolean;
  /** Whether the trajectory store is readable. Defaults to true. */
  available?: boolean;
  /** Adds 57 archived codex sessions: 60 in total, so the list needs a second page (50 per page). */
  manySessions?: boolean;
}

let state: FixtureState = {
  sessions: [],
  settings: {
    enabled: true,
    disabled_clients: [],
    disabled_global: false,
    retention_days: 30,
    max_bytes: 1_073_741_824,
    notice_seen: true,
  },
  available: true,
};

/** Restores the fixture to its initial data. Call it after `installBackend` and before navigating. */
export function resetTrajectoryFixture(options: TrajectoryFixtureOptions = {}) {
  const sessions = baseSessions.map((item) => ({ ...item }));
  if (options.manySessions) {
    sessions.push(
      ...Array.from({ length: 57 }, (_, index) => {
        const lastAt = NOW - (40 + index) * DAY;
        return summary({
          id: `session-archive-${String(index + 1).padStart(2, "0")}`,
          client: "codex",
          title: `归档会话 ${index + 1}`,
          key_source: "client",
          models: ["gpt-5"],
          step_count: 1,
          started_at: lastAt - 300_000,
          last_at: lastAt,
        });
      }),
    );
  }
  stepsBySession = {
    [LONG_ID]: [...longSteps],
    "session-claude-sse": claudeSteps,
    "session-chain": chainSteps,
  };
  changeLog = [];
  changeSeq = 0;
  state = {
    sessions,
    settings: {
      enabled: true,
      disabled_clients: [],
      disabled_global: false,
      retention_days: 30,
      max_bytes: 1_073_741_824,
      notice_seen: options.noticeSeen ?? true,
    },
    available: options.available ?? true,
  };
}
resetTrajectoryFixture();

/**
 * Appends `count` main-thread steps (user and reply records) to the last turn of the long session and returns the last step id.
 * With `announce`, the change is queued for the next `get_session_changes` poll; without it the steps are simply present.
 */
export function appendLongSessionSteps(count: number, announce: boolean): string {
  const steps = stepsBySession[LONG_ID] ?? [];
  let lastId = steps.at(-1)?.id ?? "";
  for (let index = 0; index < count; index++) {
    const number = steps.length + 1;
    lastId = `step-${number}`;
    steps.push(
      step({
        id: lastId,
        number,
        thread_id: MAIN_THREAD,
        turn_no: 3,
        records: [
          record({
            id: `step-${number}:req:0`,
            kind: "user",
            step_id: lastId,
            text: `补充的第 ${number} 步`,
          }),
          record({
            id: `step-${number}:resp:0`,
            kind: "message",
            step_id: lastId,
            text: `第 ${number} 步已完成。`,
          }),
        ],
      }),
    );
  }
  stepsBySession[LONG_ID] = steps;
  if (announce) {
    changeSeq += 1;
    changeLog.push({ seq: changeSeq, sessionId: LONG_ID });
  }
  return lastId;
}

/** Session metadata, threads and summary; null for an unknown id. */
export function sessionDetail(id: string): SessionDetail | null {
  const summaryRow = state.sessions.find((item) => item.id === id);
  if (!summaryRow) return null;
  const threads: ThreadView[] =
    id === LONG_ID
      ? longThreads
      : [
          {
            id: stepsBySession[id]?.[0]?.thread_id ?? `thread-${id}`,
            kind: "main",
            parent_thread_id: null,
            spawn_call_id: null,
            reset: false,
            step_count: summaryRow.step_count,
            started_at: summaryRow.started_at,
          },
        ];
  return { summary: summaryRow, threads };
}

/**
 * One page of a thread's turns, ascending. The long session reports `has_older = true` on its first page
 * so the load-older affordance is exercised; the page before it is empty.
 */
export function trajectoryPage(input: TrajectoryInput): TrajectoryPage {
  const steps = stepsBySession[input.session_id] ?? [];
  // The long session's first page holds turns 1 to 3; the page before it holds one older turn (turn 0), so loading
  // earlier prepends real records (see OLDER_LONG_STEPS).
  const olderPage = input.session_id === LONG_ID && input.before_turn !== null && input.before_turn > 0;
  const source = olderPage ? OLDER_LONG_STEPS : steps;
  const threadId = input.thread_id ?? steps[0]?.thread_id ?? "";
  const threadSteps = source.filter((item) => item.thread_id === threadId);
  const turnNos = [...new Set(threadSteps.map((item) => item.turn_no))].sort((a, b) => a - b);
  const turns: TrajectoryTurn[] = turnNos
    .filter((turnNo) => input.before_turn === null || turnNo < input.before_turn)
    .map((turnNo) => ({
      turn_no: turnNo,
      thread_id: threadId,
      steps: threadSteps.filter((item) => item.turn_no === turnNo),
    }));
  return {
    turns,
    has_older: input.session_id === LONG_ID && input.before_turn === null,
    seq: Math.max(0, ...steps.map((item) => item.number)),
  };
}

/** Full context of one step; null for an unknown step id. */
export function stepContext(stepId: string): StepContext | null {
  const found = Object.values(stepsBySession)
    .flat()
    .find((item) => item.id === stepId);
  if (!found) return null;
  return {
    step: found,
    system: "你是 Codex，一个编码助手。",
    // From step 6 on, the fetch tool is part of the definition: the diff against step 5 shows it as added.
    tools:
      found.number >= 6 ? '[{"name":"shell"},{"name":"fetch"}]' : '[{"name":"shell"}]',
    input: [{ kind: "message", role: "user", json: '{"role":"user"}', truncated: false }],
    output: [{ kind: "message", role: "assistant", json: '{"role":"assistant"}', truncated: false }],
    options: { temperature: 0.2 },
    chain_depth: 2,
  };
}

export function blobView(hash: string): BlobView {
  return {
    kind: "body",
    raw_bytes: 4_096,
    truncated: false,
    // The body text itself, as the backend returns it for a body blob. The hash is echoed so tests can tell blobs apart.
    json: `完整正文 · ${hash}`,
  };
}

export function sessionChanges(sinceSeq: number): SessionChanges {
  const sessionIds = [
    ...new Set(changeLog.filter((change) => change.seq > sinceSeq).map((change) => change.sessionId)),
  ];
  return { session_ids: sessionIds, seq: changeSeq };
}

export function trajectoryStatusView(): TrajectoryStatus {
  return {
    available: state.available,
    error: state.available ? null : "轨迹库无法打开：磁盘已满",
    bytes: 1_240_000,
    sessions: state.sessions.length,
    settings: { ...state.settings, disabled_clients: [...state.settings.disabled_clients] },
  };
}

export function saveTrajectorySettings(settings: TrajectorySettings) {
  state.settings = { ...settings };
}

/** Removes every session of one client (or all sessions for null) and returns how many were removed. */
export function clearSessions(client: string | null): number {
  const before = state.sessions.length;
  state.sessions = state.sessions.filter((item) => client !== null && item.client !== client);
  return before - state.sessions.length;
}

/** Removes a session from the fixture; unknown ids are ignored, as the backend does. */
export function deleteSession(id: string) {
  state.sessions = state.sessions.filter((item) => item.id !== id);
}

export const liveSnapshot: LiveSnapshot = { in_flight: [], completed: [], seq: 0 };

/**
 * One page of sessions, newest first, filtered like the backend: a null client means no client filter,
 * the title match is a case-insensitive substring, and the cursor is an offset in this fixture.
 */
export function listSessions(input: Partial<SessionListInput>): SessionPage {
  const query = input.query?.trim().toLowerCase() ?? "";
  const matched = state.sessions
    .filter((item) => input.client == null || item.client === input.client)
    .filter((item) => !query || item.title.toLowerCase().includes(query))
    .filter((item) => !input.model || item.models.includes(input.model))
    .sort((a, b) => b.last_at - a.last_at);
  const offset = input.cursor ? Number(input.cursor) : 0;
  const limit = input.limit ?? 50;
  return {
    items: matched.slice(offset, offset + limit),
    next_cursor: offset + limit < matched.length ? String(offset + limit) : null,
  };
}

/** Arguments of the trajectory IPC commands, as the page sends them (keys are camelCase in Tauri). */
export interface TrajectoryCommandArgs {
  id?: string;
  client?: string | null;
  stepId?: string;
  hash?: string;
  sinceSeq?: number;
  input?: Partial<SessionListInput> & Partial<TrajectoryInput>;
  settings?: TrajectorySettings;
}

/** Result of forwarding a command to the fixture: `handled` is false for commands this fixture does not serve. */
export interface TrajectoryReply {
  handled: boolean;
  value?: unknown;
  error?: { code: string; message: string };
}

function reply(value: unknown): TrajectoryReply {
  return { handled: true, value };
}

function found<T>(value: T | null, message: string): TrajectoryReply {
  return value === null ? { handled: true, error: { code: "not_found", message } } : reply(value);
}

/** Serves the session and trajectory commands from this fixture. Runs in Node and is exposed to the page. */
export function trajectoryCommand(command: string, args: TrajectoryCommandArgs): TrajectoryReply {
  switch (command) {
    case "list_sessions":
      if (!state.available)
        return { handled: true, error: { code: "internal", message: "轨迹库无法打开：磁盘已满" } };
      return reply(listSessions(args.input ?? {}));
    case "get_session":
      return found(sessionDetail(args.id ?? ""), `Unknown session: ${args.id}`);
    case "get_trajectory":
      return reply(trajectoryPage(args.input as TrajectoryInput));
    case "get_step_context":
      return found(stepContext(args.stepId ?? ""), `Unknown step: ${args.stepId}`);
    case "get_blob":
      return reply(blobView(args.hash ?? ""));
    case "get_session_changes":
      return reply(sessionChanges(args.sinceSeq ?? 0));
    case "delete_session":
      deleteSession(args.id ?? "");
      return reply(undefined);
    case "rebuild_trajectory_db":
      state.available = true;
      return reply(undefined);
    case "clear_sessions":
      return reply(clearSessions(args.client ?? null));
    case "get_trajectory_status":
      return reply(trajectoryStatusView());
    case "save_trajectory_settings":
      if (args.settings) saveTrajectorySettings(args.settings);
      return reply(undefined);
    case "get_live_requests":
      return reply(liveSnapshot);
    default:
      return { handled: false };
  }
}
