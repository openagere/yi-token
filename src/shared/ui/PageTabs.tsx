import type { KeyboardEvent, ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import "@/styles/page-tabs.css";

export interface PageTab<T extends string> {
  id: T;
  label: string;
  icon?: LucideIcon;
  /** 标签右侧的计数或状态徽标；`badgeTone` 决定颜色。 */
  badge?: ReactNode;
  badgeTone?: "neutral" | "info" | "warning" | "danger";
}

interface Props<T extends string> {
  /** 标签与面板 id 的前缀，面板用 `tabPanelId(idPrefix, id)` 关联。 */
  idPrefix: string;
  label: string;
  tabs: readonly PageTab<T>[];
  value: T;
  onChange: (value: T) => void;
  /** 标签栏右侧与当前面板相关的工具（筛选、切换等）。 */
  actions?: ReactNode;
}

export const tabId = (prefix: string, id: string) => `${prefix}-tab-${id}`;
export const tabPanelId = (prefix: string, id: string) => `${prefix}-panel-${id}`;

/** 页面内的分区标签：一页只显示一个面板，左右方向键 / Home / End 切换。 */
export function PageTabs<T extends string>({ idPrefix, label, tabs, value, onChange, actions }: Props<T>) {
  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const index = tabs.findIndex((tab) => tab.id === value);
    const next =
      event.key === "ArrowRight" ? (index + 1) % tabs.length
        : event.key === "ArrowLeft" ? (index - 1 + tabs.length) % tabs.length
          : event.key === "Home" ? 0
            : event.key === "End" ? tabs.length - 1
              : -1;
    if (next < 0) return;
    event.preventDefault();
    onChange(tabs[next].id);
    document.getElementById(tabId(idPrefix, tabs[next].id))?.focus();
  }

  return (
    <div className="page-tabs">
      <div className="page-tabs-list" role="tablist" aria-label={label} onKeyDown={onKeyDown}>
        {tabs.map(({ id, label: tabLabel, icon: Icon, badge, badgeTone = "neutral" }) => {
          const selected = id === value;
          return (
            <button
              key={id}
              type="button"
              role="tab"
              id={tabId(idPrefix, id)}
              aria-selected={selected}
              aria-controls={tabPanelId(idPrefix, id)}
              tabIndex={selected ? 0 : -1}
              className="page-tab"
              onClick={() => onChange(id)}
            >
              {Icon && <Icon size={14} aria-hidden="true" />}
              <span>{tabLabel}</span>
              {badge !== undefined && badge !== null && badge !== "" && <span className={`page-tab-badge ${badgeTone}`}>{badge}</span>}
            </button>
          );
        })}
      </div>
      {actions && <div className="page-tabs-actions">{actions}</div>}
    </div>
  );
}
