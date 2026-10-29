import { DEFAULT_SETTINGS } from "./defaults";
import { FONT_IDS, LANGUAGE_TAGS, THEME_MODES, type AppSettings, type FontId, type LanguageTag, type ThemeMode } from "./types";

/** 设置持久化：localStorage + 版本号。版本不匹配的旧数据不读取，直接使用默认值。 */
const STORAGE_KEY = "yi-token:settings";
const SETTINGS_VERSION = 1;

interface StoredSettings {
  version: number;
  /** 磁盘数据不可信，读取时经 sanitize 逐项校验。 */
  settings: unknown;
}

function isThemeMode(value: unknown): value is ThemeMode {
  return typeof value === "string" && (THEME_MODES as readonly string[]).includes(value);
}
function isFontId(value: unknown): value is FontId {
  return typeof value === "string" && (FONT_IDS as readonly string[]).includes(value);
}
function isLanguageTag(value: unknown): value is LanguageTag {
  return typeof value === "string" && (LANGUAGE_TAGS as readonly string[]).includes(value);
}

/** 逐项校验并回落到默认值，保证返回值始终完整合法。 */
function sanitize(raw: Record<string, unknown>): AppSettings {
  return {
    theme: isThemeMode(raw.theme) ? raw.theme : DEFAULT_SETTINGS.theme,
    font: isFontId(raw.font) ? raw.font : DEFAULT_SETTINGS.font,
    language: isLanguageTag(raw.language) ? raw.language : DEFAULT_SETTINGS.language,
  };
}

export function loadSettings(): AppSettings {
  try {
    const text = window.localStorage.getItem(STORAGE_KEY);
    if (!text) return { ...DEFAULT_SETTINGS };
    const stored = JSON.parse(text) as Partial<StoredSettings>;
    if (typeof stored !== "object" || stored === null || stored.version !== SETTINGS_VERSION) {
      return { ...DEFAULT_SETTINGS };
    }
    const data = typeof stored.settings === "object" && stored.settings !== null ? stored.settings as Record<string, unknown> : {};
    return sanitize(data);
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export function saveSettings(settings: AppSettings): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ version: SETTINGS_VERSION, settings } as StoredSettings));
  } catch {
    // 存储不可用时静默失败，设置仅在本次会话内生效。
  }
}
