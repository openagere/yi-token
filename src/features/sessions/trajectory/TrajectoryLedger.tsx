import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  type KeyboardEvent,
  type MouseEvent,
  type UIEvent,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Brain } from "lucide-react";
import type { RecordKind, TrajectoryRecord } from "@/generated";
import { useI18n, type MessageKey, type Translate } from "@/shared/i18n";
import { isCall, stateOf, type LedgerRecord } from "./records";
import {
  groupTrajectoryVirtualRows,
  trajectoryVirtualRecordKey,
  type TrajectoryVirtualRow,
} from "./virtualRows";

/** Closer than this to the bottom counts as "at the tail". */
const FOLLOW_THRESHOLD_PX = 40;
const LOAD_ROW_HEIGHT_PX = 30;
const OVERSCAN_ROWS = 8;
const LOAD_ROW_KEY = "load-earlier";

const KIND_KEY: Record<RecordKind, MessageKey> = {
  system: "session.ledger.kind.system",
  user: "session.ledger.kind.user",
  message: "session.ledger.kind.message",
  tool: "session.ledger.kind.tool",
  subtool: "session.ledger.kind.subtool",
  compacted: "session.ledger.kind.compacted",
};

export interface TrajectoryLedgerProps {
  /** Records after search and folding, in ledger order. */
  records: readonly LedgerRecord[];
  collapsedTurns: ReadonlySet<number>;
  collapsibleTurns: ReadonlySet<number>;
  collapsibleCalls: ReadonlySet<string>;
  /** Selected record id; the timeline and the rows both drive it through `onSelect`. */
  selectedId: string | null;
  onSelect: (recordId: string) => void;
  /** Record indexes inside the timeline's selected range, or null without a selection. */
  focus: ReadonlySet<number> | null;
  /** Bumped by a timeline click: the row with this id is scrolled into view. */
  reveal: { id: string; seq: number } | null;
  selectedStepId: string | null;
  /** Number of loaded pages; a larger value means older turns were prepended. */
  pageCount: number;
  hasOlder: boolean;
  loadingOlder: boolean;
  onLoadOlder: () => void;
  onToggleTurn: (turn: number) => void;
  onToggleCall: (recordId: string) => void;
  onStep: (stepId: string) => void;
  onThread: (threadId: string) => void;
}

interface ScrollAnchor {
  scrollTop: number;
  scrollHeight: number;
  pageCount: number;
}

function isNearBottom(pane: HTMLElement): boolean {
  return pane.scrollHeight - pane.clientHeight - pane.scrollTop < FOLLOW_THRESHOLD_PX;
}

/** The record's own text: the system prompt label, the call head for tools, or the summary. */
function primaryText(record: TrajectoryRecord, t: Translate): string {
  if (record.kind === "system")
    return record.system_changed ? t("session.ledger.systemChanged") : t("session.ledger.systemPrompt");
  if (!isCall(record)) return record.text;
  const args = (record.args ?? "").replace(/\s+/g, " ").trim();
  const head = [record.tool_name ?? "", args].filter((part) => part !== "").join(" · ");
  return head === "" ? record.text : head;
}

/** Result shown after the arrow on a call row. Undefined while the call is still running. */
function resultText(item: LedgerRecord, t: Translate): string | undefined {
  if (!isCall(item.record)) return undefined;
  const result = item.record.result;
  if (result !== null && result !== "") return result;
  return stateOf(item) === "running" ? undefined : t("session.ledger.noOutput");
}

interface LedgerRowProps {
  item: LedgerRecord;
  selected: boolean;
  /** Outside the timeline's selected range: drawn dimmed. */
  outside: boolean;
  turnActive: boolean;
  stepSelected: boolean;
  onActivate: (item: LedgerRecord) => void;
  onDoubleClick: (item: LedgerRecord, event: MouseEvent<HTMLTableRowElement>) => void;
  onKeyDown: (item: LedgerRecord, event: KeyboardEvent<HTMLTableRowElement>) => void;
  onStep: (item: LedgerRecord) => void;
  onThread: (threadId: string) => void;
}

