/**
 * Compact layout: the sidebar folds to its icon rail, and opening it lays it over the page.
 * The switch depends on the room left for the page, not on a fixed window width, so a sidebar
 * the user dragged wider folds earlier (and the window's minimum width still leaves room to fold).
 */
export const MIN_CONTENT_WIDTH = 760;
const DEFAULT_SIDEBAR_WIDTH = 196;

/** Width of the expanded sidebar: the user's dragged width or the stylesheet default. */
export function expandedSidebarWidth(): number {
  const value = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--sidebar-width"));
  return Number.isFinite(value) && value > 0 ? value : DEFAULT_SIDEBAR_WIDTH;
}

export function isCompactWindow(): boolean {
  if (typeof window === "undefined") return false;
  return window.innerWidth - expandedSidebarWidth() < MIN_CONTENT_WIDTH;
}
