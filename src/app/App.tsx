import { useEffect, useRef } from "react";
import { useI18n } from "@/shared/i18n";
import { isCompactWindow, useNavigationStore } from "@/shared/navigation";
import { ConfirmDialog, Toast } from "@/shared/ui";
import { Sidebar } from "./layout/Sidebar";
import { SidebarResizer } from "./layout/SidebarResizer";
import { WorkspaceToolbar } from "./layout/WorkspaceToolbar";
import { PageOutlet } from "./routes";

/** Re-evaluates the compact layout when the window resizes (the sidebar width is read at the same time). */
function useCompactSidebar() {
  const setCompactLayout = useNavigationStore((state) => state.setCompactLayout);
  useEffect(() => {
    const update = () => setCompactLayout(isCompactWindow());
    update();
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, [setCompactLayout]);
}

function DiscardDialog() {
  const { t } = useI18n();
  const pending = useNavigationStore((state) => state.pendingDiscard);
  const confirm = useNavigationStore((state) => state.confirmDiscard);
  const cancel = useNavigationStore((state) => state.cancelDiscard);
  if (!pending) return null;
  return <ConfirmDialog title={t("nav.discard.title")} description={t("nav.discard.description")} confirmLabel={t("nav.discard.confirm")} onCancel={cancel} onConfirm={confirm} />;
}

export default function App() {
  const { language, t } = useI18n();
  const page = useNavigationStore((state) => state.page);
  const collapsed = useNavigationStore((state) => state.sidebarCollapsed);
  const compact = useNavigationStore((state) => state.compactLayout);
  const setSidebarCollapsed = useNavigationStore((state) => state.setSidebarCollapsed);
  const mainPanel = useRef<HTMLDivElement>(null);

  useCompactSidebar();
  useEffect(() => { mainPanel.current?.scrollTo({ top: 0 }); }, [page]);

  return (
    <main className={`app-shell ${collapsed ? "sidebar-collapsed" : ""} ${compact ? "compact-layout" : ""}`} data-language={language}>
      <WorkspaceToolbar />
      <div className="workspace-body">
        {compact && !collapsed && <button type="button" className="sidebar-backdrop" aria-label={t("nav.closeSidebar")} onClick={() => setSidebarCollapsed(true)} />}
        <Sidebar />
        {!collapsed && !compact && <SidebarResizer />}
        <div className="main-panel" id="workspace-content" ref={mainPanel}>
          <PageOutlet />
        </div>
      </div>
      <Toast />
      <DiscardDialog />
    </main>
  );
}
