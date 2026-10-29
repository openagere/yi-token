import { useRef, type KeyboardEvent, type PointerEvent } from "react";
import { useI18n } from "@/shared/i18n";
import { clampListWidth, LIST_WIDTH_DEFAULT, LIST_WIDTH_MAX, LIST_WIDTH_MIN, saveListWidth } from "../listWidth";

interface Props {
  width: number;
  onWidthChange: (width: number) => void;
}

/** 列表与轨迹之间的拖拽把手：拖动改宽，双击恢复默认，方向键微调；与侧栏把手的交互一致。 */
export function SessionsSplitter({ width, onWidthChange }: Props) {
  const { t } = useI18n();
  const drag = useRef<{ x: number; width: number; latest: number } | null>(null);

  function onPointerDown(event: PointerEvent<HTMLDivElement>) {
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { x: event.clientX, width, latest: width };
    document.documentElement.classList.add("sessions-resizing");
  }
  function onPointerMove(event: PointerEvent<HTMLDivElement>) {
    const current = drag.current;
    if (!current) return;
    current.latest = clampListWidth(current.width + event.clientX - current.x);
    onWidthChange(current.latest);
  }
  function onPointerUp(event: PointerEvent<HTMLDivElement>) {
    const current = drag.current;
    if (!current) return;
    event.currentTarget.releasePointerCapture(event.pointerId);
    drag.current = null;
    document.documentElement.classList.remove("sessions-resizing");
    saveListWidth(current.latest);
  }
  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const next = clampListWidth(width + (event.key === "ArrowRight" ? 12 : -12));
    onWidthChange(next);
    saveListWidth(next);
  }
  function onDoubleClick() {
    onWidthChange(LIST_WIDTH_DEFAULT);
    saveListWidth(LIST_WIDTH_DEFAULT);
  }

  return (
    <div
      className="sessions-splitter"
      role="separator"
      aria-orientation="vertical"
      aria-label={t("session.splitter")}
      title={t("session.splitter")}
      aria-valuemin={LIST_WIDTH_MIN}
      aria-valuemax={LIST_WIDTH_MAX}
      aria-valuenow={width}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onDoubleClick={onDoubleClick}
      onKeyDown={onKeyDown}
    />
  );
}
