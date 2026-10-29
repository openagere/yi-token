/** 会话列表左栏宽度：记在本机，范围与默认值见下方常量。 */
const STORAGE_KEY = "yi-token:sessions-list-width";
export const LIST_WIDTH_DEFAULT = 300;
export const LIST_WIDTH_MIN = 240;
export const LIST_WIDTH_MAX = 480;

export function clampListWidth(value: number): number {
  return Math.round(Math.min(LIST_WIDTH_MAX, Math.max(LIST_WIDTH_MIN, value)));
}

export function readListWidth(): number {
  try {
    const stored = Number(window.localStorage.getItem(STORAGE_KEY));
    return stored ? clampListWidth(stored) : LIST_WIDTH_DEFAULT;
  } catch {
    return LIST_WIDTH_DEFAULT;
  }
}

export function saveListWidth(width: number) {
  try {
    window.localStorage.setItem(STORAGE_KEY, String(width));
  } catch {
    /* Storage unavailable: the width only lasts for this session. */
  }
}
