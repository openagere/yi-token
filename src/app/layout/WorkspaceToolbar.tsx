import { useEffect, useState, type MouseEvent } from "react";
import { Copy, Minus, PanelLeft, Square, X } from "lucide-react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useI18n } from "@/shared/i18n";
import { detectPlatform } from "@/shared/lib";
import { useNavigationStore } from "@/shared/navigation";
import { notify } from "@/shared/notify";

export function WorkspaceToolbar() {
  const { t } = useI18n();
  const disabled = useNavigationStore((state) => state.busy);
  const collapsed = useNavigationStore((state) => state.sidebarCollapsed);
  const toggleSidebar = useNavigationStore((state) => state.toggleSidebar);
  const guarded = useNavigationStore((state) => state.guarded);
  const native = isTauri();
  // macOS 使用系统装饰窗口（Overlay 标题栏），窗口控制由原生红绿灯按钮提供，无需自绘。
  const systemTitlebar = native && detectPlatform() === "macos";
  const [maximized, setMaximized] = useState(false);
  const title = t("nav.workspace");
  useEffect(() => {
    document.title = title;
    if (native) void getCurrentWindow().setTitle(title).catch(error => console.warn("Unable to synchronize window title", error));
  }, [native, title]);

  // 原生红绿灯关闭不经过自绘按钮，这里拦截系统关闭请求，沿用未保存修改的确认流程。
  useEffect(() => {
    if (!systemTitlebar) return;
    let active = true;
    let unlisten: (() => void) | undefined;
    void getCurrentWindow().onCloseRequested(event => {
      event.preventDefault();
      guarded(() => { void getCurrentWindow().destroy().catch(error => notify(`${t("window.failed")}${String(error)}`, "error")); });
    }).then(stop => { if (active) unlisten = stop; else stop(); }).catch(() => {});
    return () => { active = false; unlisten?.(); };
  }, [systemTitlebar, guarded, t]);

  useEffect(() => {
    if (!native || systemTitlebar) return;
    let active = true;
    const sync = () => { void getCurrentWindow().isMaximized().then(value => { if (active) setMaximized(value); }).catch(() => {}); };
    sync();
    window.addEventListener("resize", sync);
    return () => { active = false; window.removeEventListener("resize", sync); };
  }, [native, systemTitlebar]);

  async function control(action: "minimize" | "maximize" | "drag") {
    if (!native) return;
    try {
      const appWindow = getCurrentWindow();
      if (action === "minimize") await appWindow.minimize();
      else if (action === "drag") await appWindow.startDragging();
      else { await appWindow.toggleMaximize(); setMaximized(await appWindow.isMaximized()); }
    } catch (error) { notify(`${t("window.failed")}${String(error)}`, "error"); }
  }
  function closeWindow() {
    if (!native) return;
    guarded(() => { void getCurrentWindow().close().catch(error => notify(`${t("window.failed")}${String(error)}`, "error")); });
  }
  function interactive(event: MouseEvent) {
    return event.target instanceof Element && !!event.target.closest("[data-window-interactive]");
  }
  return <header className={`workspace-toolbar ${native ? "native-titlebar" : ""}`} aria-label={t("window.titlebar")}
    onMouseDown={event => { if (event.button === 0 && event.detail === 1 && !interactive(event)) void control("drag"); }}
    onDoubleClick={event => { if (event.button === 0 && !interactive(event)) void control("maximize"); }}>
    <div className="workspace-toolbar-navigation">
      <button type="button" className="icon-button" data-window-interactive title={t("nav.toggleSidebar")} aria-label={t("nav.toggleSidebar")} aria-controls="workspace-sidebar" aria-expanded={!collapsed} onClick={toggleSidebar}><PanelLeft size={15} /></button>
    </div>
    <div className="titlebar-caption" data-titlebar-drag-region />
    <div className="titlebar-actions" data-window-interactive>
      {native && !systemTitlebar && <div className="window-controls">
        <button type="button" className="window-control" title={t("window.minimize")} aria-label={t("window.minimize")} onClick={() => void control("minimize")}><Minus size={14} strokeWidth={1.4} /></button>
        <button type="button" className="window-control" title={t(maximized ? "window.restore" : "window.maximize")} aria-label={t(maximized ? "window.restore" : "window.maximize")} onClick={() => void control("maximize")}>{maximized ? <Copy size={12} strokeWidth={1.4} /> : <Square size={12} strokeWidth={1.4} />}</button>
        <button type="button" className="window-control window-control-close" title={t("window.close")} aria-label={t("window.close")} disabled={disabled} onClick={closeWindow}><X size={15} strokeWidth={1.4} /></button>
      </div>}
    </div>
  </header>;
}