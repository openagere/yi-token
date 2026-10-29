import { useMemo, useState } from "react";
import { CircleAlert, LoaderCircle, Plug } from "lucide-react";
import { useI18n } from "@/shared/i18n";
import { compactNumber } from "@/features/usage";
import { useNavigationStore } from "@/shared/navigation";
import { ProviderIcon } from "@/shared/ui";
import { CLIENT_META, clientName, formatClock, formatLatency, group } from "../format";
import { useLiveRequests } from "../hooks";
import type { LiveRequest, ProxyMonitorSnapshot } from "../types";

interface Row {
  item: LiveRequest;
  inFlight: boolean;
}

function isError(item: LiveRequest): boolean {
  return item.phase === "error" || (item.status !== null && item.status >= 400);
}

/** 客户端为空的请求归入 `api`（通用 API 入口）。 */
function clientKey(item: LiveRequest): string {
  return item.client ?? "api";
}

function Stage({ item, now }: { item: LiveRequest; now: number }) {
  const { t } = useI18n();
  if (item.phase === "waiting") {
    const seconds = Math.max(0, Math.floor((now - item.started_at) / 1000));
    return (
      <span className="run-phase waiting">
        <LoaderCircle size={13} className="proxy-spin" />
        {t("proxy.live.waitingFor", { seconds })}
      </span>
    );
  }
  if (item.phase === "streaming") {
    return (
      <span className="run-phase streaming">
        <span className="run-pulse" aria-hidden="true" />
        {t("proxy.live.streaming")}
      </span>
    );
  }
  if (item.phase === "error") {
    return <span className="run-phase error">{item.status ?? t("proxy.live.failed")}</span>;
  }
  return <span className="run-phase done">{item.status ?? "--"}</span>;
}

interface Props {
  active: boolean;
  monitor: ProxyMonitorSnapshot | undefined;
  monitorStale: boolean;
}

/** 请求流：终端筛选片（请求数与错误数来自监控路由）、仅错误开关，以及在途置顶的请求列表。 */
export function LiveRequests({ active, monitor, monitorStale }: Props) {
  const { t, language } = useI18n();
  const openSessions = useNavigationStore((state) => state.openSessions);
  const [client, setClient] = useState("all");
  const [errorsOnly, setErrorsOnly] = useState(false);
  const query = useLiveRequests(active);
  const live = query.data;
  // 每次轮询（约 1 秒）都会刷新该时间，计时器据此前进，渲染保持纯净。
  const now = query.dataUpdatedAt;
  const groups = useMemo(() => group(monitor?.routes ?? []), [monitor]);
  const totals = groups.reduce(
    (sum, item) => ({ requests: sum.requests + item.requests, errors: sum.errors + item.errors }),
    { requests: 0, errors: 0 },
  );
  const rows = useMemo<Row[]>(
    () =>
      live
        ? [
            ...live.inFlight.map((item) => ({ item, inFlight: true })),
            ...[...live.completed].reverse().map((item) => ({ item, inFlight: false })),
          ]
        : [],
    [live],
  );
  const visible = rows.filter(
    ({ item }) => (client === "all" || clientKey(item) === client) && (!errorsOnly || isError(item)),
  );
  const count = (value: number) => value.toLocaleString(language);
  const chipCount = (requests: number, errors: number) =>
    t("proxy.live.chipCount", { requests: count(requests), errors: count(errors) });
  const message = query.isError
    ? t("proxy.live.unavailable")
    : !live
      ? t("proxy.live.loading")
      : rows.length === 0
        ? t("proxy.live.empty")
        : t("proxy.live.emptyFiltered");

  return (
    <section className="run-section" aria-label={t("proxy.live.title")}>
      <div className="run-section-head">
        <span className="run-live-summary">
          {t("proxy.live.summary", { inFlight: live?.inFlight.length ?? 0, completed: live?.completed.length ?? 0 })}
        </span>
        <div className="run-chips" role="group" aria-label={t("proxy.live.clientFilter")}>
          <button type="button" className="run-chip" aria-pressed={client === "all"} onClick={() => setClient("all")}>
            {t("proxy.live.all")}
            <small>{monitorStale ? "--" : chipCount(totals.requests, totals.errors)}</small>
          </button>
          {groups.map((entry) => {
            const brand = CLIENT_META[entry.client]?.brand;
            return (
              <button
                type="button"
                key={entry.client}
                className="run-chip"
                aria-pressed={client === entry.client}
                onClick={() => setClient(entry.client)}
              >
                {brand ? <ProviderIcon brand={brand} size={13} /> : <Plug size={13} />}
                {clientName(entry.client, t)}
                <small>{chipCount(entry.requests, entry.errors)}</small>
              </button>
            );
          })}
        </div>
        <button
          type="button"
          className="run-errors-toggle"
          aria-pressed={errorsOnly}
          onClick={() => setErrorsOnly((value) => !value)}
        >
          <CircleAlert size={13} />
          {t("proxy.live.errorsOnly")}
        </button>
      </div>
      <div className="run-live-scroll">
        <table className="run-live-table">
          <thead>
            <tr>
              <th>{t("proxy.live.time")}</th>
              <th>{t("proxy.live.client")}</th>
              <th>{t("proxy.live.model")}</th>
              <th>{t("proxy.live.stage")}</th>
              <th>{t("proxy.live.ttft")}</th>
              <th>{t("proxy.live.duration")}</th>
              <th>{t("proxy.live.tokens")}</th>
              <th>{t("proxy.live.session")}</th>
            </tr>
          </thead>
          <tbody>
            {visible.map(({ item, inFlight }) => {
              const key = clientKey(item);
              const brand = CLIENT_META[key]?.brand;
              const end = item.finished_at ?? now;
              const session = item.session_id;
              const classes = [inFlight ? "in-flight" : "", isError(item) ? "error" : ""].filter(Boolean).join(" ");
              return (
                <tr key={item.step_id} data-step-id={item.step_id} className={classes}>
                  <td>{formatClock(item.started_at, language)}</td>
                  <td>
                    <span className="run-client">
                      {brand ? <ProviderIcon brand={brand} size={14} /> : <Plug size={14} />}
                      {clientName(key, t)}
                    </span>
                  </td>
                  <td className="run-model">
                    {item.model}
                    <small>{item.provider_name}</small>
                  </td>
                  <td>
                    <Stage item={item} now={now} />
                  </td>
                  <td>
                    {item.first_token_at !== null ? formatLatency(item.first_token_at - item.started_at) : "--"}
                  </td>
                  <td>{formatLatency(end - item.started_at)}</td>
                  <td>
                    {compactNumber(item.input_tokens, language)} / {compactNumber(item.output_tokens, language)}
                  </td>
                  <td>
                    <button
                      type="button"
                      className="run-open-session"
                      disabled={!session}
                      title={session ? t("proxy.live.openSession") : t("proxy.live.noSession")}
                      onClick={() => {
                        if (session) openSessions({ sessionId: session, stepId: item.step_id });
                      }}
                    >
                      {t("proxy.live.openSession")}
                    </button>
                  </td>
                </tr>
              );
            })}
            {visible.length === 0 && (
              <tr>
                <td colSpan={8} className="run-live-empty">
                  {message}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}
