import { Activity, ArrowRight, CircleAlert, LoaderCircle, RefreshCw, RotateCw, Workflow } from "lucide-react";
import "@/styles/proxy.css";
import { describeError } from "@/shared/api";
import { useProviders } from "@/features/providers";
import { useI18n } from "@/shared/i18n";
import { useNavigation } from "@/shared/navigation";
import { PageHeader } from "@/shared/ui";
import { formatListenAddress, formatUptime } from "../format";
import { useProxyControl, useProxyMonitor, useProxyRuntime } from "../hooks";
import { LiveRequests } from "./LiveRequests";
import { LogDrawer } from "./LogDrawer";
import { TrafficStrip } from "./TrafficStrip";

/** 流量与终端筛选的监控窗口；终端路由按代理启动以来累计，与此窗口无关。 */
const MONITOR_MINUTES = 15;

/** 监听分组可能晚于设置页挂载，因此短暂重试查找锚点。 */
function scrollToListenGroup(attempt = 0) {
  const target = document.getElementById("settings-listen");
  if (target) target.scrollIntoView({ block: "start" });
  else if (attempt < 20) window.setTimeout(() => scrollToListenGroup(attempt + 1), 50);
}

/** 运行页（spec 8.2）：状态条、流量、请求流与底部日志抽屉。只有可见时才轮询。 */
export function RunPage({ active }: { active: boolean }) {
  const { t } = useI18n();
  const { back, navigate } = useNavigation();
  const runtimeQuery = useProxyRuntime();
  const { control, busy } = useProxyControl();
  const providersQuery = useProviders();
  const monitorQuery = useProxyMonitor(MONITOR_MINUTES, { active });
  const runtime = runtimeQuery.data;
  const error = runtimeQuery.isError ? describeError(runtimeQuery.error) : "";
  const phase = error ? null : runtime?.phase;
  const running = phase === "running";
  const transitioning = phase === "starting" || phase === "stopping";
  const providers = providersQuery.data ?? [];
  const enabled = providers.filter((provider) => provider.enabled);
  const statusLabel = phase ? t(`nav.phase.${phase}`) : error ? t("proxy.statusUnavailable") : t("proxy.statusConnecting");
  const refreshing = runtimeQuery.isFetching;
  const known = runtime && !error;

  function openListenSettings() {
    navigate("settings");
    window.setTimeout(() => scrollToListenGroup(), 0);
  }

  return (
    <section className="proxy-page proxy-workspace run-page">
      <PageHeader
        navigationInToolbar
        eyebrow={t("proxy.eyebrow")}
        title={t("proxy.title")}
        subtitle={t("pageDescriptions.proxy")}
        onBack={back}
        backLabel={t("common.back")}
        actions={
          <button
            type="button"
            className="icon-button"
            title={t("proxy.refresh")}
            aria-label={t("proxy.refresh")}
            disabled={refreshing}
            onClick={() => void runtimeQuery.refetch()}
          >
            <RefreshCw size={16} className={refreshing ? "proxy-spin" : ""} />
          </button>
        }
      />
      <section className="proxy-runtime" aria-label={t("proxy.title")}>
        <div className="proxy-runtime-main">
          <div className={`proxy-state-symbol ${phase ?? "unknown"}`}>
            {transitioning || (!runtime && !error) ? (
              <LoaderCircle size={21} className="proxy-spin" />
            ) : phase === "error" || error ? (
              <CircleAlert size={21} />
            ) : (
              <Activity size={21} />
            )}
          </div>
          <div className="proxy-state-copy">
            <span>{t("proxy.localProxy")}</span>
            <strong role="status">{statusLabel}</strong>
          </div>
          <div className="proxy-runtime-actions">
            {running && (
              <button
                type="button"
                className="icon-button"
                title={t("proxy.restart")}
                aria-label={t("proxy.restart")}
                disabled={!!busy || transitioning}
                onClick={() => control("restart")}
              >
                <RotateCw size={16} />
              </button>
            )}
            <button
              type="button"
              className={running ? "secondary-button" : "primary-button"}
              disabled={!!busy || transitioning || !runtime || !!error}
              onClick={() => control(running ? "stop" : "start")}
            >
              {(busy || transitioning) && <LoaderCircle size={15} className="proxy-spin" />}
              {phase === "stopping" ? t("proxy.stopping") : phase === "starting" ? t("proxy.starting") : running ? t("proxy.stop") : t("proxy.start")}
            </button>
          </div>
        </div>
        <dl className="proxy-runtime-facts proxy-monitor-facts run-status-facts">
          <div>
            <dt>{t("proxy.facts.listen")}</dt>
            <dd>
              <code>{runtime?.address ?? (runtime ? formatListenAddress(runtime.settings) : "--")}</code>
            </dd>
          </div>
          <div>
            <dt>{t("proxy.facts.uptime")}</dt>
            <dd>{known ? formatUptime(runtime.started_at, runtimeQuery.dataUpdatedAt, t) : "--"}</dd>
          </div>
          <div>
            <dt>{t("proxy.run.facts.connections")}</dt>
            <dd>
              {known ? runtime.active_connections : "--"}
              <small>{t("proxy.facts.tcp")}</small>
            </dd>
          </div>
          <div>
            <dt>{t("proxy.run.facts.inFlight")}</dt>
            <dd>{known ? runtime.active_requests : "--"}</dd>
          </div>
        </dl>
      </section>
      {(error || runtime?.last_error) && (
        <div className="proxy-alert error" role="alert">
          <CircleAlert size={17} />
          <div>
            <strong>{error ? t("proxy.alerts.statusFailed") : t("proxy.alerts.startFailed")}</strong>
            <p>{error || runtime?.last_error}</p>
          </div>
          {error ? (
            <button type="button" className="text-button" onClick={() => void runtimeQuery.refetch()}>
              {t("common.retry")}
              <ArrowRight size={14} />
            </button>
          ) : (
            <button type="button" className="text-button" onClick={openListenSettings}>
              {t("proxy.run.goSettings")}
              <ArrowRight size={14} />
            </button>
          )}
        </div>
      )}
      {!providersQuery.isPending && !providersQuery.isError && enabled.length === 0 && (
        <div className="proxy-alert">
          <Workflow size={17} />
          <div>
            <strong>{t("proxy.alerts.noProviders")}</strong>
          </div>
          <button type="button" className="text-button" onClick={() => navigate("provider-list")}>
            {t("proxy.alerts.manageProviders")}
            <ArrowRight size={14} />
          </button>
        </div>
      )}
      {phase === "stopped" && (
        <div className="proxy-stopped-note">
          <span className="proxy-status-dot stopped" />
          {t("proxy.alerts.stoppedNote")}
        </div>
      )}
      <TrafficStrip snapshot={monitorQuery.data} stale={monitorQuery.isError} />
      <LiveRequests active={active} monitor={monitorQuery.data} monitorStale={monitorQuery.isError} />
      <LogDrawer active={active} />
    </section>
  );
}
