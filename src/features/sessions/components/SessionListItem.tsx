import { Plug, Trash2 } from "lucide-react";
import type { SessionSummary } from "@/generated";
import { useI18n } from "@/shared/i18n";
import { ProviderIcon } from "@/shared/ui";
import { formatRelative, formatTokens, terminalBrand, tokenTotal } from "../format";

interface Props {
  item: SessionSummary;
  /** 相对时间的参照点，由列表查询的更新时间提供，保持渲染纯净。 */
  now: number;
  selected: boolean;
  onSelect: (id: string) => void;
  onDelete: (item: SessionSummary) => void;
}

/** 列表中的一行：终端图标、标题、步数、token、相对时间，以及进行中、推断与新会话标记。 */
export function SessionListItem({ item, now, selected, onSelect, onDelete }: Props) {
  const { t, language } = useI18n();
  const brand = terminalBrand(item.client);
  const title = item.title.trim() || t("session.list.untitled");
  return (
    <li className={`session-item ${selected ? "selected" : ""}`} data-session-id={item.id}>
      <button type="button" className="session-item-select" aria-current={selected ? "true" : undefined} onClick={() => onSelect(item.id)}>
        <span className="session-item-icon" aria-hidden="true">
          {brand ? <ProviderIcon brand={brand} size={16} /> : <Plug size={16} />}
        </span>
        <span className="session-item-main">
          <span className="session-item-title" title={title}>{title}</span>
          <span className="session-item-meta">
            {t("session.list.steps", { count: item.step_count })} · {t("session.list.tokens", { count: formatTokens(tokenTotal(item), language) })} · {formatRelative(item.last_at, now, language)}
          </span>
        </span>
        {item.live && <span className="session-live" role="img" title={t("session.list.live")} aria-label={t("session.list.live")} />}
        {item.key_source === "chain" && <span className="session-marker inferred" title={t("session.list.inferredHint")}>{t("session.list.inferred")}</span>}
        {item.key_source === "new" && <span className="session-marker new" title={t("session.list.newSessionHint")}>{t("session.list.newSession")}</span>}
      </button>
      <button type="button" className="session-item-delete" title={t("session.list.delete")} aria-label={t("session.list.delete")} onClick={() => onDelete(item)}>
        <Trash2 size={14} aria-hidden="true" />
      </button>
    </li>
  );
}
