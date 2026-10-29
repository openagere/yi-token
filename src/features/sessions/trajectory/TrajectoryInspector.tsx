import { useEffect, useState, type KeyboardEvent, type PointerEvent } from "react";
import { useI18n, type MessageKey } from "@/shared/i18n";
import { KIND_KEY, type LedgerRecord } from "./records";
import { TrajectoryInspectorTabs } from "./TrajectoryInspectorTabs";

/** A record row (or its request marker) selected in the ledger or the timeline. */
export type InspectorTarget =
  { kind: "record"; item: LedgerRecord } | { kind: "request"; item: LedgerRecord };

export type InspectorTabId =
  | "overview"
  | "preview"
  | "raw"
  | "arguments"
  | "result"
  | "schema"
  | "timing"
  | "system"
  | "tools"
  | "diff"
  | "usage"
  | "options"
  | "context";

export const TAB_KEYS: Record<InspectorTabId, MessageKey> = {
  overview: "session.inspector.tab.overview",
  preview: "session.inspector.tab.preview",
  raw: "session.inspector.tab.raw",
  arguments: "session.inspector.tab.arguments",
  result: "session.inspector.tab.result",
  schema: "session.inspector.tab.schema",
  timing: "session.inspector.tab.timing",
  system: "session.inspector.tab.system",
  tools: "session.inspector.tab.tools",
  diff: "session.inspector.tab.diff",
  usage: "session.inspector.tab.usage",
  options: "session.inspector.tab.options",
  context: "session.inspector.tab.context",
};

/**
 * The tabs of a target, in the order the spec table gives: system prompts, text records, tools, and request markers.
 * The diff tab only exists for a system row whose prompt changed.
 */
export function inspectorTabs(target: InspectorTarget): InspectorTabId[] {
  if (target.kind === "request") return ["usage", "options", "timing", "context"];
  const { record } = target.item;
  if (record.kind === "system")
    return record.system_changed ? ["system", "tools", "diff"] : ["system", "tools"];
  if (record.kind === "tool" || record.kind === "subtool")
    return ["overview", "arguments", "result", "schema", "timing"];
  return ["overview", "preview", "raw"];
}

/** Width in pixels, persisted per browser. */
export const INSPECTOR_WIDTH_KEY = "yi-token:trajectory-inspector-width";
export const INSPECTOR_DEFAULT_WIDTH = 420;
export const INSPECTOR_MIN_WIDTH = 320;
/** Share of the content width the drawer may take. */
export const INSPECTOR_MAX_SHARE = 0.7;
const RESIZE_STEP = 16;

function readWidth(): number {
  try {
    const stored = Number(window.localStorage.getItem(INSPECTOR_WIDTH_KEY));
    return Number.isFinite(stored) && stored >= INSPECTOR_MIN_WIDTH ? stored : INSPECTOR_DEFAULT_WIDTH;
  } catch {
    return INSPECTOR_DEFAULT_WIDTH;
  }
}

function writeWidth(width: number) {
  try {
    window.localStorage.setItem(INSPECTOR_WIDTH_KEY, String(Math.round(width)));
  } catch {
    // Storage can be unavailable (private windows, blocked site data); the width then lasts for this view only.
  }
}

/** Keeps a width within the minimum and the share of the content width it may take. */
export function clampInspectorWidth(width: number, contentWidth: number): number {
  const maximum = Math.max(INSPECTOR_MIN_WIDTH, contentWidth * INSPECTOR_MAX_SHARE);
  return Math.min(maximum, Math.max(INSPECTOR_MIN_WIDTH, width));
}

interface TrajectoryInspectorProps {
  target: InspectorTarget;
  /** The step before this one in the same thread, for the system diff; null when none is loaded. */
  previousStepId: string | null;
  onClose: () => void;
}

interface ResizeDrag {
  pointerId: number;
  startX: number;
  startWidth: number;
  contentWidth: number;
}

/**
 * The inspector drawer on the right of the trajectory: tabs follow the selected record kind (or the request marker),
 * the left edge drags to resize, and Escape closes it.
 */
