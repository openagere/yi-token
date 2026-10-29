import type { TimelineMode, TimeRange } from "./timeline";

/** The SVG viewBox is this wide; bars are positioned in these units so the track can stretch freely. */
export const VIEW_WIDTH = 1000;
export const TIMELINE_HEIGHT = 72;
export const LANE_HEIGHT = 24;
/** Narrowest bar drawn, in viewBox units, so a zero-length request still shows. */
export const MIN_BAR_WIDTH = 2;
/** Pointer travel below this (px) is a click, not a drag. */
export const MIN_DRAG_PX = 3;
export const TOOLTIP_DELAY_MS = 500;
/** Sequence mode never zooms closer than this many slots. */
const MIN_SEQUENCE_SLOTS = 4;
/** Timed modes never zoom closer than the full domain divided by this. */
const MAX_TIMED_ZOOM = 500;
const ZOOM_RATE = 0.0015;

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function orderedRange(left: number, right: number): TimeRange {
  return left <= right ? { start: left, end: right } : { start: right, end: left };
}

/** A window `width` wide centred on `center`, kept inside [minimum, maximum]. */
export function centeredRange(center: number, width: number, minimum: number, maximum: number): TimeRange {
  const clampedWidth = Math.min(maximum - minimum, Math.max(0, width));
  const start = clamp(center - clampedWidth / 2, minimum, maximum - clampedWidth);
  return { start, end: start + clampedWidth };
}

/** A window of `duration` starting at `start`, moved so it stays inside the full domain. */
export function clampWindow(start: number, duration: number, full: TimeRange): TimeRange {
  const clampedStart = clamp(start, full.start, Math.max(full.start, full.end - duration));
  return { start: clampedStart, end: clampedStart + duration };
}

/** Smallest window a zoom may reach, in the units of the active projection. */
export function minimumZoom(mode: TimelineMode, full: TimeRange): number {
  const span = full.end - full.start;
  return mode === "sequence" ? Math.min(MIN_SEQUENCE_SLOTS, span) : span / MAX_TIMED_ZOOM;
}

/**
 * The viewport after one wheel step, zoomed around `anchor` (0 to 1 across the track). Returns null when
 * the step zooms back out to the full domain.
 */
export function zoomedViewport(
  view: TimeRange,
  full: TimeRange,
  anchor: number,
  deltaY: number,
  minimumDuration: number,
): TimeRange | null {
  const duration = view.end - view.start;
  const fullDuration = full.end - full.start;
  const next = clamp(
    duration * Math.exp(deltaY * ZOOM_RATE),
    Math.min(minimumDuration, fullDuration),
    fullDuration,
  );
  if (next >= fullDuration * 0.999) return null;
  const anchorTime = view.start + anchor * duration;
  return clampWindow(anchorTime - anchor * next, next, full);
}
