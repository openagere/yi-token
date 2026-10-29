import type { TrajectoryStatus } from "@/generated";
import { describeError } from "@/shared/api";
import { useI18n } from "@/shared/i18n";
import { useSessionDetail } from "../hooks";
import { TrajectoryView } from "../trajectory/TrajectoryView";
import { SessionHeader } from "./SessionHeader";
import { TrajectoryUnavailable } from "./TrajectoryUnavailable";

interface Props {
  status: TrajectoryStatus | undefined;
  sessionId: string | null;
  stepId: string | null;
  threadId: string | null;
  onThread: (threadId: string) => void;
  onStep: (stepId: string) => void;
}

/** 右栏：轨迹库不可用时给出说明；否则显示所选会话的会话头与轨迹视图。 */
export function SessionDetailPane({ status, sessionId, stepId, threadId, onThread, onStep }: Props) {
  const { t } = useI18n();
  const unavailable = status !== undefined && !status.available;
  const detail = useSessionDetail(unavailable ? null : sessionId);
  if (status && !status.available) return <TrajectoryUnavailable error={status.error} />;
  if (!sessionId) return <p className="sessions-empty-detail">{t("session.placeholder.select")}</p>;
  if (detail.isError)
    return (
      <p className="sessions-empty-detail" role="alert">
        {describeError(detail.error)}
      </p>
    );
  if (!detail.data) return <p className="sessions-empty-detail">{t("session.list.loading")}</p>;
  const threads = detail.data.threads;
  const active =
    threads.find((thread) => thread.id === threadId) ??
    threads.find((thread) => thread.kind === "main") ??
    threads[0] ??
    null;
  return (
    <div className="session-detail">
      <SessionHeader detail={detail.data} activeThread={active} onThread={onThread} />
      {/* Keyed by thread: switching threads starts the ledger from its own folds, selection and scroll position. */}
      <TrajectoryView
        key={`${sessionId}:${active?.id ?? ""}`}
        sessionId={sessionId}
        threadId={active?.id ?? null}
        stepId={stepId}
        onStep={onStep}
        onThread={onThread}
      />
    </div>
  );
}