/** One ledger row: request marker, turn label and kind tag on the left; the record or a folded summary on the right. */
function LedgerRow({
  item,
  selected,
  outside,
  turnActive,
  stepSelected,
  onActivate,
  onDoubleClick,
  onKeyDown,
  onStep,
  onThread,
}: LedgerRowProps) {
  const { t } = useI18n();
  const { record } = item;
  const summary = item.collapsedSummary;
  const folded = summary !== undefined;
  const state = stateOf(item);
  const result = resultText(item, t);
  const subThread = isCall(record) ? record.sub_thread_id : null;
  const foldedLabel =
    item.collapsedSummaryKind === "turn" ? "session.ledger.collapsedTurn" : "session.ledger.collapsedCalls";
  const requestLabel = t("session.ledger.requestLabel", { number: item.step.number });

  return (
    <tr
      className="ledger-row"
      tabIndex={0}
      aria-selected={folded ? undefined : selected}
      aria-label={folded ? t(foldedLabel, { summary: summary ?? "" }) : undefined}
      data-kind={record.kind}
      data-trajectory-row-key={trajectoryVirtualRecordKey(item)}
      data-collapsed-summary={item.collapsedSummaryKind}
      data-selected={selected || undefined}
      data-timeline-focus={outside ? "outside" : undefined}
      data-turn-active={turnActive || undefined}
      data-turn-start={item.turnStart || undefined}
      data-turn-end={item.turnEnd || undefined}
      data-error={record.is_error || undefined}
      data-running={state === "running" || undefined}
      onClick={() => onActivate(item)}
      onDoubleClick={(event) => onDoubleClick(item, event)}
      onKeyDown={(event) => onKeyDown(item, event)}
    >
      <td className="ledger-kind-cell">
        {!folded && item.stepStart && (
          <button
            type="button"
            className="ledger-request-marker"
            aria-label={requestLabel}
            aria-pressed={stepSelected}
            title={requestLabel}
            data-request-status={state}
            onClick={(event) => {
              event.stopPropagation();
              onStep(item);
            }}
            onDoubleClick={(event) => event.stopPropagation()}
          />
        )}
        {!folded && item.turnStart && (
          <span className={turnActive ? "ledger-turn-label is-active" : "ledger-turn-label"}>
            #{item.turn}
          </span>
        )}
        {!folded && (
          <span className={`ledger-kind ledger-kind-${record.kind}`} data-role-kind={record.kind}>
            {t(KIND_KEY[record.kind])}
          </span>
        )}
      </td>
      <td className="ledger-content-cell">
        {folded ? (
          <span className="ledger-summary" title={summary}>
            <span className="ledger-summary-ellipsis" aria-hidden="true">
              …
            </span>
            {summary}
          </span>
        ) : (
          <span
            className="ledger-content"
            title={result === undefined ? undefined : `${record.text} → ${result}`}
          >
            {record.kind === "message" && record.thinking !== null && (
              <Brain
                className="ledger-thinking"
                size={12}
                role="img"
                aria-label={t("session.ledger.thinking")}
              />
            )}
            <span className="ledger-primary">{primaryText(record, t)}</span>
            {result !== undefined && (
              <span className={record.is_error ? "ledger-result is-error" : "ledger-result"}>
                <span className="ledger-arrow" aria-hidden="true">
                  →
                </span>
                {result}
              </span>
            )}
            {subThread !== null && (
              <button
                type="button"
                className="ledger-subagent"
                onClick={(event) => {
                  event.stopPropagation();
                  onThread(subThread);
                }}
                onDoubleClick={(event) => event.stopPropagation()}
              >
                {t("session.ledger.expandSubagent")}
              </button>
            )}
          </span>
        )}
      </td>
    </tr>
  );
}

/**
 * The ledger table. Virtualized, follows the tail while the reader is at the bottom, and keeps the viewport
 * still when older turns are prepended.
 */
