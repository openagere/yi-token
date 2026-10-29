import { useEffect, useMemo, useRef, type UIEvent } from "react";
import { describeError } from "@/shared/api";
import { useI18n, type MessageKey } from "@/shared/i18n";
import type { SessionSummary } from "@/generated";
import { groupSessionsByDay, type DayGroup } from "../format";
import { useSessionList } from "../hooks";
import type { SessionFilter } from "../types";
import { SessionListItem } from "./SessionListItem";

const GROUP_LABELS = {
  today: "session.list.groups.today",
  yesterday: "session.list.groups.yesterday",
  week: "session.list.groups.week",
  earlier: "session.list.groups.earlier",
} as const satisfies Record<DayGroup, MessageKey>;

/** 距离底部这么近时加载下一页。 */
const LOAD_MORE_THRESHOLD = 160;

interface Props {
  filter: SessionFilter;
  /** 终端选择为 api 时：后端没有"无终端"筛选，因此在本地过滤并把剩余各页全部取回。 */
  apiOnly: boolean;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onDelete: (item: SessionSummary) => void;
}

/** 会话列表：按日分组，滚动到底部时加载下一页。 */
export function SessionList({ filter, apiOnly, selectedId, onSelect, onDelete }: Props) {
  const { t } = useI18n();
  const scroller = useRef<HTMLDivElement>(null);
  const query = useSessionList(filter);
  const { hasNextPage, isFetchingNextPage, fetchNextPage, dataUpdatedAt } = query;
  const items = useMemo(
    () => (query.data?.pages.flatMap((page) => page.items) ?? []).filter((item) => !apiOnly || item.client === null),
    [query.data, apiOnly],
  );
  const groups = groupSessionsByDay(items, dataUpdatedAt);
  const filtered = Boolean(filter.query || filter.client || filter.model || apiOnly);

  useEffect(() => {
    const element = scroller.current;
    if (!hasNextPage || isFetchingNextPage || !element) return;
    // Pull more when the list does not fill the pane (nothing to scroll), and always for the local api filter.
    if (apiOnly || element.scrollHeight <= element.clientHeight) void fetchNextPage();
  }, [apiOnly, hasNextPage, isFetchingNextPage, fetchNextPage, dataUpdatedAt]);

  function onScroll(event: UIEvent<HTMLDivElement>) {
    const element = event.currentTarget;
    if (hasNextPage && !isFetchingNextPage && element.scrollHeight - element.scrollTop - element.clientHeight < LOAD_MORE_THRESHOLD) {
      void fetchNextPage();
    }
  }

  return (
    <div className="session-list-scroll" ref={scroller} onScroll={onScroll} role="region" aria-label={t("session.list.label")}>
      {query.isPending && <p className="session-list-status">{t("session.list.loading")}</p>}
      {query.isError && <p className="session-list-status error" role="alert">{t("session.list.failed", { error: describeError(query.error) })}</p>}
      {query.isSuccess && items.length === 0 && <p className="session-list-status">{t(filtered ? "session.list.emptyFiltered" : "session.list.empty")}</p>}
      {groups.map((group) => (
        <section className="session-group" key={group.key}>
          <h3 className="session-group-label">{t(GROUP_LABELS[group.key])}</h3>
          <ul className="session-group-items">
            {group.items.map((item) => (
              <SessionListItem key={item.id} item={item} now={dataUpdatedAt} selected={item.id === selectedId} onSelect={onSelect} onDelete={onDelete} />
            ))}
          </ul>
        </section>
      ))}
      {isFetchingNextPage && <p className="session-list-status">{t("session.list.loadingMore")}</p>}
    </div>
  );
}
