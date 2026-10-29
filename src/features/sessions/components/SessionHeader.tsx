import type { SessionDetail, ThreadView } from "@/generated";
import { useI18n } from "@/shared/i18n";
import { clientLabel, formatDuration, formatTokens, tokenTotal } from "../format";
import { ThreadChips } from "./ThreadChips";

interface Props {
  detail: SessionDetail;
  activeThread: ThreadView | null;
  onThread: (threadId: string) => void;
}

/** 会话头：标题与线程切换片同一行，下方是终端、模型、步数、总 token、总时长（`last_at - started_at`）。 */
export function SessionHeader({ detail, activeThread, onThread }: Props) {
  const { t, language } = useI18n();
  const { summary, threads } = detail;
  const title = summary.title.trim() || t("session.list.untitled");
  const duration = formatDuration(summary.last_at - summary.started_at);
  const resetThread = threads.some((thread) => thread.reset);
  return (
    <header className="session-header" aria-label={t("session.header.label")}>
      <div className="session-header-main">
        <h2 title={title}>{title}</h2>
        <ThreadChips threads={threads} activeKind={activeThread?.kind ?? null} onSelect={(thread) => onThread(thread.id)} />
      </div>
      <p className="session-facts">
        <span>{clientLabel(summary.client, t)}</span>
        {summary.models.length > 0 && <span>{summary.models.join(" · ")}</span>}
        <span>{t("session.header.steps", { count: summary.step_count })}</span>
        <span>{t("session.header.tokens", { count: formatTokens(tokenTotal(summary), language) })}</span>
        <span>{t("session.header.duration", { duration })}</span>
      </p>
      {resetThread && (
        <p className="session-reset" role="status">
          <strong>{t("session.header.reset")}</strong> {t("session.header.resetHint")}
        </p>
      )}
    </header>
  );
}
