import { Fragment, useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { formatMs } from "./inspectorFormat";
import { useI18n, type MessageKey } from "@/shared/i18n";
import type { LedgerRecord } from "./records";
import {
  deriveTimeline,
  recordTimings,
  type RecordTiming,
  type TimelineMode,
  type TimelineSpan,
  type TimeRange,
} from "./timeline";
import {
  LANE_HEIGHT,
  MIN_BAR_WIDTH,
  MIN_DRAG_PX,
  TIMELINE_HEIGHT,
  TOOLTIP_DELAY_MS,
  VIEW_WIDTH,
  centeredRange,
  clamp,
  clampWindow,
  minimumZoom,
  orderedRange,
  zoomedViewport,
} from "./timelineGeometry";

const LANES = [0, 1, 2] as const;
const BAR_INSET = 5;
const BAR_HEIGHT = LANE_HEIGHT - BAR_INSET * 2;

export interface TrajectoryTimelineProps {
  /** Every loaded record in ledger order, before search and folding. */
  records: readonly LedgerRecord[];
  mode: TimelineMode;
  /** The selected interval in the active projection's domain, or null. */
  range: TimeRange | null;
  onRangeChange: (range: TimeRange | null) => void;
  /** A span was clicked: the ledger should select the record with this LedgerRecord.index. */
  onRecordSelect: (index: number) => void;
}

interface DragGesture {
  pointerId: number;
  anchorTime: number;
  anchorX: number;
  recordIndex: number | null;
}

interface PanGesture {
  pointerId: number;
  anchorX: number;
  anchorStart: number;
  duration: number;
  moved: boolean;
}

function clock(timestamp: number, language: string): string {
  return new Date(timestamp).toLocaleTimeString(language, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** Record index under a pointer target, read from the span it sits on. */
function recordIndexOf(target: EventTarget | null): number | null {
  if (!(target instanceof Element)) return null;
  const value = target.closest("[data-timeline-record-index]")?.getAttribute("data-timeline-record-index");
  return value === undefined || value === null ? null : Number(value);
}

/** Overview of the trajectory in time: drag to select, wheel to zoom, right-drag to pan when zoomed. */
export function TrajectoryTimeline({
  records,
  mode,
  range,
  onRangeChange,
  onRecordSelect,
}: TrajectoryTimelineProps) {
  const { t, language } = useI18n();
  const model = useMemo(() => deriveTimeline(records, mode), [records, mode]);
  const timings = useMemo(() => recordTimings(records), [records]);
  const [viewport, setViewport] = useState<TimeRange | null>(null);
  const [draft, setDraft] = useState<TimeRange | null>(null);
  const [hovered, setHovered] = useState<{ index: number | null; fraction: number } | null>(null);
  const [tipIndex, setTipIndex] = useState<number | null>(null);
  const [panning, setPanning] = useState(false);
  const trackRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<DragGesture | null>(null);
  const panRef = useRef<PanGesture | null>(null);

  const full: TimeRange = model ?? { start: 0, end: 1 };
  const view =
    viewport !== null && model !== null && viewport.end > model.start && viewport.start < model.end
      ? viewport
      : null;
  const domain = view ?? full;
  const duration = Math.max(1, domain.end - domain.start);
  const activeRange = draft ?? range;
  const hoverIndex = hovered?.index ?? null;
  const tipSpanIndex = draft === null && hoverIndex !== null && tipIndex === hoverIndex ? hoverIndex : null;

  // A span's tooltip waits until the pointer has rested on it for the delay.
  useEffect(() => {
    if (hoverIndex === null) return;
    const timer = window.setTimeout(() => {
      setTipIndex(hoverIndex);
    }, TOOLTIP_DELAY_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [hoverIndex]);

  // Wheel zooms around the pointer. Registered natively and not passive, so the page does not scroll.
  useEffect(() => {
    const track = trackRef.current;
    if (track === null || model === null) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const rect = track.getBoundingClientRect();
      const anchor = clamp((event.clientX - rect.left) / Math.max(1, rect.width), 0, 1);
      const next = zoomedViewport(domain, model, anchor, event.deltaY, minimumZoom(mode, model));
      setViewport(next);
    };
    track.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      track.removeEventListener("wheel", onWheel);
    };
  }, [domain, mode, model]);

  if (model === null) {
    return (
      <section className="trajectory-timeline" aria-label={t("session.timeline.aria")}>
        <TimelineLabels t={t} />
        <div ref={trackRef} className="trajectory-timeline-track">
          <span className="trajectory-timeline-empty">{t("session.timeline.noTimingData")}</span>
        </div>
      </section>
    );
  }

  const fraction = (time: number) => (time - domain.start) / duration;
  const toX = (time: number) => fraction(time) * VIEW_WIDTH;
  const timeAt = (pointerFraction: number) => domain.start + pointerFraction * duration;
  const pointerFraction = (clientX: number, element: HTMLElement) => {
    const rect = element.getBoundingClientRect();
    return clamp((clientX - rect.left) / Math.max(1, rect.width), 0, 1);
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    const track = event.currentTarget;
    if (event.button === 2) {
      panRef.current = {
        pointerId: event.pointerId,
        anchorX: event.clientX,
        anchorStart: domain.start,
        duration,
        moved: false,
      };
      setPanning(true);
      track.setPointerCapture(event.pointerId);
      return;
    }
    if (event.button !== 0) return;
    const anchorTime = timeAt(pointerFraction(event.clientX, track));
    dragRef.current = {
      pointerId: event.pointerId,
      anchorTime,
      anchorX: event.clientX,
      recordIndex: recordIndexOf(event.target),
    };
    track.setPointerCapture(event.pointerId);
    setDraft({ start: anchorTime, end: anchorTime });
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const pointer = pointerFraction(event.clientX, event.currentTarget);
    const index = recordIndexOf(event.target);
    if (index !== hoverIndex) setTipIndex(null);
    setHovered({ index, fraction: pointer });
    const pan = panRef.current;
    if (pan !== null && pan.pointerId === event.pointerId) {
      if (Math.abs(event.clientX - pan.anchorX) >= MIN_DRAG_PX) pan.moved = true;
      if (view === null) return;
      const width = event.currentTarget.getBoundingClientRect().width;
      const delta = (event.clientX - pan.anchorX) / Math.max(1, width);
      setViewport(clampWindow(pan.anchorStart - delta * pan.duration, pan.duration, model));
      return;
    }
    const drag = dragRef.current;
    if (drag !== null && drag.pointerId === event.pointerId) {
      setDraft(orderedRange(drag.anchorTime, timeAt(pointer)));
    }
  };

  const onPointerEnd = (event: PointerEvent<HTMLDivElement>) => {
    const pan = panRef.current;
    if (pan !== null && pan.pointerId === event.pointerId) {
      panRef.current = null;
      setPanning(false);
      // A right click without travel clears the selection; a right drag only pans.
      if (!pan.moved) onRangeChange(null);
      return;
    }
    const drag = dragRef.current;
    if (drag === null || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    setDraft(null);
    const selected = orderedRange(
      drag.anchorTime,
      timeAt(pointerFraction(event.clientX, event.currentTarget)),
    );
    const click = Math.abs(event.clientX - drag.anchorX) < MIN_DRAG_PX;
    const clicked =
      click && drag.recordIndex !== null
        ? model.spans.find((span) => span.index === drag.recordIndex)
        : undefined;
    if (clicked !== undefined) {
      onRangeChange(null);
      onRecordSelect(clicked.index);
      return;
    }
    const minimum = Math.min(duration, (model.end - model.start) / model.spans.length);
    onRangeChange(
      selected.end - selected.start < minimum
        ? centeredRange(
            click ? selected.start : (selected.start + selected.end) / 2,
            minimum,
            model.start,
            model.end,
          )
        : selected,
    );
  };

  const onPointerCancel = () => {
    dragRef.current = null;
    panRef.current = null;
    setDraft(null);
    setHovered(null);
    setTipIndex(null);
    setPanning(false);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Escape" || range === null) return;
    event.preventDefault();
    onRangeChange(null);
  };

  const visibleSpans = model.spans.filter((span) => span.end >= domain.start && span.start <= domain.end);
  const tip = tipSpanIndex === null ? undefined : model.spans.find((span) => span.index === tipSpanIndex);
  const selectionLeft = activeRange === null ? 0 : clamp(fraction(activeRange.start), 0, 1) * 100;
  const selectionRight = activeRange === null ? 0 : clamp(fraction(activeRange.end), 0, 1) * 100;

  return (
    <section className="trajectory-timeline" aria-label={t("session.timeline.aria")}>
      <TimelineLabels t={t} />
      <div
        ref={trackRef}
        className="trajectory-timeline-track"
        data-zoomed={view === null ? undefined : "true"}
        data-panning={panning || undefined}
        aria-label={t("session.timeline.overviewAria")}
        tabIndex={0}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerEnd}
        onPointerCancel={onPointerCancel}
        onPointerLeave={() => {
          if (dragRef.current === null && panRef.current === null) {
            setHovered(null);
            setTipIndex(null);
          }
        }}
        onKeyDown={onKeyDown}
        onContextMenu={(event) => {
          event.preventDefault();
        }}
      >
        <svg
          className="trajectory-timeline-svg"
          viewBox={`0 0 ${VIEW_WIDTH} ${TIMELINE_HEIGHT}`}
          preserveAspectRatio="none"
          aria-hidden="true"
          focusable="false"
        >
          {LANES.map((lane) => (
            <g key={lane} data-timeline-lane={lane}>
              <rect
                className="tl-lane"
                x={0}
                y={lane * LANE_HEIGHT}
                width={VIEW_WIDTH}
                height={LANE_HEIGHT}
              />
            </g>
          ))}
          {model.turnBoundaries
            .filter((boundary) => boundary.time >= domain.start && boundary.time <= domain.end)
            .map((boundary) => (
              <line
                key={boundary.turn}
                className="tl-turn"
                data-turn-boundary
                data-turn={boundary.turn}
                x1={toX(boundary.time)}
                x2={toX(boundary.time)}
                y1={0}
                y2={TIMELINE_HEIGHT}
              />
            ))}
          {visibleSpans.map((span) => (
            <SpanBar
              key={span.index}
              span={span}
              x={toX(span.start)}
              width={Math.max(MIN_BAR_WIDTH, toX(span.end) - toX(span.start))}
              generationX={span.ttftEnd === undefined ? null : toX(span.ttftEnd)}
              inRange={
                activeRange === null ? null : span.start <= activeRange.end && span.end >= activeRange.start
              }
              hovered={hoverIndex === span.index}
              endX={toX(span.end)}
            />
          ))}
        </svg>
        {activeRange !== null && (
          <div
            className="trajectory-timeline-selection"
            data-dragging={draft === null ? undefined : "true"}
            aria-hidden="true"
            style={{ left: `${selectionLeft}%`, width: `${Math.max(0, selectionRight - selectionLeft)}%` }}
          />
        )}
        {tip !== undefined && (
          <TimelineTip
            span={tip}
            timing={timings.get(tip.index)}
            kindLabel={t(`session.ledger.kind.${tip.kind}` as MessageKey)}
            left={(hovered?.fraction ?? 0) * 100}
            language={language}
            t={t}
          />
        )}
      </div>
    </section>
  );
}

function TimelineLabels({ t }: { t: ReturnType<typeof useI18n>["t"] }) {
  return (
    <div className="trajectory-timeline-labels" aria-hidden="true">
      <span>{t("session.timeline.lanes.input")}</span>
      <span>{t("session.timeline.lanes.model")}</span>
      <span>{t("session.timeline.lanes.tools")}</span>
    </div>
  );
}

interface SpanBarProps {
  span: TimelineSpan;
  x: number;
  width: number;
  /** Assistant spans only: where generation starts. Everything before it is time to first token. */
  generationX: number | null;
  endX: number;
  inRange: boolean | null;
  hovered: boolean;
}

function SpanBar({ span, x, width, generationX, endX, inRange, hovered }: SpanBarProps) {
  const y = span.lane * LANE_HEIGHT + BAR_INSET;
  const estimated = span.kind === "tool" || span.kind === "subtool";
  return (
    <Fragment>
      <rect
        className={`tl-span tl-${span.kind}`}
        data-timeline-span={span.kind}
        data-timeline-record-index={span.index}
        data-assistant-timing={generationX === null ? undefined : "true"}
        data-error={span.isError ? "true" : undefined}
        data-estimated={estimated ? "true" : undefined}
        data-in-range={inRange === null ? undefined : String(inRange)}
        data-hovered={hovered || undefined}
        x={x}
        y={y}
        width={width}
        height={BAR_HEIGHT}
      />
      {generationX !== null && (
        <rect
          className="tl-generation"
          data-timeline-generation
          x={generationX}
          y={y}
          width={Math.max(0, endX - generationX)}
          height={BAR_HEIGHT}
        />
      )}
    </Fragment>
  );
}

interface TimelineTipProps {
  span: TimelineSpan;
  timing: RecordTiming | undefined;
  kindLabel: string;
  left: number;
  language: string;
  t: ReturnType<typeof useI18n>["t"];
}

function TimelineTip({ span, timing, kindLabel, left, language, t }: TimelineTipProps) {
  const lines: string[] = [kindLabel];
  if (timing !== undefined) {
    lines.push(t("session.timeline.started", { time: clock(timing.start, language) }));
    const durationText = t("session.timeline.duration", {
      duration: formatMs(timing.end - timing.start, language),
    });
    lines.push(timing.estimated ? `${durationText} ${t("session.timeline.estimated")}` : durationText);
    if (timing.ttftMs !== undefined && timing.generationMs !== undefined) {
      lines.push(
        t("session.timeline.ttftGeneration", {
          ttft: formatMs(timing.ttftMs, language),
          generation: formatMs(timing.generationMs, language),
        }),
      );
    }
  }
  return (
    <div
      role="tooltip"
      className="trajectory-timeline-tip"
      data-kind={span.kind}
      style={{ left: `${left}%` }}
    >
      {lines.map((line) => (
        <span key={line} className="trajectory-timeline-tip-line">
          {line}
        </span>
      ))}
    </div>
  );
}
