import { useEffect, useState, type CSSProperties } from "react";
import { ArrowLeft } from "lucide-react";
import "@/styles/sessions.css";
import type { SessionSummary } from "@/generated";
import { useI18n } from "@/shared/i18n";
import { useNavigationStore, type SessionTarget } from "@/shared/navigation";
import { ConfirmDialog, PageHeader } from "@/shared/ui";
import { describeError } from "@/shared/api";
import { useDeleteSession, useSessionChanges, useTrajectoryStatus } from "../hooks";
import { readListWidth } from "../listWidth";
import type { SessionFilter } from "../types";
import { useDebouncedValue } from "../useDebouncedValue";
import { SessionDetailPane } from "./SessionDetailPane";
import { SessionList } from "./SessionList";
import { SessionListToolbar, type TerminalFilter } from "./SessionListToolbar";
import { SessionsSplitter } from "./SessionsSplitter";
import { TrajectoryNotice } from "./TrajectoryNotice";

/** 标题搜索停止输入这么久后才生效。 */
const SEARCH_DELAY_MS = 300;

interface Selection {
  sessionId: string | null;
  stepId: string | null;
  threadId: string | null;
}

const NO_SELECTION: Selection = { sessionId: null, stepId: null, threadId: null };

/** 会话页（spec 8.3）：左栏列表，右栏轨迹。从运行页等处跳来时读取一次跳转目标。 */
export function SessionsPage() {
  const { t } = useI18n();
  // Read once on mount; the store entry is cleared by the effect below, so a later mount starts clean.
  const [initial] = useState<SessionTarget | null>(() => useNavigationStore.getState().sessionTarget);
  const consumeSessionTarget = useNavigationStore((state) => state.consumeSessionTarget);
  useEffect(() => {
    consumeSessionTarget();
  }, [consumeSessionTarget]);

  const [terminal, setTerminal] = useState<TerminalFilter>(initial?.client ?? "all");
  const [search, setSearch] = useState("");
  const [model, setModel] = useState<string | null>(initial?.model ?? null);
  const [selection, setSelection] = useState<Selection>({
    sessionId: initial?.sessionId ?? null,
    stepId: initial?.stepId ?? null,
    threadId: null,
  });
  const [listWidth, setListWidth] = useState(readListWidth);
  const [pendingDelete, setPendingDelete] = useState<SessionSummary | null>(null);
  const query = useDebouncedValue(search.trim(), SEARCH_DELAY_MS);
  const status = useTrajectoryStatus();
  const remove = useDeleteSession();
  // Polls for changes while the page is open; affected lists, details and trajectories are invalidated there.
  useSessionChanges(true);

  const filter: SessionFilter = {
    client: terminal === "all" || terminal === "api" ? null : terminal,
    query: query || null,
    model,
    start_ms: null,
    end_ms: null,
  };

  function select(sessionId: string | null) {
    setSelection({ ...NO_SELECTION, sessionId });
  }
  function chooseThread(threadId: string) {
    setSelection((current) => ({ ...current, threadId, stepId: null }));
  }
  function chooseStep(stepId: string) {
    setSelection((current) => ({ ...current, stepId }));
  }
  function cancelDelete() {
    setPendingDelete(null);
    remove.reset();
  }
  function confirmDelete() {
    const target = pendingDelete;
    if (!target) return;
    remove.mutate(target.id, {
      onSuccess: () => {
        setPendingDelete(null);
        setSelection((current) => (current.sessionId === target.id ? NO_SELECTION : current));
      },
    });
  }

  return (
    <section
      className={`sessions-page ${selection.sessionId ? "has-selection" : ""} ${status.data?.available === false ? "has-unavailable" : ""}`}
      style={{ "--sessions-list-width": `${listWidth}px` } as CSSProperties}
    >
      <PageHeader
        eyebrow={t("session.page.eyebrow")}
        title={t("session.page.title")}
        subtitle={t("session.page.description")}
      />
      {status.data?.available && <TrajectoryNotice settings={status.data.settings} />}
      <div className="sessions-body">
        <div className="sessions-list-pane">
          <SessionListToolbar
            terminal={terminal}
            onTerminal={setTerminal}
            search={search}
            onSearch={setSearch}
            model={model}
            onClearModel={() => setModel(null)}
          />
          <SessionList
            filter={filter}
            apiOnly={terminal === "api"}
            selectedId={selection.sessionId}
            onSelect={select}
            onDelete={(item) => {
              remove.reset();
              setPendingDelete(item);
            }}
          />
        </div>
        <SessionsSplitter width={listWidth} onWidthChange={setListWidth} />
        <div className="sessions-detail-pane">
          <button type="button" className="sessions-back" onClick={() => select(null)}>
            <ArrowLeft size={15} aria-hidden="true" />
            {t("session.back")}
          </button>
          <SessionDetailPane
            status={status.data}
            sessionId={selection.sessionId}
            stepId={selection.stepId}
            threadId={selection.threadId}
            onThread={chooseThread}
            onStep={chooseStep}
          />
        </div>
      </div>
      {pendingDelete && (
        <ConfirmDialog
          title={t("session.deleteDialog.title")}
          description={t("session.deleteDialog.description")}
          confirmLabel={t("session.deleteDialog.confirm")}
          destructive
          busy={remove.isPending}
          error={remove.isError ? describeError(remove.error) : undefined}
          onCancel={cancelDelete}
          onConfirm={confirmDelete}
        />
      )}
    </section>
  );
}
