/** Pure projection from ledger records to measurable virtual rows (ported from the reference ledger). */

const CONTENT_ROW_HEIGHT = 30;
const COLLAPSED_SUMMARY_HEIGHT = 20;
const TERMINAL_BOUNDARY_HEIGHT = 9;

/** Minimal shape the projection needs. Records that carry no ledger row of their own set `requestOnly`. */
export interface VirtualizableRecord {
  readonly record: { readonly id: string };
  readonly requestOnly?: boolean;
  readonly collapsedSummaryKind?: "turn" | "assistant";
}

export interface TrajectoryVirtualRowEntry<T extends VirtualizableRecord> {
  logicalIndex: number;
  record: T;
}

/** One virtualizer item. Request-only records ride along with the next content row, so no item has zero height. */
export interface TrajectoryVirtualRow<T extends VirtualizableRecord> {
  entries: readonly TrajectoryVirtualRowEntry<T>[];
  height: number;
  key: string;
}

/**
 * DOM-safe row identity shared by React, the virtualizer and the browser tests. The record id is encoded, so the
 * `|summary:` suffix of a folded summary row cannot collide with it.
 */
export function trajectoryVirtualRecordKey(record: VirtualizableRecord): string {
  const identity = encodeURIComponent(record.record.id);
  return record.collapsedSummaryKind === undefined
    ? identity
    : `${identity}|summary:${record.collapsedSummaryKind}`;
}

/** Row height in pixels for a record: a content row, or a folded summary row. */
export function trajectoryVirtualRowHeight(record: VirtualizableRecord): number {
  return record.collapsedSummaryKind === undefined ? CONTENT_ROW_HEIGHT : COLLAPSED_SUMMARY_HEIGHT;
}

/**
 * Attaches request-only records to the next content row, so the virtualizer never owns a zero-height item.
 * A trailing request-only run becomes a standalone terminal boundary row.
 */
export function groupTrajectoryVirtualRows<T extends VirtualizableRecord>(
  records: readonly T[],
): readonly TrajectoryVirtualRow<T>[] {
  const rows: TrajectoryVirtualRow<T>[] = [];
  let pending: TrajectoryVirtualRowEntry<T>[] = [];

  for (const [logicalIndex, record] of records.entries()) {
    const entry = { logicalIndex, record };
    if (record.requestOnly === true) {
      pending.push(entry);
      continue;
    }
    const entries = [...pending, entry];
    pending = [];
    rows.push({
      entries,
      height: trajectoryVirtualRowHeight(record),
      key: trajectoryVirtualRecordKey(record),
    });
  }

  if (pending.length > 0) {
    rows.push({
      entries: pending,
      height: TERMINAL_BOUNDARY_HEIGHT,
      key: pending.map((candidate) => trajectoryVirtualRecordKey(candidate.record)).join("|"),
    });
  }

  return rows;
}
