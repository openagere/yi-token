import { Suspense, lazy, useMemo, useState } from "react";
import { ChevronDown, ChevronUp, FileText } from "lucide-react";
import { useI18n } from "@/shared/i18n";
import { useProxyLogs } from "../hooks";
import { parseLogs } from "../logs";

const ProxyLogs = lazy(() => import("./ProxyLogs").then((module) => ({ default: module.ProxyLogs })));

/** 底部日志抽屉：默认收起；收起时也统计 WARN/ERROR，展开后才显示日志内容。 */
export function LogDrawer({ active }: { active: boolean }) {
  const { t, language } = useI18n();
  const [open, setOpen] = useState(false);
  const logsQuery = useProxyLogs({ active });
  const issues = useMemo(() => {
    const entries = parseLogs(logsQuery.data ?? "", language);
    return {
      warn: entries.filter((entry) => entry.level === "WARN").length,
      error: entries.filter((entry) => entry.level === "ERROR").length,
    };
  }, [logsQuery.data, language]);
  const hasIssues = issues.warn > 0 || issues.error > 0;

  return (
    <section className={`run-log-drawer ${open ? "open" : ""}`} aria-label={t("proxy.drawer.title")}>
      <div className="run-log-bar">
        <button
          type="button"
          className="run-log-toggle"
          aria-expanded={open}
          aria-controls="run-log-body"
          onClick={() => setOpen((value) => !value)}
        >
          <FileText size={15} />
          <strong>{t("proxy.drawer.title")}</strong>
          {hasIssues && <span className="run-issues">{t("proxy.drawer.issues", { warn: issues.warn, error: issues.error })}</span>}
        </button>
        <button
          type="button"
          className="icon-button"
          aria-label={open ? t("proxy.drawer.collapse") : t("proxy.drawer.expand")}
          title={open ? t("proxy.drawer.collapse") : t("proxy.drawer.expand")}
          onClick={() => setOpen((value) => !value)}
        >
          {open ? <ChevronDown size={16} /> : <ChevronUp size={16} />}
        </button>
      </div>
      <div id="run-log-body" className="run-log-body" hidden={!open}>
        <Suspense fallback={<div className="proxy-loading">{t("proxy.logs.loading")}</div>}>
          <ProxyLogs visible={active && open} />
        </Suspense>
      </div>
    </section>
  );
}
