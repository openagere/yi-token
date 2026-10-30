import { Clock, Search } from "lucide-react";
import { useI18n } from "@/shared/i18n";
import type { ToolbarPrefs } from "./toolbarPrefs";

export interface TrajectoryToolbarProps {
  prefs: ToolbarPrefs;
  onPrefs: (patch: Partial<ToolbarPrefs>) => void;
  allTurnsCollapsed: boolean;
  onToggleAllTurns: () => void;
  allCallsCollapsed: boolean;
  onToggleAllCalls: () => void;
  searchQuery: string;
  onSearchQueryChange: (query: string) => void;
}

/** Trajectory toolbar: timeline display choices, fold-all controls and the record search. */
export function TrajectoryToolbar({
  prefs,
  onPrefs,
  allTurnsCollapsed,
  onToggleAllTurns,
  allCallsCollapsed,
  onToggleAllCalls,
  searchQuery,
  onSearchQueryChange,
}: TrajectoryToolbarProps) {
  const { t } = useI18n();
  const durationLabel = prefs.actualDuration
    ? t("session.toolbar.durationActual")
    : t("session.toolbar.durationEqual");
  const turnsLabel = allTurnsCollapsed
    ? t("session.toolbar.expandTurns")
    : t("session.toolbar.collapseTurns");
  const callsLabel = allCallsCollapsed
    ? t("session.toolbar.expandCalls")
    : t("session.toolbar.collapseCalls");
  return (
    <div className="trajectory-toolbar" role="toolbar" aria-label={t("session.toolbar.label")}>
      <div className="trajectory-toolbar-actions">
        <button
          type="button"
          className="trajectory-toolbar-button"
          aria-pressed={prefs.actualDuration}
          aria-label={durationLabel}
          title={durationLabel}
          onClick={() => onPrefs({ actualDuration: !prefs.actualDuration })}
        >
          <Clock size={12} aria-hidden="true" />
          {durationLabel}
        </button>
        <button
          type="button"
          className="trajectory-toolbar-button"
          role="switch"
          aria-checked={prefs.actualTime}
          onClick={() => onPrefs({ actualTime: !prefs.actualTime })}
        >
          {t("session.toolbar.actualTime")}
        </button>
        <button
          type="button"
          className="trajectory-toolbar-button"
          aria-pressed={allTurnsCollapsed}
          aria-label={turnsLabel}
          title={turnsLabel}
          onClick={onToggleAllTurns}
        >
          {t("session.toolbar.turns")}
        </button>
        <button
          type="button"
          className="trajectory-toolbar-button"
          aria-pressed={allCallsCollapsed}
          aria-label={callsLabel}
          title={callsLabel}
          onClick={onToggleAllCalls}
        >
          {t("session.toolbar.calls")}
        </button>
      </div>
      <label className="trajectory-search">
        <Search size={12} aria-hidden="true" />
        <input
          type="search"
          aria-label={t("session.toolbar.search")}
          placeholder={t("session.toolbar.searchPlaceholder")}
          value={searchQuery}
          onChange={(event) => onSearchQueryChange(event.currentTarget.value)}
        />
      </label>
    </div>
  );
}
