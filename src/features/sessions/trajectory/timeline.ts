import type { RecordKind, TrajectoryStep } from "@/generated";
import { isCall, type LedgerRecord } from "./records";

/** Horizontal projection of the timeline: equal or recorded widths, with or without the idle gaps. */
export type TimelineMode = "sequence" | "duration" | "time" | "actual";

export interface TimeRange {
  start: number;
  end: number;
}

export interface TimelineSpan extends TimeRange {
  /** LedgerRecord.index of the record this span draws. */
  index: number;
  isError: boolean;
  kind: RecordKind;
  label: string;
  lane: 0 | 1 | 2;
  /** Assistant spans only: the boundary between the time to first token and generation. */
  ttftEnd?: number;
}

export interface TimelineModel extends TimeRange {
  spans: TimelineSpan[];
  turnBoundaries: { turn: number; time: number }[];
}

/** Recorded timing of one ledger record, in epoch milliseconds. Tool spans are estimates. */
export interface RecordTiming extends TimeRange {
  ttftMs?: number;
  generationMs?: number;
  estimated: boolean;
}

/** System and user records sit in lane 0, the model in lane 1, tool calls in lane 2. */
export function laneFor(kind: RecordKind): 0 | 1 | 2 {
  if (kind === "tool" || kind === "subtool") return 2;
  if (kind === "message" || kind === "compacted") return 1;
  return 0;
}

/**
 * Timing of every record that has any. A request spans its requested and finished times. A tool call
 * starts when its request finishes and is estimated to end when the next request is sent, because the
 * proxy only sees the gap. Records without timing (no finish yet, or no following request) are absent.
 */
export function recordTimings(records: readonly LedgerRecord[]): ReadonlyMap<number, RecordTiming> {
  const steps: TrajectoryStep[] = [];
  const seen = new Set<string>();
  for (const item of records) {
    if (seen.has(item.step.id)) continue;
    seen.add(item.step.id);
    steps.push(item.step);
  }
  const nextStep = new Map<string, TrajectoryStep>();
  steps.forEach((step, position) => {
    const next = steps[position + 1];
    if (next !== undefined) nextStep.set(step.id, next);
  });

  const timings = new Map<number, RecordTiming>();
  for (const item of records) {
    const timing = timingOf(item, nextStep.get(item.step.id));
    if (timing !== null) timings.set(item.index, timing);
  }
  return timings;
}

function timingOf(item: LedgerRecord, next: TrajectoryStep | undefined): RecordTiming | null {
  const { record, step } = item;
  if (record.kind === "message" || record.kind === "compacted") {
    if (step.finished_at === null) return null;
    const start = step.requested_at;
    const end = Math.max(start, step.finished_at);
    const first = step.first_token_at;
    if (record.kind === "message" && first !== null && first >= start && end >= first) {
      return { start, end, ttftMs: first - start, generationMs: end - first, estimated: false };
    }
    return { start, end, estimated: false };
  }
  if (isCall(record)) {
    if (step.finished_at === null) return null;
    const start = step.finished_at;
    if (next !== undefined) return { start, end: Math.max(start, next.requested_at), estimated: true };
    if (record.duration_ms !== null)
      return { start, end: start + Math.max(0, record.duration_ms), estimated: true };
  }
  return null;
}

/** Fraction of a bar spent before the first token, or null when the request has no first-token time. */
function ttftFraction(timing: RecordTiming): number | null {
  if (timing.ttftMs === undefined || timing.generationMs === undefined) return null;
  const total = timing.ttftMs + timing.generationMs;
  return total > 0 ? timing.ttftMs / total : null;
}

/**
 * Projects the ledger onto the timeline. Sequence mode gives every record one equal slot. The other modes
 * place records at their recorded times. `duration` keeps the idle gaps between operations; `time` and `actual`
 * cut them (spec 8.3.1). `time` also gives every record the same width.
 * @returns The model, or null when there is nothing to draw.
 */
export function deriveTimeline(records: readonly LedgerRecord[], mode: TimelineMode): TimelineModel | null {
  if (records.length === 0) return null;
  const timings = recordTimings(records);
  return mode === "sequence" ? sequenceModel(records, timings) : timedModel(records, timings, mode);
}

