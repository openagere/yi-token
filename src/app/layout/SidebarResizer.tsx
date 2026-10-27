import { useEffect, useRef, type KeyboardEvent, type PointerEvent } from "react";
import { useI18n } from "@/shared/i18n";

const STORAGE_KEY = "yi-token.sidebar-width";
const MIN = 168;
const MAX = 360;
const clamp = (value: number) => Math.round(Math.min(MAX, Math.max(MIN, value)));

function apply(width: number | null) {
  const root = document.documentElement;
  if (width === null) root.style.removeProperty("--sidebar-width");
  else root.style.setProperty("--sidebar-width", `${width}px`);
}

function save(width: number | null) {
  try {
    if (width === null) window.localStorage.removeItem(STORAGE_KEY);
    else window.localStorage.setItem(STORAGE_KEY, String(width));
  } catch {
    /* Storage unavailable: the width only lasts for this session. */
  }
}

function current(): number {
  return document.querySelector<HTMLElement>(".sidebar")?.getBoundingClientRect().width ?? 196;
}

/** 侧栏右缘的拖拽把手：拖动改宽度，双击恢复默认，方向键微调；宽度记在本机。 */
export function SidebarResizer() {
  const { t } = useI18n();
  const drag = useRef<{ x: number; width: number } | null>(null);

  useEffect(() => {
    try {
      const stored = Number(window.localStorage.getItem(STORAGE_KEY));
      if (stored) apply(clamp(stored));
    } catch {
      /* ignore */
    }
  }, []);

  function onPointerDown(event: PointerEvent<HTMLDivElement>) {
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { x: event.clientX, width: current() };
    document.documentElement.classList.add("sidebar-resizing");
  }
  function onPointerMove(event: PointerEvent<HTMLDivElement>) {
    if (!drag.current) return;
    apply(clamp(drag.current.width + event.clientX - drag.current.x));
  }
  function onPointerUp(event: PointerEvent<HTMLDivElement>) {
    if (!drag.current) return;
    event.currentTarget.releasePointerCapture(event.pointerId);
    drag.current = null;
    document.documentElement.classList.remove("sidebar-resizing");
    save(clamp(current()));
  }
  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const width = clamp(current() + (event.key === "ArrowRight" ? 12 : -12));
    apply(width);
    save(width);
  }

  return <div className="sidebar-resizer" role="separator" aria-orientation="vertical" aria-label={t("nav.resizeSidebar")} title={t("nav.resizeSidebar")}
    aria-valuemin={MIN} aria-valuemax={MAX} tabIndex={0}
    onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerUp}
    onDoubleClick={() => { apply(null); save(null); }} onKeyDown={onKeyDown} />;
}
