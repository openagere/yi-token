import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import "@/styles/trajectory.css";
import { describeError } from "@/shared/api";
import { useI18n } from "@/shared/i18n";
import { useStepContext, useTrajectory } from "../hooks";
import {
  collapseAssistants,
  collapseTurns,
  collapsibleCalls,
  collapsibleTurns,
  filterRecords,
  flattenTurns,
  previousStepId,
} from "./records";
import { TrajectoryInspector, type InspectorTarget } from "./TrajectoryInspector";
import { TrajectoryLedger } from "./TrajectoryLedger";
import { TrajectoryTimeline } from "./TrajectoryTimeline";
import { TrajectoryToolbar } from "./TrajectoryToolbar";
import { useToolbarPrefs, type ToolbarPrefs } from "./toolbarPrefs";
import { focusIndexes, type TimelineMode, type TimeRange } from "./timeline";

/** Toolbar choices to timeline projection: equal or recorded width, idle gaps dropped or kept. */
function timelineMode(prefs: ToolbarPrefs): TimelineMode {
  if (prefs.actualDuration) return prefs.actualTime ? "actual" : "duration";
  return prefs.actualTime ? "time" : "sequence";
}

interface Props {
  sessionId: string;
  threadId: string | null;
  /** The request selected from the run page or a marker; shown as "已选中 请求 #N". */
  stepId: string | null;
  onStep: (stepId: string) => void;
  onThread: (threadId: string) => void;
}

/** Longer than a double click, so the first click of a fold gesture does not open the drawer. */
const INSPECTOR_OPEN_DELAY_MS = 300;

/** What the inspector shows: a record row, or the request marker of a step (by step id). */
type InspectorSelection = { kind: "record"; id: string } | { kind: "request"; stepId: string };

function toggled<T>(current: ReadonlySet<T>, value: T): Set<T> {
  const next = new Set(current);
  if (next.has(value)) next.delete(value);
  else next.add(value);
  return next;
}

/**
 * The trajectory of one thread: toolbar, then the ledger. Pages arrive newest first, so the ledger
 * reads them in reverse; folding and search are applied to the loaded records.
 */
