import { useState } from "react";
import { LoaderCircle } from "lucide-react";
import { describeError } from "@/shared/api";
import { useI18n } from "@/shared/i18n";
import { proxyBaseUrl } from "@/shared/lib";
import { notify } from "@/shared/notify";
import { Select } from "@/shared/ui";
import { useProxySettings, useSaveProxySettings, type ServerSettings } from "@/features/proxy";

const LOG_LEVELS: ServerSettings["log_level"][] = ["error", "warn", "info", "debug", "trace"];
const DEFAULT_SETTINGS: ServerSettings = { host: "127.0.0.1", port: 11435, log_level: "info" };

/**
 * 代理监听设置（监听地址 / 端口 / 日志级别）。
 * 位于设置页的"代理监听"分区，运行页的"去设置"直接打开该分区；通过 Tauri 命令读写，与本地偏好设置相互独立。
 */
export function ListenSettings() {
  const { t } = useI18n();
  const query = useProxySettings();
  const save = useSaveProxySettings();
  const saved = query.data ?? null;
  const [draft, setDraft] = useState<ServerSettings | null>(null);
  const settings = draft ?? saved ?? DEFAULT_SETTINGS;
  const dirty = draft !== null && saved !== null && JSON.stringify(draft) !== JSON.stringify(saved);
  const busy = save.isPending;

  function update(patch: Partial<ServerSettings>) {
    setDraft({ ...settings, ...patch });
  }

  function submit() {
    if (busy || !dirty) return;
    save.mutate(settings, {
      onSuccess: () => {
        setDraft(null);
        notify(t("proxy.listen.savedToast"));
      },
    });
  }

  const notice = save.isError ? describeError(save.error) : save.isIdle && query.isError ? describeError(query.error) : "";
  const status = notice || (busy ? t("common.saving") : dirty ? t("proxy.listen.dirty") : saved ? t("proxy.listen.allSaved") : t("proxy.listen.loading"));

  return (
    <section id="settings-listen" className="editor-section" aria-label={t("proxy.listen.heading")}>
      <div className="section-heading">
        <div>
          <h2>{t("proxy.listen.heading")}</h2>
          <p>{t("proxy.listen.description")}</p>
        </div>
      </div>
      <div className="settings-group-body">
        <div className="settings-endpoint"><span>{t("proxy.listen.endpoint")}</span><code>{proxyBaseUrl(settings.host, settings.port)}</code></div>
        <fieldset disabled={busy || saved === null} className="connection-form-fields form-grid settings-grid">
          <label className="field"><span>{t("proxy.listen.host")}</span><input value={settings.host} onChange={(event) => update({ host: event.target.value })} placeholder="127.0.0.1" /></label>
          <label className="field"><span>{t("proxy.listen.port")}</span><input type="number" min={1} max={65535} value={settings.port} onChange={(event) => update({ port: Number(event.target.value) })} /></label>
          <div className="field"><span>{t("proxy.listen.logLevel")}</span><Select label={t("proxy.listen.logLevel")} value={settings.log_level} disabled={busy || saved === null} onValueChange={(value) => update({ log_level: value as ServerSettings["log_level"] })} options={LOG_LEVELS.map((level) => ({ value: level, label: level }))} /></div>
        </fieldset>
        <p className="settings-note">{t("proxy.listen.note")}</p>
        <div className="settings-actions">
          <span className={`form-message ${save.isSuccess ? "success" : ""}`} role="status">{status}</span>
          <button type="button" className="secondary-button" disabled={busy || !dirty} onClick={submit}>
            {busy && <LoaderCircle size={15} className="spinning" />}
            {busy ? t("common.saving") : t("proxy.listen.save")}
          </button>
        </div>
      </div>
    </section>
  );
}
