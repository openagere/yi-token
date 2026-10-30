import { useCallback, useState } from "react";

/** Display choices kept on this machine, so the ledger opens the way the user left it. */
export interface ToolbarPrefs {
  /** Timeline blocks use recorded durations instead of equal widths. */
  actualDuration: boolean;
  /** Timeline keeps the idle gaps between operations. */
  actualTime: boolean;
}

const STORAGE_KEY = "yi-token:trajectory-toolbar";
const DEFAULT_PREFS: ToolbarPrefs = { actualDuration: false, actualTime: false };

function readPrefs(): ToolbarPrefs {
  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(STORAGE_KEY) ?? "null",
    ) as Partial<ToolbarPrefs> | null;
    return { actualDuration: parsed?.actualDuration === true, actualTime: parsed?.actualTime === true };
  } catch {
    return DEFAULT_PREFS;
  }
}

function writePrefs(prefs: ToolbarPrefs) {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs));
  } catch {
    /* Storage unavailable: the choice only lasts for this session. */
  }
}

/** Toolbar display prefs, persisted under `yi-token:trajectory-toolbar`. */
export function useToolbarPrefs(): [ToolbarPrefs, (patch: Partial<ToolbarPrefs>) => void] {
  const [prefs, setPrefs] = useState(readPrefs);
  const update = useCallback((patch: Partial<ToolbarPrefs>) => {
    setPrefs((current) => {
      const next = { ...current, ...patch };
      writePrefs(next);
      return next;
    });
  }, []);
  return [prefs, update];
}
