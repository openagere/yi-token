import { useLayoutEffect, useState, type RefObject } from "react";

interface Options {
  /** Selector of one rendered row inside the container. */
  itemSelector: string;
  /** Row height used before any row is rendered. */
  fallbackHeight: number;
  /** Changes whenever the rendered rows may change, so the rows are measured again. */
  revision: unknown;
}

/**
 * How many rows fit in a list container whose height comes from the page layout (flex: 1 1 0),
 * so a list page shows exactly one screen of rows and pages through the rest instead of scrolling.
 * Re-measured when the container resizes or the rendered rows change.
 */
export function useFitRows(ref: RefObject<HTMLElement | null>, { itemSelector, fallbackHeight, revision }: Options): number {
  const [rows, setRows] = useState(6);
  useLayoutEffect(() => {
    const container = ref.current;
    if (!container) return;
    const measure = () => {
      const items = [...container.querySelectorAll<HTMLElement>(itemSelector)];
      const height = Math.max(0, ...items.map((item) => item.offsetHeight)) || fallbackHeight;
      const next = Math.max(1, Math.floor(container.clientHeight / height));
      setRows((current) => (current === next ? current : next));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    return () => observer.disconnect();
  }, [ref, itemSelector, fallbackHeight, revision]);
  return rows;
}