export function TrajectoryInspector({ target, previousStepId, onClose }: TrajectoryInspectorProps) {
  const { t } = useI18n();
  const [width, setWidth] = useState(readWidth);
  const [drag, setDrag] = useState<ResizeDrag | null>(null);
  const [activeTab, setActiveTab] = useState<InspectorTabId>("overview");
  const tabs = inspectorTabs(target);
  const tab = tabs.includes(activeTab) ? activeTab : (tabs[0] ?? "overview");
  const { record, step } = target.item;
  const contentKey = target.kind === "request" ? `request:${step.id}` : `record:${record.id}`;

  useEffect(() => {
    function onKey(event: globalThis.KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  /** The drawer's parent is the content row: its width is the limit. */
  function contentWidthOf(handle: HTMLElement): number {
    return handle.parentElement?.parentElement?.getBoundingClientRect().width ?? 0;
  }

  function resizeTo(next: number, contentWidth: number) {
    const clamped = clampInspectorWidth(next, contentWidth);
    setWidth(clamped);
    writeWidth(clamped);
  }

  function onPointerDown(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    const aside = event.currentTarget.parentElement;
    if (aside === null) return;
    const contentWidth = contentWidthOf(event.currentTarget);
    setDrag({
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: aside.getBoundingClientRect().width,
      contentWidth,
    });
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  }

  function onPointerMove(event: PointerEvent<HTMLDivElement>) {
    if (drag === null || drag.pointerId !== event.pointerId) return;
    // The handle sits on the left edge, so moving left widens the drawer.
    setWidth(clampInspectorWidth(drag.startWidth + drag.startX - event.clientX, drag.contentWidth));
  }

  function onPointerUp(event: PointerEvent<HTMLDivElement>) {
    if (drag === null || drag.pointerId !== event.pointerId) return;
    setDrag(null);
    writeWidth(clampInspectorWidth(drag.startWidth + drag.startX - event.clientX, drag.contentWidth));
    event.currentTarget.releasePointerCapture(event.pointerId);
  }

  function onHandleKey(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const direction = event.key === "ArrowLeft" ? 1 : -1;
    resizeTo(width + direction * RESIZE_STEP, contentWidthOf(event.currentTarget));
  }

  const title =
    target.kind === "request" ? (
      <>
        <span className="trajectory-inspector-dot" aria-hidden="true" />
        <span className="trajectory-inspector-name">
          {t("session.ledger.requestLabel", { number: step.number })}
        </span>
        <span className="trajectory-inspector-location">
          {t("session.ledger.turnLabel", { turn: step.turn_no })}
        </span>
      </>
    ) : (
      <>
        <span className={`ledger-kind ledger-kind-${record.kind}`}>{t(KIND_KEY[record.kind])}</span>
        <span className="trajectory-inspector-location">
          {t("session.ledger.requestLabel", { number: step.number })}
        </span>
      </>
    );

  return (
    <aside
      className="trajectory-inspector"
      aria-label={t("session.inspector.label")}
      data-target={target.kind}
      style={{ width }}
    >
      <div
        className="trajectory-inspector-resize"
        role="separator"
        tabIndex={0}
        aria-orientation="vertical"
        aria-label={t("session.inspector.resize")}
        aria-controls="trajectory-inspector-panel"
        title={t("session.inspector.resizeTitle")}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => setDrag(null)}
        onDoubleClick={() => {
          setWidth(INSPECTOR_DEFAULT_WIDTH);
          writeWidth(INSPECTOR_DEFAULT_WIDTH);
        }}
        onKeyDown={onHandleKey}
      />
      <header className="trajectory-inspector-head">
        <div className="trajectory-inspector-title">{title}</div>
        <button
          type="button"
          className="trajectory-inspector-close"
          aria-label={t("session.inspector.close")}
          onClick={onClose}
        >
          <span aria-hidden="true">×</span>
        </button>
      </header>
      <div role="tablist" aria-label={t("session.inspector.tabs")} className="trajectory-inspector-tabs">
        {tabs.map((item) => (
          <button
            key={item}
            id={`trajectory-inspector-tab-${item}`}
            type="button"
            role="tab"
            aria-selected={tab === item}
            aria-controls="trajectory-inspector-panel"
            className={tab === item ? "trajectory-inspector-tab is-active" : "trajectory-inspector-tab"}
            onClick={() => setActiveTab(item)}
          >
            {t(TAB_KEYS[item])}
          </button>
        ))}
      </div>
      <div
        id="trajectory-inspector-panel"
        role="tabpanel"
        aria-labelledby={`trajectory-inspector-tab-${tab}`}
        className="trajectory-inspector-panel"
      >
        <TrajectoryInspectorTabs key={contentKey} tab={tab} target={target} previousStepId={previousStepId} />
      </div>
    </aside>
  );
}
