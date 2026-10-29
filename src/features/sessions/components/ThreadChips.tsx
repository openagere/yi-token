import type { ThreadKindView, ThreadView } from "@/generated";
import { useI18n } from "@/shared/i18n";

const KIND_ORDER: ThreadKindView[] = ["main", "subagent", "side", "other"];

interface Props {
  threads: ThreadView[];
  activeKind: ThreadKindView | null;
  /** 选择某一类线程时交出该类的第一条线程。 */
  onSelect: (thread: ThreadView) => void;
}

/** 线程切换片：主线程、子 Agent ×N、旁路 ×N、其他 ×N；只显示存在的类别。 */
export function ThreadChips({ threads, activeKind, onSelect }: Props) {
  const { t } = useI18n();
  const chips = KIND_ORDER.map((kind) => ({ kind, members: threads.filter((thread) => thread.kind === kind) })).filter(
    (chip) => chip.members.length > 0,
  );
  const label = (kind: ThreadKindView, count: number) => {
    if (kind === "main") return t("session.header.main");
    if (kind === "subagent") return t("session.header.subagent", { count });
    if (kind === "side") return t("session.header.side", { count });
    return t("session.header.other", { count });
  };
  return (
    <div className="thread-chips" role="group" aria-label={t("session.header.threads")}>
      {chips.map(({ kind, members }) => (
        <button key={kind} type="button" className="thread-chip" aria-pressed={activeKind === kind} onClick={() => onSelect(members[0])}>
          {label(kind, members.length)}
        </button>
      ))}
    </div>
  );
}
