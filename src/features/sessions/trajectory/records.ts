import type { RecordKind, TrajectoryRecord, TrajectoryStep, TrajectoryTurn } from "@/generated";
import type { MessageKey, Translate } from "@/shared/i18n";

/** One record of the ledger, with the step and turn it belongs to. Positions are recomputed by every projection. */
export interface LedgerRecord {
  record: TrajectoryRecord;
  step: TrajectoryStep;
  turn: number;
  threadId: string;
  /** 1-based position across the loaded records. */
  index: number;
  /** First record of its step: shows the request boundary marker. */
  stepStart: boolean;
  turnStart: boolean;
  turnEnd: boolean;
  collapsedSummary?: string;
  collapsedSummaryKind?: "turn" | "assistant";
}

export type RecordState = "running" | "error" | "complete";

/** Message keys of the record kinds, shared by the ledger tag and the inspector title. */
export const KIND_KEY: Record<RecordKind, MessageKey> = {
  system: "session.ledger.kind.system",
  user: "session.ledger.kind.user",
  message: "session.ledger.kind.message",
  tool: "session.ledger.kind.tool",
  subtool: "session.ledger.kind.subtool",
  compacted: "session.ledger.kind.compacted",
};

const ERROR_STOP_REASONS: ReadonlySet<string> = new Set(["error", "cancelled"]);
const IN_FLIGHT_PHASES: ReadonlySet<string> = new Set(["waiting", "streaming"]);

export function isCall(record: TrajectoryRecord): boolean {
  return record.kind === "tool" || record.kind === "subtool";
}

/** A request is in flight while the live registry reports it waiting or streaming. */
export function isInFlight(step: TrajectoryStep): boolean {
  return step.live_phase !== null && IN_FLIGHT_PHASES.has(step.live_phase);
}

/** Recomputes the step and turn boundaries of a projected list, so filtered lists stay well formed. */
function relabel(records: readonly LedgerRecord[]): LedgerRecord[] {
  const started = new Set<number>();
  return records.map((item, position) => {
    const previous = records[position - 1];
    const next = records[position + 1];
    const turnStart = !started.has(item.turn) && item.record.kind !== "system";
    if (turnStart) started.add(item.turn);
    return {
      ...item,
      stepStart: previous === undefined || previous.step.id !== item.step.id,
      turnStart,
      turnEnd: next === undefined || next.turn !== item.turn,
    };
  });
}

/** Flattens the turns of one or more pages (in ledger order) into numbered ledger records. */
export function flattenTurns(turns: readonly TrajectoryTurn[]): LedgerRecord[] {
  const records: LedgerRecord[] = [];
  for (const turn of turns) {
    for (const step of turn.steps) {
      for (const record of step.records) {
        records.push({
          record,
          step,
          turn: turn.turn_no,
          threadId: turn.thread_id,
          index: records.length + 1,
          stepStart: false,
          turnStart: false,
          turnEnd: false,
        });
      }
    }
  }
  return relabel(records);
}

/** The step before `stepId` in the same thread, among the loaded records; null when none is loaded. */
export function previousStepId(records: readonly LedgerRecord[], stepId: string): string | null {
  const steps: TrajectoryStep[] = [];
  const seen = new Set<string>();
  for (const item of records) {
    if (seen.has(item.step.id)) continue;
    seen.add(item.step.id);
    steps.push(item.step);
  }
  const index = steps.findIndex((step) => step.id === stepId);
  if (index < 0) return null;
  const thread = steps[index]?.thread_id;
  for (let position = index - 1; position >= 0; position--) {
    if (steps[position]?.thread_id === thread) return steps[position]?.id ?? null;
  }
  return null;
}

/** Records of a turn that count as content: everything except the system prompt. */
function contentOf(records: readonly LedgerRecord[]): LedgerRecord[] {
  return records.filter((item) => item.record.kind !== "system");
}

function groupByTurn(records: readonly LedgerRecord[]): Map<number, LedgerRecord[]> {
  const byTurn = new Map<number, LedgerRecord[]>();
  for (const item of records) {
    const group = byTurn.get(item.turn) ?? [];
    group.push(item);
    byTurn.set(item.turn, group);
  }
  return byTurn;
}

