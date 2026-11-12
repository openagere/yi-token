import { useState, type ReactNode } from "react";
import { Check, Copy, Layers, ListChecks, LoaderCircle } from "lucide-react";
import type { GlobalApiSettings, ProviderView } from "@/generated";
import { describeError } from "@/shared/api";
import { useI18n } from "@/shared/i18n";
import { PROTOCOLS_BY_ID, proxyBaseUrl, proxyOrigin } from "@/shared/lib";
import { useNavigation } from "@/shared/navigation";
import { notify } from "@/shared/notify";
import { ProviderIcon } from "@/shared/ui";
import { useApiSettings, useProxyRuntime, useProxySettings, useSaveApiSettings } from "@/features/proxy";
import { useProviders } from "@/features/providers";

const DEFAULT_LISTEN = { host: "127.0.0.1", port: 11435 };
/** 本地代理不校验 Key；工具要求必填时示例里用这个占位值。 */
const PLACEHOLDER_KEY = "yi-token";

/** 一个可调用的模型：所属服务商、对外名、上游名与实际上游协议。 */
interface ExposedModel {
  id: string;
  exposedName: string;
  upstream: string;
  protocol: ProviderView["provider_type"];
  provider: ProviderView;
}

function exposedModels(providers: ProviderView[]): ExposedModel[] {
  return providers
    .filter((provider) => provider.enabled)
    .flatMap((provider) =>
      provider.models.map((model) => ({
        id: model.id,
        exposedName: model.route_id,
        upstream: model.name,
        protocol: model.protocol ?? provider.provider_type,
        provider,
      })),
    );
}

function sameSettings(a: GlobalApiSettings, b: GlobalApiSettings) {
  return (
    a.restrict_models === b.restrict_models &&
    a.default_fallback === b.default_fallback &&
    a.model_ids.length === b.model_ids.length &&
    a.model_ids.every((id) => b.model_ids.includes(id))
  );
}