export function TrajectoryView({ sessionId, threadId, stepId, onStep, onThread }: Props) {
  const { t } = useI18n();
  const trajectory = useTrajectory(sessionId, threadId);
  const step = useStepContext(stepId);
  const [prefs, setPrefs] = useToolbarPrefs();
  const [query, setQuery] = useState("");
  const [collapsedTurns, setCollapsedTurns] = useState<ReadonlySet<number>>(() => new Set());
  const [collapsedCalls, setCollapsedCalls] = useState<ReadonlySet<string>>(() => new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [inspectorSelection, setInspectorSelection] = useState<InspectorSelection | null>(null);
  const openTimer = useRef<number | undefined>(undefined);
  const [reveal, setReveal] = useState<{ id: string; seq: number } | null>(null);
  useEffect(() => () => window.clearTimeout(openTimer.current), []);

  /**
   * A record click selects the row now and opens the inspector a moment later. The delay lets a double click fold
   * or unfold a turn first: an open drawer would move the rows, or cover the one under the pointer.
   */
  function selectRecord(recordId: string) {
    setSelectedId(recordId);
    window.clearTimeout(openTimer.current);
    openTimer.current = window.setTimeout(() => {
      setInspectorSelection({ kind: "record", id: recordId });
    }, INSPECTOR_OPEN_DELAY_MS);
  }
  // The range belongs to the projection it was drawn in: switching modes drops it.
  const [timeRange, setTimeRange] = useState<{ mode: TimelineMode; range: TimeRange } | null>(null);
  const mode = timelineMode(prefs);
  const range = timeRange !== null && timeRange.mode === mode ? timeRange.range : null;

  const pages = trajectory.data?.pages;
  const base = useMemo(
    () => flattenTurns([...(pages ?? [])].reverse().flatMap((page) => page.turns)),
    [pages],
  );
  const foldableTurns = useMemo(() => collapsibleTurns(base), [base]);
  const foldableCalls = useMemo(() => collapsibleCalls(base), [base]);
  const rows = useMemo(
    () => collapseAssistants(collapseTurns(filterRecords(base, query), collapsedTurns, t), collapsedCalls, t),
    [base, query, collapsedTurns, collapsedCalls, t],
  );

  const focus = useMemo(() => (range === null ? null : focusIndexes(base, range, mode)), [base, range, mode]);

  // A selection whose record is no longer loaded (another thread, or a reloaded window) simply shows no inspector.
  const inspected = useMemo<InspectorTarget | null>(() => {
    if (inspectorSelection === null) return null;
    if (inspectorSelection.kind === "record") {
      const item = base.find((candidate) => candidate.record.id === inspectorSelection.id);
      return item === undefined ? null : { kind: "record", item };
    }
    const item = base.find((candidate) => candidate.step.id === inspectorSelection.stepId);
    return item === undefined ? null : { kind: "request", item };
  }, [base, inspectorSelection]);
  const inspectedPrevious = useMemo(
    () => (inspected === null ? null : previousStepId(base, inspected.item.step.id)),
    [base, inspected],
  );

  const allTurnsCollapsed =
    foldableTurns.size > 0 && [...foldableTurns].every((turn) => collapsedTurns.has(turn));
  const allCallsCollapsed =
    foldableCalls.size > 0 && [...foldableCalls].every((id) => collapsedCalls.has(id));
  const hasOlder = trajectory.hasNextPage;

  let body: ReactNode;
  if (trajectory.data === undefined && trajectory.isError) {
    body = (
      <p className="ledger-status" role="alert">
        {t("session.ledger.failed", { error: describeError(trajectory.error) })}
      </p>
    );
  } else if (trajectory.data === undefined) {
    body = <p className="ledger-status">{t("session.ledger.loading")}</p>;
  } else {
    body = (
      <>
        <TrajectoryTimeline
          records={base}
          mode={mode}
          range={range}
          onRangeChange={(next) => {
            setTimeRange(next === null ? null : { mode, range: next });
          }}
          onRecordSelect={(index) => {
            const item = base.find((candidate) => candidate.index === index);
            if (item === undefined) return;
            selectRecord(item.record.id);
            setReveal((current) => ({ id: item.record.id, seq: (current?.seq ?? 0) + 1 }));
          }}
        />
        <TrajectoryLedger
          records={rows}
          selectedId={selectedId}
          onSelect={selectRecord}
          focus={focus}
          reveal={reveal}
          collapsedTurns={collapsedTurns}
          collapsibleTurns={foldableTurns}
          collapsibleCalls={foldableCalls}
          selectedStepId={stepId}
          pageCount={pages?.length ?? 0}
          hasOlder={hasOlder}
          loadingOlder={trajectory.isFetchingNextPage}
          onLoadOlder={() => {
            void trajectory.fetchNextPage();
          }}
          onToggleTurn={(turn) => setCollapsedTurns((current) => toggled(current, turn))}
          onToggleCall={(id) => setCollapsedCalls((current) => toggled(current, id))}
          onStep={(stepId) => {
            // The ledger selects the marker's record first (which schedules a record inspector): cancel that, the request wins.
            window.clearTimeout(openTimer.current);
            setInspectorSelection({ kind: "request", stepId });
            onStep(stepId);
          }}
          onThread={onThread}
        />
        {rows.length === 0 && !hasOlder && (
          <p className="ledger-status">
            {base.length === 0 ? t("session.ledger.empty") : t("session.ledger.noMatch")}
          </p>
        )}
      </>
    );
  }

  return (
    <section
      className="trajectory-view"
      aria-label={t("session.ledger.label")}
      data-session-id={sessionId}
      data-thread-id={threadId ?? ""}
    >
      <TrajectoryToolbar
        prefs={prefs}
        onPrefs={setPrefs}
        allTurnsCollapsed={allTurnsCollapsed}
        onToggleAllTurns={() => setCollapsedTurns(allTurnsCollapsed ? new Set() : new Set(foldableTurns))}
        allCallsCollapsed={allCallsCollapsed}
        onToggleAllCalls={() => setCollapsedCalls(allCallsCollapsed ? new Set() : new Set(foldableCalls))}
        searchQuery={query}
        onSearchQueryChange={setQuery}
      />
      {step.data && (
        <p className="trajectory-selected-step">
          {t("session.ledger.selectedStep", { number: step.data.step.number })}
        </p>
      )}
      <div className="trajectory-main">
        <div className="trajectory-body">{body}</div>
        {inspected !== null && (
          <TrajectoryInspector
            target={inspected}
            previousStepId={inspectedPrevious}
            onClose={() => setInspectorSelection(null)}
          />
        )}
      </div>
    </section>
  );
}
