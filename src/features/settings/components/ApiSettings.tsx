import { useState, type ReactNode } from "react";
import { Copy, Link2, ListChecks, LoaderCircle, Route, type LucideIcon } from "lucide-react";
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

type ApiSection = "endpoints" | "models" | "fallback";

/** 一个可调用的模型：对外名、上游名与实际上游协议。 */
interface ExposedModel {
  id: string;
  exposedName: string;
  upstream: string;
  protocol: ProviderView["provider_type"];
}

/** 已启用服务商及其模型，按服务商分组展示。 */
interface ProviderGroup {
  provider: ProviderView;
  models: ExposedModel[];
}

function providerGroups(providers: ProviderView[]): ProviderGroup[] {
  return providers
    .filter((provider) => provider.enabled && provider.models.length > 0)
    .map((provider) => ({
      provider,
      models: provider.models.map((model) => ({
        id: model.id,
        exposedName: model.route_id,
        upstream: model.name,
        protocol: model.protocol ?? provider.provider_type,
      })),
    }));
}

function sameSettings(a: GlobalApiSettings, b: GlobalApiSettings) {
  return (
    a.restrict_models === b.restrict_models &&
    a.default_fallback === b.default_fallback &&
    a.model_ids.length === b.model_ids.length &&
    a.model_ids.every((id) => b.model_ids.includes(id))
  );
}

/**
 * 设置页"通用 API"分区：左侧切换接入地址、开放模型、转发规则，右侧一次只显示一块。
 * 开放模型与转发规则共用一份草稿，切换子分区不会丢失未保存的修改。
 */