/** 设置页"通用 API"分区：全局入口的接入地址、开放模型与默认服务商转发；后两者共用一个保存按钮。 */
export function ApiSettings() {
  const { t } = useI18n();
  const { navigate } = useNavigation();
  const listen = useProxySettings().data ?? DEFAULT_LISTEN;
  const runtime = useProxyRuntime();
  const providers = useProviders();
  const query = useApiSettings();
  const save = useSaveApiSettings();
  const [draft, setDraft] = useState<GlobalApiSettings | null>(null);

  const saved = query.data ?? null;
  const settings = draft ?? saved;
  const dirty = draft !== null && saved !== null && !sameSettings(draft, saved);
  const busy = save.isPending;

  const all = exposedModels(providers.data ?? []);
  const disabledProviders = (providers.data ?? []).filter(
    (provider) => !provider.enabled && provider.models.length > 0,
  );
  const defaultProvider =
    (providers.data ?? []).find((provider) => provider.enabled && provider.is_default) ?? null;
  const isOpen = (model: ExposedModel) => !settings?.restrict_models || settings.model_ids.includes(model.id);
  const open = all.filter(isOpen);

  const origin = proxyOrigin(listen.host, listen.port);
  const base = proxyBaseUrl(listen.host, listen.port);
  const endpoints = [
    { id: "openai_chat", label: PROTOCOLS_BY_ID.openai_chat.name, base, url: `${base}/chat/completions` },
    { id: "responses", label: PROTOCOLS_BY_ID.responses.name, base, url: `${base}/responses` },
    // Anthropic SDKs append `/v1/messages` themselves, so their base URL is the bare origin.
    { id: "anthropic", label: PROTOCOLS_BY_ID.anthropic.name, base: origin, url: `${origin}/v1/messages` },
    { id: "models", label: t("settings.api.models"), base, url: `${base}/models` },
  ];
  const exampleModel = open[0]?.exposedName ?? t("settings.api.exampleNoModel");
  const example = [
    `curl ${base}/chat/completions \\`,
    `  -H "Content-Type: application/json" \\`,
    `  -H "Authorization: Bearer ${PLACEHOLDER_KEY}" \\`,
    `  -d '${JSON.stringify({ model: exampleModel, messages: [{ role: "user", content: "Hello" }] })}'`,
  ].join("\n");

  async function copy(text: string, name = text) {
    try {
      await navigator.clipboard.writeText(text);
      notify(t("settings.api.copied", { name }));
    } catch {
      notify(t("settings.api.copyFailed"), "error");
    }
  }

  function update(patch: Partial<GlobalApiSettings>) {
    if (settings) setDraft({ ...settings, ...patch });
  }

  function setRestricted(restrict: boolean) {
    if (!settings || settings.restrict_models === restrict) return;
    // Switching to a selection starts from what is exposed now, so nothing disappears until unchecked.
    const ids =
      restrict && settings.model_ids.length === 0 ? all.map((model) => model.id) : settings.model_ids;
    update({ restrict_models: restrict, model_ids: ids });
  }

  function toggleModel(id: string, checked: boolean) {
    if (!settings) return;
    update({
      model_ids: checked ? [...settings.model_ids, id] : settings.model_ids.filter((item) => item !== id),
    });
  }

  function submit() {
    if (!settings || busy || !dirty) return;
    save.mutate(settings, {
      onSuccess: () => {
        setDraft(null);
        notify(t("settings.api.savedToast"));
      },
    });
  }

  const copyButton = (text: string, name?: string) => (
    <button
      type="button"
      className="icon-button api-copy"
      title={t("settings.api.copy")}
      aria-label={`${t("settings.api.copy")} ${name ?? text}`}
      onClick={() => void copy(text, name)}
    >
      <Copy size={14} />
    </button>
  );

  const row = (title: string, description: string, content: ReactNode, id?: string) => (
    <section id={id} className="editor-section" aria-label={title}>
      <div className="section-heading">
        <div>
          <h2>{title}</h2>
          <p>{description}</p>
        </div>
      </div>
      <div className="settings-group-body">{content}</div>
    </section>
  );

  const loadError = query.isError
    ? describeError(query.error)
    : providers.isError
      ? describeError(providers.error)
      : "";
  const notice = save.isError ? describeError(save.error) : loadError;
  const status =
    notice ||
    (busy
      ? t("common.saving")
      : dirty
        ? t("settings.api.dirty")
        : saved
          ? t("settings.api.allSaved")
          : t("settings.api.loading"));
  const running = runtime.data?.phase === "running";

  return (
    <section
      id="settings-api"
      className="settings-section api-settings"
      aria-label={t("settings.api.heading")}
    >
      {row(
        t("settings.api.endpointsHeading"),
        t("settings.api.endpointsDescription"),
        <>
          {runtime.data && !running && (
            <div className="api-notice" role="status">
              <span>{t("settings.api.notRunning")}</span>
              <button type="button" className="text-button" onClick={() => navigate("proxy")}>
                {t("settings.api.openRun")}
              </button>
            </div>
          )}
          <div className="api-endpoints" role="table" aria-label={t("settings.api.endpointsHeading")}>
            <div className="api-endpoint-row api-endpoint-head" role="row">
              <span role="columnheader">{t("settings.api.protocolColumn")}</span>
              <span role="columnheader">{t("settings.api.baseUrl")}</span>
              <span role="columnheader">{t("settings.api.endpoint")}</span>
            </div>
            {endpoints.map((item) => (
              <div className="api-endpoint-row" role="row" key={item.id}>
                <span role="cell" className="api-endpoint-label">
                  {item.label}
                </span>
                <span role="cell" className="api-endpoint-value">
                  <code>{item.base}</code>
                  {copyButton(item.base)}
                </span>
                <span role="cell" className="api-endpoint-value">
                  <code>{item.url}</code>
                  {copyButton(item.url)}
                </span>
              </div>
            ))}
            <div className="api-endpoint-row" role="row">
              <span role="cell" className="api-endpoint-label">
                {t("settings.api.apiKey")}
              </span>
              <span role="cell" className="api-endpoint-value api-endpoint-wide">
                <span>{t("settings.api.apiKeyValue")}</span>
              </span>
            </div>
          </div>
          <p className="settings-note">{t("settings.api.apiKeyNote")}</p>
          <p className="settings-note">{t("settings.api.modelNote")}</p>
          <div className="api-example">
            <div className="api-example-head">
              <span>{t("settings.api.example")}</span>
              {copyButton(example, t("settings.api.example"))}
            </div>
            <pre>{example}</pre>
          </div>
        </>,
        "settings-api-endpoints",
      )}

      {row(
        t("settings.api.modelsHeading"),
        t("settings.api.modelsDescription"),
        settings === null ? (
          <p className="settings-note" role={loadError ? "alert" : undefined}>
            {loadError || t("settings.api.loading")}
          </p>
        ) : (
          <>
            <div className="preference-options" role="group" aria-label={t("settings.api.modelsHeading")}>
              {[
                {
                  restrict: false,
                  icon: Layers,
                  label: t("settings.api.scopeAll"),
                  desc: t("settings.api.scopeAllDesc"),
                },
                {
                  restrict: true,
                  icon: ListChecks,
                  label: t("settings.api.scopeSelected"),
                  desc: t("settings.api.scopeSelectedDesc"),
                },
              ].map(({ restrict, icon: Icon, label, desc }) => {
                const selected = settings.restrict_models === restrict;
                return (
                  <button
                    key={label}
                    type="button"
                    className={`preference-option ${selected ? "selected" : ""}`}
                    aria-pressed={selected}
                    disabled={busy}
                    onClick={() => setRestricted(restrict)}
                  >
                    <span className="preference-option-head">
                      <span className="preference-option-title">
                        <Icon size={15} strokeWidth={1.9} />
                        {label}
                      </span>
                      <span className="preference-check" aria-hidden="true">
                        {selected && <Check size={11} strokeWidth={3} />}
                      </span>
                    </span>
                    <span className="preference-option-desc">{desc}</span>
                  </button>
                );
              })}
            </div>
            {all.length === 0 ? (
              <p className="settings-note">{t("settings.api.empty")}</p>
            ) : (
              <>
                <div className="api-model-toolbar">
                  <span>{t("settings.api.count", { count: open.length, total: all.length })}</span>
                  {settings.restrict_models && (
                    <span className="api-model-bulk">
                      <button
                        type="button"
                        className="text-button"
                        disabled={busy}
                        onClick={() => update({ model_ids: all.map((model) => model.id) })}
                      >
                        {t("settings.api.selectAll")}
                      </button>
                      <button
                        type="button"
                        className="text-button"
                        disabled={busy}
                        onClick={() => update({ model_ids: [] })}
                      >
                        {t("settings.api.selectNone")}
                      </button>
                    </span>
                  )}
                </div>
                <ul className="api-model-list" aria-label={t("settings.api.modelsHeading")}>
                  {all.map((model) => {
                    const checked = isOpen(model);
                    return (
                      <li key={model.id} className={`api-model ${checked ? "" : "off"}`}>
                        {settings.restrict_models && (
                          <input
                            type="checkbox"
                            aria-label={model.exposedName}
                            checked={checked}
                            disabled={busy}
                            onChange={(event) => toggleModel(model.id, event.target.checked)}
                          />
                        )}
                        <ProviderIcon provider={model.provider} size={16} />
                        <span className="api-model-name">
                          <code title={t("settings.api.exposedName")}>{model.exposedName}</code>
                          <small>
                            {t("settings.api.upstream")} {model.upstream} ·{" "}
                            {PROTOCOLS_BY_ID[model.protocol].shortName}
                          </small>
                        </span>
                        {copyButton(model.exposedName)}
                      </li>
                    );
                  })}
                </ul>
                {settings.restrict_models && open.length === 0 && (
                  <p className="trajectory-field-error">{t("settings.api.noneSelected")}</p>
                )}
              </>
            )}
            {disabledProviders.length > 0 && (
              <p className="settings-note">
                {t("settings.api.disabledNote", {
                  names: disabledProviders.map((provider) => provider.name).join("、"),
                })}
              </p>
            )}
          </>
        ),
        "settings-api-models",
      )}

      {row(
        t("settings.api.fallbackHeading"),
        t("settings.api.fallbackDescription"),
        <>
          {settings && (
            <div className="trajectory-switch-list">
              <button
                type="button"
                role="switch"
                aria-checked={settings.default_fallback}
                className="trajectory-switch"
                disabled={busy}
                onClick={() => update({ default_fallback: !settings.default_fallback })}
              >
                <span className="trajectory-switch-track" aria-hidden="true" />
                <span className="trajectory-switch-label">{t("settings.api.fallback")}</span>
              </button>
            </div>
          )}
          <p className="settings-note">
            {defaultProvider
              ? t("settings.api.fallbackTarget", { name: defaultProvider.name })
              : t("settings.api.fallbackNone")}
          </p>
          <div className="settings-actions">
            <span className={`form-message ${save.isSuccess ? "success" : ""}`} role="status">
              {status}
            </span>
            <button type="button" className="secondary-button" disabled={busy || !dirty} onClick={submit}>
              {busy && <LoaderCircle size={15} className="spinning" />}
              {busy ? t("common.saving") : t("settings.api.save")}
            </button>
          </div>
        </>,
        "settings-api-fallback",
      )}
    </section>
  );
}