function sequenceModel(
  records: readonly LedgerRecord[],
  timings: ReadonlyMap<number, RecordTiming>,
): TimelineModel {
  const spans: TimelineSpan[] = [];
  const turnBoundaries: TimelineModel["turnBoundaries"] = [];
  records.forEach((item, position) => {
    const previous = records[position - 1];
    if (previous === undefined || previous.turn !== item.turn) {
      turnBoundaries.push({ turn: item.turn, time: position });
    }
    const span: TimelineSpan = {
      start: position,
      end: position + 1,
      index: item.index,
      isError: item.record.is_error,
      kind: item.record.kind,
      label: item.record.text,
      lane: laneFor(item.record.kind),
    };
    const timing = timings.get(item.index);
    const fraction = timing === undefined ? null : ttftFraction(timing);
    spans.push(fraction === null ? span : { ...span, ttftEnd: position + fraction });
  });
  return { start: 0, end: records.length, spans, turnBoundaries };
}

function timedModel(
  records: readonly LedgerRecord[],
  timings: ReadonlyMap<number, RecordTiming>,
  mode: Exclude<TimelineMode, "sequence">,
): TimelineModel | null {
  const timed = records.flatMap((item) => {
    const timing = timings.get(item.index);
    return timing === undefined ? [] : [{ item, timing }];
  });
  if (timed.length === 0) return null;

  // Spec 8.3.1: the time and actual projections cut idle gaps; duration keeps them. Each span is shifted by
  // the gaps removed before it.
  const compressIdle = mode === "time" || mode === "actual";
  const offsets = new Map<number, number>();
  const byStart = [...timed].sort(
    (left, right) => left.timing.start - right.timing.start || left.timing.end - right.timing.end,
  );
  let removed = 0;
  let coveredUntil: number | null = null;
  for (const { item, timing } of byStart) {
    if (compressIdle && coveredUntil !== null && timing.start > coveredUntil) {
      removed += timing.start - coveredUntil;
    }
    offsets.set(item.index, removed);
    coveredUntil = coveredUntil === null ? timing.end : Math.max(coveredUntil, timing.end);
  }

  // The time mode gives every bar the same width: the average spacing between the bars' compressed starts.
  const equalWidth = mode === "time";
  const starts = timed.map(({ item, timing }) => timing.start - (offsets.get(item.index) ?? 0));
  const unit = Math.max(1, (Math.max(...starts) - Math.min(...starts)) / timed.length);

  const spans: TimelineSpan[] = [];
  const turnBoundaries: TimelineModel["turnBoundaries"] = [];
  for (const { item, timing } of timed) {
    const offset = offsets.get(item.index) ?? 0;
    const start = timing.start - offset;
    const end = equalWidth ? start + unit : timing.end - offset;
    const span: TimelineSpan = {
      start,
      end,
      index: item.index,
      isError: item.record.is_error,
      kind: item.record.kind,
      label: item.record.text,
      lane: laneFor(item.record.kind),
    };
    const fraction = ttftFraction(timing);
    if (fraction !== null) {
      span.ttftEnd = equalWidth ? start + fraction * unit : Math.min(end, start + (timing.ttftMs ?? 0));
    }
    spans.push(span);
    const boundary = turnBoundaries.find((candidate) => candidate.turn === item.turn);
    if (boundary === undefined) turnBoundaries.push({ turn: item.turn, time: start });
    else boundary.time = Math.min(boundary.time, start);
  }

  return {
    start: Math.min(...spans.map((span) => span.start)),
    end: Math.max(...spans.map((span) => span.end)),
    spans,
    turnBoundaries,
  };
}

/**
 * Index of every ledger record whose span overlaps the selected range, in the given projection.
 * Records outside the range are the ones the ledger dims.
 */
export function focusIndexes(
  records: readonly LedgerRecord[],
  range: TimeRange,
  mode: TimelineMode,
): ReadonlySet<number> {
  const model = deriveTimeline(records, mode);
  return new Set(
    model?.spans
      .filter((span) => span.start <= range.end && span.end >= range.start)
      .map((span) => span.index) ?? [],
  );
}