export function ApiSettings() {
  const { t } = useI18n();
  const { navigate } = useNavigation();
  const listen = useProxySettings().data ?? DEFAULT_LISTEN;
  const runtime = useProxyRuntime();
  const providers = useProviders();
  const query = useApiSettings();
  const save = useSaveApiSettings();
  const [section, setSection] = useState<ApiSection>("endpoints");
  const [draft, setDraft] = useState<GlobalApiSettings | null>(null);

  const saved = query.data ?? null;
  const settings = draft ?? saved;
  const dirty = draft !== null && saved !== null && !sameSettings(draft, saved);
  const sectionDirty: Record<ApiSection, boolean> = {
    endpoints: false,
    models:
      draft !== null &&
      saved !== null &&
      !sameSettings({ ...draft, default_fallback: saved.default_fallback }, saved),
    fallback: draft !== null && saved !== null && draft.default_fallback !== saved.default_fallback,
  };
  const busy = save.isPending;

  const groups = providerGroups(providers.data ?? []);
  const all = groups.flatMap((group) => group.models);
  const disabledProviders = (providers.data ?? []).filter(
    (provider) => !provider.enabled && provider.models.length > 0,
  );
  const defaultProvider =
    (providers.data ?? []).find((provider) => provider.enabled && provider.is_default) ?? null;
  const isOpen = (model: ExposedModel) => !settings?.restrict_models || settings.model_ids.includes(model.id);
  const openCount = all.filter(isOpen).length;

  const origin = proxyOrigin(listen.host, listen.port);
  const base = proxyBaseUrl(listen.host, listen.port);
  const paths = [
    {
      id: "openai_chat",
      label: PROTOCOLS_BY_ID.openai_chat.name,
      method: "POST",
      path: "/v1/chat/completions",
    },
    { id: "responses", label: PROTOCOLS_BY_ID.responses.name, method: "POST", path: "/v1/responses" },
    { id: "anthropic", label: PROTOCOLS_BY_ID.anthropic.name, method: "POST", path: "/v1/messages" },
    { id: "models", label: t("settings.api.modelList"), method: "GET", path: "/v1/models" },
  ];
  const exampleModel = all.find(isOpen)?.exposedName ?? t("settings.api.exampleNoModel");
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

  const nav: { id: ApiSection; icon: LucideIcon; label: string; hint: string }[] = [
    {
      id: "endpoints",
      icon: Link2,
      label: t("settings.api.endpointsHeading"),
      hint: base.replace(/^https?:\/\//, ""),
    },
    {
      id: "models",
      icon: ListChecks,
      label: t("settings.api.modelsHeading"),
      hint: settings ? t("settings.api.count", { count: openCount, total: all.length }) : "",
    },
    {
      id: "fallback",
      icon: Route,
      label: t("settings.api.fallbackHeading"),
      hint: settings ? (settings.default_fallback ? t("settings.api.on") : t("settings.api.off")) : "",
    },
  ];

  const heading = (title: string, description: string) => (
    <header className="api-pane-head">
      <h2>{title}</h2>
      <p>{description}</p>
    </header>
  );

  const loading = (
    <p className="settings-note" role={loadError ? "alert" : undefined}>
      {loadError || t("settings.api.loading")}
    </p>
  );

  const saveBar = (
    <div className="api-save-bar">
      <span className={`form-message ${save.isSuccess ? "success" : ""}`} role="status">
        {status}
      </span>
      <button type="button" className="secondary-button" disabled={busy || !dirty} onClick={submit}>
        {busy && <LoaderCircle size={15} className="spinning" />}
        {busy ? t("common.saving") : t("settings.api.save")}
      </button>
    </div>
  );

  let pane: ReactNode;
  if (section === "endpoints") {
    pane = (
      <>
        {heading(t("settings.api.endpointsHeading"), t("settings.api.endpointsDescription"))}
        {runtime.data && !running && (
          <p className="api-inline-notice" role="status">
            {t("settings.api.notRunning")}
            <button type="button" className="text-button" onClick={() => navigate("proxy")}>
              {t("settings.api.openRun")}
            </button>
          </p>
        )}
        <dl className="api-fields">
          <div>
            <dt>{t("settings.api.baseUrl")}</dt>
            <dd>
              <code className="api-base-url">{base}</code>
              {copyButton(base)}
            </dd>
          </div>
          <div>
            <dt>{t("settings.api.apiKey")}</dt>
            <dd>
              <span>{t("settings.api.apiKeyValue")}</span>
            </dd>
          </div>
        </dl>
        <p className="settings-note">{t("settings.api.apiKeyNote")}</p>

        <div className="api-block">
          <h3>{t("settings.api.protocols")}</h3>
          <ul className="api-paths">
            {paths.map((item) => (
              <li key={item.id}>
                <span className="api-path-label">{item.label}</span>
                <code>
                  <span className="api-method">{item.method}</span>
                  {item.path}
                </code>
                {copyButton(`${origin}${item.path}`)}
              </li>
            ))}
          </ul>
          <p className="settings-note">
            {t("settings.api.anthropicNote", { origin })}
            <br />
            {t("settings.api.modelNote")}
          </p>
        </div>

        <details className="api-example">
          <summary>{t("settings.api.example")}</summary>
          <div className="api-example-body">
            <pre>{example}</pre>
            {copyButton(example, t("settings.api.example"))}
          </div>
        </details>
      </>
    );
  } else if (section === "models") {
    pane = (
      <>
        {heading(t("settings.api.modelsHeading"), t("settings.api.modelsDescription"))}
        {settings === null ? (
          loading
        ) : (
          <>
            <div className="api-model-toolbar">
              <div className="segmented-control" role="group" aria-label={t("settings.api.modelsHeading")}>
                <button
                  type="button"
                  aria-pressed={!settings.restrict_models}
                  disabled={busy}
                  onClick={() => setRestricted(false)}
                >
                  {t("settings.api.scopeAll")}
                </button>
                <button
                  type="button"
                  aria-pressed={settings.restrict_models}
                  disabled={busy}
                  onClick={() => setRestricted(true)}
                >
                  {t("settings.api.scopeSelected")}
                </button>
              </div>
              <span className="api-model-count">
                {settings.restrict_models && all.length > 0 && (
                  <>
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
                  </>
                )}
                {t("settings.api.count", { count: openCount, total: all.length })}
              </span>
            </div>
            {all.length === 0 ? (
              <p className="settings-note">{t("settings.api.empty")}</p>
            ) : (
              <ul className="api-model-groups" aria-label={t("settings.api.modelsHeading")}>
                {groups.map(({ provider, models }) => (
                  <li key={provider.id} className="api-model-group">
                    <div className="api-model-group-head">
                      <ProviderIcon provider={provider} size={16} />
                      <span>{provider.name}</span>
                    </div>
                    <ul>
                      {models.map((model) => {
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
                            <code>{model.exposedName}</code>
                            <span className="api-model-meta">
                              {model.upstream} · {PROTOCOLS_BY_ID[model.protocol].shortName}
                            </span>
                            {copyButton(model.exposedName)}
                          </li>
                        );
                      })}
                    </ul>
                  </li>
                ))}
              </ul>
            )}
            {settings.restrict_models && all.length > 0 && openCount === 0 && (
              <p className="trajectory-field-error">{t("settings.api.noneSelected")}</p>
            )}
            {disabledProviders.length > 0 && (
              <p className="settings-note">
                {t("settings.api.disabledNote", {
                  names: disabledProviders.map((provider) => provider.name).join("、"),
                })}
              </p>
            )}
            {saveBar}
          </>
        )}
      </>
    );
  } else {
    pane = (
      <>
        {heading(t("settings.api.fallbackHeading"), t("settings.api.fallbackDescription"))}
        {settings === null ? (
          loading
        ) : (
          <>
            <div className="api-fallback">
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
              <p className="settings-note">
                {defaultProvider
                  ? t("settings.api.fallbackTarget", { name: defaultProvider.name })
                  : t("settings.api.fallbackNone")}
              </p>
            </div>
            {saveBar}
          </>
        )}
      </>
    );
  }

  return (
    <section
      id="settings-api"
      className="settings-section api-settings"
      aria-label={t("settings.api.heading")}
    >
      <nav className="api-nav" aria-label={t("settings.api.nav")}>
        {nav.map(({ id, icon: Icon, label, hint }) => (
          <button
            key={id}
            type="button"
            className={`api-nav-item ${section === id ? "selected" : ""}`}
            aria-current={section === id ? "page" : undefined}
            onClick={() => setSection(id)}
          >
            <Icon size={15} strokeWidth={1.9} />
            <span className="api-nav-text">
              <span className="api-nav-label">
                {label}
                {sectionDirty[id] && <i className="api-nav-dirty" aria-label={t("settings.api.dirty")} />}
              </span>
              {hint && <small>{hint}</small>}
            </span>
          </button>
        ))}
      </nav>
      <div className="api-pane" id={`settings-api-${section}`}>
        {pane}
      </div>
    </section>
  );
}