/** Turns with more than one content record: the ones a fold can hide anything in. */
export function collapsibleTurns(records: readonly LedgerRecord[]): Set<number> {
  const turns = new Set<number>();
  for (const [turn, group] of groupByTurn(records)) {
    if (contentOf(group).length > 1) turns.add(turn);
  }
  return turns;
}

/** Ids of assistant records that are followed by at least one tool or subtool call. */
export function collapsibleCalls(records: readonly LedgerRecord[]): Set<string> {
  const ids = new Set<string>();
  for (const [position, item] of records.entries()) {
    const next = records[position + 1];
    if (item.record.kind === "message" && next !== undefined && isCall(next.record)) ids.add(item.record.id);
  }
  return ids;
}

function turnSummary(hidden: readonly LedgerRecord[], t: Translate): string {
  const steps = new Set(hidden.map((item) => item.step.id)).size;
  const calls = hidden.filter((item) => isCall(item.record)).length;
  return t("session.ledger.turnSummary", { steps, calls });
}

function callsSummary(calls: readonly LedgerRecord[], t: Translate): string {
  const summary = t("session.ledger.callSummary", { count: calls.length });
  const names = [...new Set(calls.map((item) => item.record.tool_name ?? "").filter((name) => name !== ""))];
  return names.length > 0 ? `${summary} · ${names.join(", ")}` : summary;
}

/**
 * Folds the turns in `collapsed`: the first content record stays, and one summary row stands in for the rest.
 * A turn with one content record cannot fold, so it is left as it is.
 */
export function collapseTurns(
  records: readonly LedgerRecord[],
  collapsed: ReadonlySet<number>,
  t: Translate,
): LedgerRecord[] {
  const byTurn = groupByTurn(records);
  return records.flatMap((item) => {
    if (!collapsed.has(item.turn) || item.record.kind === "system") return [item];
    const content = contentOf(byTurn.get(item.turn) ?? [item]);
    if (content.length <= 1) return [item];
    if (item.index !== content[0]?.index) return [];
    const hidden = content.slice(1);
    return [
      { ...item, turnEnd: false },
      {
        ...item,
        stepStart: false,
        turnStart: false,
        turnEnd: true,
        collapsedSummary: turnSummary(hidden, t),
        collapsedSummaryKind: "turn",
      },
    ];
  });
}

/** Folds the tool and subtool records that follow each assistant record in `collapsed` into one summary row. */
export function collapseAssistants(
  records: readonly LedgerRecord[],
  collapsed: ReadonlySet<string>,
  t: Translate,
): LedgerRecord[] {
  const out: LedgerRecord[] = [];
  for (let position = 0; position < records.length; position++) {
    const item = records[position];
    if (item === undefined) continue;
    out.push(item);
    if (item.record.kind !== "message" || !collapsed.has(item.record.id)) continue;
    const calls: LedgerRecord[] = [];
    for (let next = position + 1; next < records.length; next++) {
      const candidate = records[next];
      if (candidate === undefined || candidate.collapsedSummary !== undefined || !isCall(candidate.record))
        break;
      calls.push(candidate);
    }
    const last = calls.at(-1);
    if (last === undefined) continue;
    out[out.length - 1] = { ...item, turnEnd: false };
    out.push({
      ...item,
      stepStart: false,
      turnStart: false,
      turnEnd: last.turnEnd,
      collapsedSummary: callsSummary(calls, t),
      collapsedSummaryKind: "assistant",
    });
    position += calls.length;
  }
  return out;
}

/** Case-insensitive substring match over the summaries a record carries. An empty query keeps everything. */
export function filterRecords(records: readonly LedgerRecord[], query: string): LedgerRecord[] {
  const needle = query.trim().toLowerCase();
  if (needle === "") return [...records];
  const matches = (item: LedgerRecord) =>
    [
      item.record.text,
      item.record.body,
      item.record.thinking,
      item.record.tool_name,
      item.record.args,
      item.record.result,
    ].some((field) => field !== null && field.toLowerCase().includes(needle));
  return relabel(records.filter(matches));
}

/** running while the step is in flight (a finished call is not), error on a failed record or stop reason, otherwise complete. */
export function stateOf(item: LedgerRecord): RecordState {
  if (item.record.is_error || ERROR_STOP_REASONS.has(item.step.stop_reason)) return "error";
  if (isInFlight(item.step) && !(isCall(item.record) && item.record.result !== null)) return "running";
  return "complete";
}