export function TrajectoryLedger({
  records,
  collapsedTurns,
  collapsibleTurns,
  collapsibleCalls,
  selectedId,
  onSelect,
  focus,
  reveal,
  selectedStepId,
  pageCount,
  hasOlder,
  loadingOlder,
  onLoadOlder,
  onToggleTurn,
  onToggleCall,
  onStep,
  onThread,
}: TrajectoryLedgerProps) {
  const { t } = useI18n();
  const handledReveal = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const followRef = useRef(true);
  const anchorRef = useRef<ScrollAnchor | null>(null);
  const rows = useMemo<readonly TrajectoryVirtualRow<LedgerRecord>[]>(
    () => groupTrajectoryVirtualRows(records),
    [records],
  );
  const offset = hasOlder ? 1 : 0;

  const virtualizer = useVirtualizer<HTMLDivElement, HTMLTableRowElement>({
    count: rows.length + offset,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) =>
      index < offset ? LOAD_ROW_HEIGHT_PX : (rows[index - offset]?.height ?? LOAD_ROW_HEIGHT_PX),
    getItemKey: (index) => (index < offset ? LOAD_ROW_KEY : (rows[index - offset]?.key ?? index)),
    overscan: OVERSCAN_ROWS,
  });
  const items = virtualizer.getVirtualItems();
  const topSpace = items[0]?.start ?? 0;
  const bottomSpace =
    items.length === 0 ? 0 : Math.max(0, virtualizer.getTotalSize() - (items.at(-1)?.end ?? 0));

  // Follow the tail when new records arrive; after an older page is prepended, keep the viewport where it was.
  // The spacer sizes are in the dependencies: the virtualizer renders its window one pass after the first commit,
  // so the pane only has its full height once the spacers exist.
  useLayoutEffect(() => {
    const pane = scrollRef.current;
    if (pane === null) return;
    const anchor = anchorRef.current;
    if (anchor !== null && pageCount !== anchor.pageCount) {
      anchorRef.current = null;
      pane.scrollTop = anchor.scrollTop + (pane.scrollHeight - anchor.scrollHeight);
      followRef.current = isNearBottom(pane);
      return;
    }
    if (followRef.current) pane.scrollTop = pane.scrollHeight;
  }, [rows, pageCount, topSpace, bottomSpace]);

  // A timeline click scrolls its record into view once the record's row exists.
  useEffect(() => {
    if (reveal === null || reveal.seq === handledReveal.current) return;
    const position = rows.findIndex((row) =>
      row.entries.some(({ record }) => record.record.id === reveal.id),
    );
    if (position < 0) return;
    handledReveal.current = reveal.seq;
    virtualizer.scrollToIndex(position + offset, { align: "center" });
  }, [reveal, rows, offset, virtualizer]);

  function onScroll(event: UIEvent<HTMLDivElement>) {
    followRef.current = isNearBottom(event.currentTarget);
  }

  function loadOlder() {
    const pane = scrollRef.current;
    if (pane !== null)
      anchorRef.current = { scrollTop: pane.scrollTop, scrollHeight: pane.scrollHeight, pageCount };
    onLoadOlder();
  }

  const activeTurn = records.find((item) => item.record.id === selectedId)?.turn ?? null;

  function activate(item: LedgerRecord) {
    if (item.collapsedSummary === undefined) {
      onSelect(item.record.id);
    } else if (item.collapsedSummaryKind === "turn") {
      onToggleTurn(item.turn);
    } else {
      onToggleCall(item.record.id);
    }
  }

  function onDoubleClick(item: LedgerRecord, event: MouseEvent<HTMLTableRowElement>) {
    if (item.collapsedSummary !== undefined) return;
    // A folded turn opens from any of its rows; a call-carrying assistant row folds its calls; a turn header folds its turn.
    if (collapsedTurns.has(item.turn)) {
      event.preventDefault();
      onToggleTurn(item.turn);
    } else if (item.record.kind === "message" && collapsibleCalls.has(item.record.id)) {
      event.preventDefault();
      onToggleCall(item.record.id);
    } else if (item.turnStart && collapsibleTurns.has(item.turn)) {
      event.preventDefault();
      onToggleTurn(item.turn);
    }
  }

  function onKeyDown(item: LedgerRecord, event: KeyboardEvent<HTMLTableRowElement>) {
    // Keys pressed on a control inside the row (the request marker, the subagent button) belong to that control.
    if (event.target !== event.currentTarget) return;
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    activate(item);
  }

  return (
    <div ref={scrollRef} className="ledger-scroll" data-trajectory-scroll="" onScroll={onScroll}>
      <table role="grid" className="ledger-table" aria-label={t("session.ledger.label")}>
        <colgroup>
          <col className="ledger-kind-col" />
          <col />
        </colgroup>
        <tbody>
          {topSpace > 0 && (
            <tr className="ledger-spacer" aria-hidden="true">
              <td colSpan={2} style={{ height: topSpace }} />
            </tr>
          )}
          {items.flatMap((item) => {
            if (item.index < offset) {
              return [
                <tr key={LOAD_ROW_KEY} className="ledger-load-row" data-history-load="">
                  <td colSpan={2}>
                    <button
                      type="button"
                      className="ledger-load-button"
                      disabled={loadingOlder}
                      onClick={loadOlder}
                    >
                      {loadingOlder && <span className="ledger-spin" aria-hidden="true" />}
                      <span aria-live="polite">
                        {loadingOlder ? t("session.ledger.loadingEarlier") : t("session.ledger.loadEarlier")}
                      </span>
                    </button>
                  </td>
                </tr>,
              ];
            }
            const row = rows[item.index - offset];
            if (row === undefined) return [];
            return row.entries.map(({ record }) => (
              <LedgerRow
                key={trajectoryVirtualRecordKey(record)}
                item={record}
                selected={selectedId === record.record.id}
                outside={focus !== null && !focus.has(record.index)}
                turnActive={activeTurn !== null && record.turn === activeTurn}
                stepSelected={selectedStepId === record.step.id}
                onActivate={activate}
                onDoubleClick={onDoubleClick}
                onKeyDown={onKeyDown}
                onStep={(stepItem) => {
                  onSelect(stepItem.record.id);
                  onStep(stepItem.step.id);
                }}
                onThread={onThread}
              />
            ));
          })}
          {bottomSpace > 0 && (
            <tr className="ledger-spacer" aria-hidden="true">
              <td colSpan={2} style={{ height: bottomSpace }} />
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
