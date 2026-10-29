import { useMemo, useRef, useState } from "react";
import {
  Boxes,
  Cable,
  Cpu,
  Eye,
  EyeOff,
  FlaskConical,
  LoaderCircle,
  Plus,
  Route,
  SlidersHorizontal,
  Trash2,
} from "lucide-react";
import { describeError } from "@/shared/api";
import { useStandardModels, ModelCapabilitySummary, type StandardModel } from "@/features/models";
import { useI18n } from "@/shared/i18n";
import { companyOf, PROTOCOLS, PROTOCOLS_BY_ID, type ProviderType } from "@/shared/lib";
import { useEditorGuard, useNavigation } from "@/shared/navigation";
import { notify } from "@/shared/notify";
import {
  modelBrand,
  PageHeader,
  ProtocolSelector,
  ProviderIcon,
  providerSlug,
  routeName,
  ListPagination,
  Select,
} from "@/shared/ui";
import { providerApi } from "../api";
import { useProviders, useSaveProvider } from "../hooks";
import { ProviderModelCatalog, providerModelsUrl, type CatalogPick } from "./ProviderModelCatalog";
import { useProviderSelection } from "../selection";
import type { ModelMapping, ProviderView } from "../types";

/** 模型表每页行数：每行带能力摘要，行数过多会把编辑区撑得很长。 */
const MODEL_PAGE_SIZE = 5;

const TYPE_URLS: Record<ProviderType, string> = {
  anthropic: "https://api.anthropic.com",
  openai_chat: "https://api.openai.com/v1",
  responses: "https://api.openai.com/v1",
};

function blankProvider(isDefault: boolean): ProviderView {
  const id = crypto.randomUUID();
  return {
    id,
    provider_type: "anthropic",
    name: "",
    base_url: TYPE_URLS.anthropic,
    api_key: "",
    enabled: true,
    thinking: "medium",
    extra: {},
    is_default: isDefault,
    models: [],
  };
}

function ProviderEditorForm({
  provider,
  providers,
  standardModels,
}: {
  provider: ProviderView | null;
  providers: ProviderView[];
  standardModels: StandardModel[];
}) {
  const { t } = useI18n();
  const { back, goTo } = useNavigation();
  const saveProvider = useSaveProvider();
  const [initial] = useState(() =>
    provider ? structuredClone(provider) : blankProvider(providers.length === 0),
  );
  const [draft, setDraft] = useState<ProviderView>(() => structuredClone(initial));
  const [extraText, setExtraText] = useState(() => JSON.stringify(initial.extra, null, 2));
  const [showKey, setShowKey] = useState(false);
  const [testModel, setTestModel] = useState("");
  const [busy, setBusy] = useState<"save" | "test" | null>(null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [tab, setTab] = useState<"connection" | "models">("connection");
  const [modelPage, setModelPage] = useState(1);
  const modelPages = Math.max(1, Math.ceil(draft.models.length / MODEL_PAGE_SIZE));
  const currentModelPage = Math.min(modelPage, modelPages);
  const modelStart = (currentModelPage - 1) * MODEL_PAGE_SIZE;
  /** 新增的模型落在最后一页，便于马上编辑。 */
  function showLastModelPage(count: number) {
    setModelPage(Math.max(1, Math.ceil(count / MODEL_PAGE_SIZE)));
  }
  const form = useRef<HTMLFormElement>(null);
  const dirty = useMemo(
    () =>
      JSON.stringify(draft) !== JSON.stringify(initial) ||
      extraText !== JSON.stringify(initial.extra, null, 2),
    [draft, initial, extraText],
  );
  const currentDefault = providers.find((item) => item.is_default && item.id !== draft.id);
  useEditorGuard(dirty, busy !== null);

  function update<K extends keyof ProviderView>(key: K, value: ProviderView[K]) {
    setDraft((current) => ({ ...current, [key]: value }));
    setFieldErrors((current) => ({ ...current, [key]: "" }));
  }
  function chooseUpstreamType(type: ProviderType) {
    setDraft((current) => ({
      ...current,
      provider_type: type,
      base_url: current.base_url === TYPE_URLS[current.provider_type] ? TYPE_URLS[type] : current.base_url,
      models: current.models.map((model) => ({
        ...model,
        standard_model_id: companyMatches(model.standard_model_id, model.protocol ?? type) ? model.standard_model_id : null,
      })),
    }));
  }
  /** 模型的上游协议：自身覆盖的协议，否则跟随服务商类型。 */
  const upstreamOf = (model: ModelMapping): ProviderType => model.protocol ?? draft.provider_type;
  /** 协议所属公司的标准模型：Chat 与 Responses 共用 OpenAI 的标准模型。 */
  const standardsFor = (protocol: ProviderType) => standardModels.filter((model) => model.company === companyOf(protocol));
  const matchingStandards = standardsFor(draft.provider_type);
  function companyMatches(standardId: string | null, protocol: ProviderType) {
    const standard = standardModels.find((item) => item.id === standardId);
    return !standard || standard.company === companyOf(protocol);
  }
  function chooseModelProtocol(index: number, protocol: ProviderType | null) {
    update("models", draft.models.map((item, itemIndex) => {
      if (itemIndex !== index) return item;
      const keep = companyMatches(item.standard_model_id, protocol ?? draft.provider_type);
      return { ...item, protocol, standard_model_id: keep ? item.standard_model_id : null };
    }));
  }
  function parsedExtra() {
    const extra: unknown = JSON.parse(extraText.trim() || "{}");
    if (!extra || Array.isArray(extra) || typeof extra !== "object")
      throw new Error(t("provider.editor.errors.headersObject"));
    if (Object.values(extra).some((value) => typeof value !== "string"))
      throw new Error(t("provider.editor.errors.headersString"));
    return extra as Record<string, string>;
  }
  function cleanModels() {
    return draft.models
      .filter((model) => model.name.trim() && model.standard_model_id)
      .map((model) => ({
        ...model,
        provider_id: draft.id,
        route_id: routeName(draft.name, model.name),
        name: model.name.trim(),
      }));
  }
  function focusInvalid() {
    window.requestAnimationFrame(() => {
      const invalid = form.current?.querySelector<HTMLElement>('[aria-invalid="true"]');
      invalid?.scrollIntoView({ block: "center", behavior: "smooth" });
      invalid?.focus({ preventScroll: true });
    });
  }
  function validate(forSave: boolean) {
    const errors: Record<string, string> = {};
    if (forSave && !providerSlug(draft.name)) errors.name = t("provider.editor.errors.nameRequired");
    else if (
      forSave &&
      providers.some(
        (item) =>
          item.id !== draft.id &&
          providerSlug(item.name).toLowerCase() === providerSlug(draft.name).toLowerCase(),
      )
    )
      errors.name = t("provider.editor.errors.nameDuplicate");
    try {
      const url = new URL(draft.base_url.trim());
      if (
        !["http:", "https:"].includes(url.protocol) ||
        url.search ||
        url.hash ||
        url.username ||
        url.password
      )
        throw new Error("invalid");
    } catch {
      errors.base_url = t("provider.editor.errors.baseUrl");
    }
    try {
      parsedExtra();
    } catch (error) {
      errors.extra =
        error instanceof SyntaxError
          ? t("provider.editor.errors.headersJson")
          : error instanceof Error
            ? error.message
            : String(error);
    }
    if (forSave) {
      const models = cleanModels();
      if (
        draft.models.some(
          (model) =>
            !model.name.trim() ||
            !standardsFor(upstreamOf(model)).some((standard) => standard.id === model.standard_model_id),
        )
      )
        errors.models = t("provider.editor.errors.modelsIncomplete");
      else if (new Set(models.map((model) => model.name)).size !== models.length)
        errors.models = t("provider.editor.errors.modelsDuplicate");
      else if (!draft.is_default && models.length === 0)
        errors.models = t("provider.editor.errors.modelsRequired");
    }
    setFieldErrors(errors);
    if (Object.keys(errors).length) {
      // 错误在哪个页签，就切到哪个页签再定位。
      setTab(errors.models && Object.keys(errors).length === 1 ? "models" : "connection");
      notify(Object.values(errors)[0], "error");
      focusInvalid();
      return false;
    }
    return true;
  }
  function payload() {
    return {
      ...draft,
      name: draft.name.trim(),
      base_url: draft.base_url.trim().replace(/\/+$/, ""),
      extra: parsedExtra(),
      models: cleanModels(),
    };
  }
  function save() {
    if (busy || !validate(true)) return;
    setBusy("save");
    const next = payload();
    const creating = provider === null;
    saveProvider.mutate(next, {
      onSuccess: () => {
        const message = t(creating ? "provider.editor.added" : "provider.editor.saved", { name: next.name });
        goTo("provider-list");
        notify(message, "success");
      },
      onError: (error) => notify(t("provider.editor.saveFailed", { reason: describeError(error) }), "error"),
      onSettled: () => setBusy(null),
    });
  }
  async function testConnection() {
    if (busy || !validate(false)) return;
    const model = testModel.trim() || cleanModels()[0]?.name || "";
    if (!model) {
      setFieldErrors((current) => ({ ...current, test_model: t("provider.editor.test.missing") }));
      notify(t("provider.editor.test.missingDetail"), "error");
      form.current?.querySelector<HTMLInputElement>('[name="test_model"]')?.focus();
      return;
    }
    setBusy("test");
    try {
      const next = payload();
      const { models, ...providerInput } = next;
      const response = await providerApi.test(providerInput, models, model);
      notify(t("provider.editor.test.success", { response }));
    } catch (error) {
      notify(t("provider.editor.test.failed", { reason: describeError(error) }), "error");
    } finally {
      setBusy(null);
    }
  }
  async function discoverModels() {
    if (!validate(false)) return null;
    const next = payload();
    const { models: _models, ...providerInput } = next;
    return providerApi.discoverModels(providerInput);
  }
  function addDiscoveredModels(picks: CatalogPick[]) {
    const owned = new Set(draft.models.map((model) => model.name.trim()));
    const rows = picks.flatMap((pick) => {
      const trimmed = pick.name.trim();
      if (!trimmed || owned.has(trimmed)) return [];
      owned.add(trimmed);
      const standard = matchingStandards.find((item) => item.name === pick.standard);
      return [
        {
          id: "",
          provider_id: draft.id,
          route_id: "",
          name: trimmed,
          standard_model_id: standard?.id ?? null,
          protocol: null,
          non_standard: false,
        },
      ];
    });
    if (rows.length) {
      update("models", [...draft.models, ...rows]);
      showLastModelPage(draft.models.length + rows.length);
    }
  }
  const footer =
    busy === "test"
      ? t("provider.editor.footer.testing")
      : busy === "save"
        ? t("provider.editor.footer.saving")
        : dirty
          ? t("provider.editor.footer.unsaved")
          : provider
            ? t("provider.editor.footer.saved")
            : t("provider.editor.footer.fresh");
  const submitLabel = busy === "save" ? t("provider.editor.saving") : t("provider.editor.saveChanges");

  const modelCount = draft.models.filter((model) => model.name.trim()).length;
  const tabs = [
    {
      id: "connection" as const,
      icon: Cable,
      label: t("provider.editor.tabs.connection"),
      invalid: Boolean(fieldErrors.name || fieldErrors.base_url || fieldErrors.extra),
    },
    {
      id: "models" as const,
      icon: Boxes,
      label: t("provider.editor.tabs.models"),
      count: modelCount,
      invalid: Boolean(fieldErrors.models),
    },
  ];

  return (
    <section className="editor-page connection-editor-page provider-editor-page">
      <PageHeader
        eyebrow={provider ? provider.name : t("provider.editor.eyebrowNew")}
        title={provider ? t("provider.editor.titleEdit") : t("provider.editor.titleAdd")}
        onBack={back}
        backLabel={t("provider.editor.back")}
        disabled={Boolean(busy)}
      />
      <div className="provider-editor-tabs" role="tablist" aria-label={t("provider.editor.tabs.label")}>
        {tabs.map(({ id, icon: Icon, label, count, invalid }) => (
          <button
            key={id}
            id={`provider-tab-${id}`}
            type="button"
            role="tab"
            aria-selected={tab === id}
            aria-controls={`provider-panel-${id}`}
            className={invalid ? "invalid" : ""}
            onClick={() => setTab(id)}
          >
            <Icon size={15} aria-hidden="true" />
            <span>{label}</span>
            {count !== undefined && <em>{count}</em>}
          </button>
        ))}
      </div>
      <form
        ref={form}
        id="provider-form"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          save();
        }}
      >
        <fieldset disabled={Boolean(busy)} className="connection-form-fields">
          <div
            id="provider-panel-connection"
            role="tabpanel"
            aria-labelledby="provider-tab-connection"
            hidden={tab !== "connection"}
            className="provider-editor-panel provider-connection-panel"
          >
            <section className="provider-editor-card">
              <div className="connection-section-title">
                <span className="provider-editor-title-icon">
                  <ProviderIcon provider={draft} size={18} />
                </span>
                <h2>{t("provider.editor.info")}</h2>
              </div>
              <div className="connection-form-grid">
                <label className="field connection-name-field">
                  <span>
                    {t("provider.editor.name")}
                    <span className="required-mark">*</span>
                  </span>
                  <input
                    autoFocus
                    name="name"
                    value={draft.name}
                    aria-invalid={Boolean(fieldErrors.name)}
                    onChange={(event) => update("name", event.target.value)}
                    placeholder={t("provider.editor.namePlaceholder")}
                  />
                  {fieldErrors.name && <small className="field-error">{fieldErrors.name}</small>}
                </label>
                <div className="field connection-upstream-field">
                  <span>{t("provider.editor.upstream")}</span>
                  <ProtocolSelector
                    value={draft.provider_type}
                    onChange={chooseUpstreamType}
                    label={t("provider.editor.upstream")}
                    disabled={Boolean(busy)}
                  />
                </div>
                <label className="field full">
                  <span>
                    Base URL <span className="required-mark">*</span>
                  </span>
                  <input
                    name="base_url"
                    value={draft.base_url}
                    aria-invalid={Boolean(fieldErrors.base_url)}
                    onChange={(event) => update("base_url", event.target.value)}
                    placeholder={TYPE_URLS[draft.provider_type]}
                    spellCheck={false}
                  />
                  {fieldErrors.base_url && <small className="field-error">{fieldErrors.base_url}</small>}
                </label>
                <label className="field full">
                  <span>
                    API Key / Token <small className="optional-label">{t("provider.editor.optional")}</small>
                  </span>
                  <span className="input-action">
                    <input
                      type={showKey ? "text" : "password"}
                      value={draft.api_key}
                      onChange={(event) => update("api_key", event.target.value)}
                      placeholder={t("provider.editor.apiKey")}
                      autoComplete="new-password"
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="secret-toggle"
                      title={showKey ? t("provider.editor.hideKey") : t("provider.editor.showKey")}
                      aria-label={showKey ? t("provider.editor.hideKey") : t("provider.editor.showKey")}
                      onClick={() => setShowKey((value) => !value)}
                    >
                      {showKey ? <EyeOff size={16} /> : <Eye size={16} />}
                    </button>
                  </span>
                </label>
              </div>
              <div className="connection-options">
                <label className="toggle-row">
                  <input
                    type="checkbox"
                    checked={draft.enabled}
                    onChange={(event) => update("enabled", event.target.checked)}
                  />
                  <span className="toggle" />
                  <span>{t("provider.editor.enable")}</span>
                </label>
                <label className="connection-default-choice">
                  <input
                    type="checkbox"
                    checked={draft.is_default}
                    onChange={(event) => {
                      update("is_default", event.target.checked);
                      setFieldErrors((current) => ({ ...current, models: "" }));
                    }}
                  />
                  <span>{t("provider.editor.defaultProvider")}</span>
                </label>
                {draft.is_default && currentDefault && (
                  <small className="default-replacement">
                    {t("provider.editor.replaceDefault", { name: currentDefault.name })}
                  </small>
                )}
              </div>
            </section>
            <div className="provider-editor-side">
              <section className="provider-editor-card connection-test-section">
                <div className="connection-section-title">
                  <FlaskConical size={16} />
                  <h2>{t("provider.editor.test.title")}</h2>
                </div>
                <p className="provider-editor-hint">{t("provider.editor.test.hint")}</p>
                <div className="connection-test-controls">
                  <label className="field">
                    <span>
                      {t("provider.editor.test.model")}
                      <small className="optional-label">
                        {cleanModels()[0]?.name
                          ? t("provider.editor.optional")
                          : t("provider.editor.required")}
                      </small>
                    </span>
                    <input
                      name="test_model"
                      aria-invalid={Boolean(fieldErrors.test_model)}
                      value={testModel}
                      onChange={(event) => {
                        setTestModel(event.target.value);
                        setFieldErrors((current) => ({ ...current, test_model: "" }));
                      }}
                      placeholder={cleanModels()[0]?.name || t("provider.editor.test.modelPlaceholder")}
                      spellCheck={false}
                    />
                  </label>
                  <button type="button" className="secondary-button" onClick={() => void testConnection()}>
                    {busy === "test" && <LoaderCircle size={16} className="spinning" />}
                    {busy === "test" ? t("provider.editor.test.testing") : t("provider.editor.test.action")}
                  </button>
                </div>
                {fieldErrors.test_model && <small className="field-error">{fieldErrors.test_model}</small>}
              </section>
              <section className="provider-editor-card">
                <details className="connection-advanced" open={fieldErrors.extra ? true : undefined}>
                  <summary>
                    <SlidersHorizontal size={16} />
                    <span>{t("provider.editor.advanced")}</span>
                  </summary>
                  <div className="connection-advanced-content">
                    <label className="field">
                      <span>{t("provider.editor.headers")}</span>
                      <textarea
                        className="json-input"
                        value={extraText}
                        aria-invalid={Boolean(fieldErrors.extra)}
                        onChange={(event) => {
                          setExtraText(event.target.value);
                          setFieldErrors((current) => ({ ...current, extra: "" }));
                        }}
                        spellCheck={false}
                        placeholder={'{\n  "Header-Name": "value"\n}'}
                      />
                      {fieldErrors.extra && <small className="field-error">{fieldErrors.extra}</small>}
                    </label>
                  </div>
                </details>
              </section>
            </div>
          </div>
          <div
            id="provider-panel-models"
            role="tabpanel"
            aria-labelledby="provider-tab-models"
            hidden={tab !== "models"}
            className="provider-editor-panel provider-models-panel"
          >
            <div className="provider-models-toolbar">
              <div>
                <h2>{t("provider.editor.models.title")}</h2>
                <p>{t("provider.editor.models.hint")}</p>
              </div>
              <span className="connection-section-actions">
                <ProviderModelCatalog
                  disabled={Boolean(busy)}
                  endpoint={providerModelsUrl(draft.provider_type, draft.base_url)}
                  existing={draft.models.map((model) => model.name.trim()).filter(Boolean)}
                  standards={matchingStandards}
                  onDiscover={discoverModels}
                  onAdd={addDiscoveredModels}
                />
                <button
                  type="button"
                  className="text-button"
                  onClick={() => {
                    update("models", [
                      ...draft.models,
                      {
                        id: "",
                        provider_id: draft.id,
                        route_id: "",
                        name: "",
                        standard_model_id: null,
                        protocol: null,
                        non_standard: false,
                      },
                    ]);
                    showLastModelPage(draft.models.length + 1);
                  }}
                >
                  <Plus size={15} />
                  {t("provider.editor.models.add")}
                </button>
              </span>
            </div>
            <div
              className="connection-model-table"
              tabIndex={fieldErrors.models ? -1 : undefined}
              aria-invalid={Boolean(fieldErrors.models)}
            >
              {draft.models.length > 0 && (
                <div className="connection-model-head">
                  <span>{t("provider.editor.models.name")}</span>
                  <span>{t("provider.editor.models.protocol")}</span>
                  <span>{t("provider.editor.models.standard")}</span>
                  <span>{t("provider.editor.models.nonStandard")}</span>
                  <span />
                </div>
              )}
              {draft.models.slice(modelStart, modelStart + MODEL_PAGE_SIZE).map((model, offset) => {
                const index = modelStart + offset;
                const modelStandards = standardsFor(upstreamOf(model));
                const standard = modelStandards.find((item) => item.id === model.standard_model_id);
                return (
                  <div className="provider-model-binding" key={`${model.id}-${index}`}>
                    <div className="connection-model-row">
                      <label className="provider-model-name">
                        <input
                          aria-label={t("provider.editor.models.nameLabel", { index: index + 1 })}
                          value={model.name}
                          placeholder={draft.provider_type === "anthropic" ? "claude-sonnet-4-5" : "gpt-4.1"}
                          spellCheck={false}
                          onChange={(event) =>
                            update(
                              "models",
                              draft.models.map((item, itemIndex) =>
                                itemIndex === index ? { ...item, name: event.target.value } : item,
                              ),
                            )
                          }
                        />
                      </label>
                      <Select
                        label={t("provider.editor.models.protocolLabel", { index: index + 1 })}
                        value={model.protocol ?? ""}
                        icon={Route}
                        className="provider-model-protocol-select"
                        menuClassName="standard-model-select-menu"
                        disabled={Boolean(busy)}
                        onValueChange={(value) => chooseModelProtocol(index, (value || null) as ProviderType | null)}
                        options={[
                          {
                            value: "",
                            label: t("provider.editor.models.protocolInherit", { protocol: PROTOCOLS_BY_ID[draft.provider_type].shortName }),
                            leading: <ProviderIcon brand={PROTOCOLS_BY_ID[draft.provider_type].brand} size={18} />,
                          },
                          ...PROTOCOLS.map((item) => ({
                            value: item.id,
                            label: item.shortName,
                            leading: <ProviderIcon brand={item.brand} size={18} />,
                            description: item.name,
                          })),
                        ]}
                      />
                      <Select
                        label={t("provider.editor.models.standardLabel", { index: index + 1 })}
                        value={standard?.id ?? ""}
                        icon={Cpu}
                        className="provider-standard-model-select"
                        menuClassName="standard-model-select-menu"
                        disabled={Boolean(busy) || !modelStandards.length}
                        onValueChange={(value) =>
                          update(
                            "models",
                            draft.models.map((item, itemIndex) =>
                              itemIndex === index ? { ...item, standard_model_id: value || null } : item,
                            ),
                          )
                        }
                        options={[
                          {
                            value: "",
                            label: modelStandards.length
                              ? t("provider.editor.models.choose")
                              : t("provider.editor.models.noneForProtocol"),
                          },
                          ...modelStandards.map((item) => ({
                            value: item.id,
                            label: item.name,
                            leading: <ProviderIcon brand={item.brand || modelBrand(item.name)} size={20} />,
                          })),
                        ]}
                      />
                      <label className="toggle-row provider-model-nonstandard">
                        <input
                          type="checkbox"
                          checked={Boolean(model.non_standard)}
                          aria-label={t("provider.editor.models.nonStandardLabel", { index: index + 1 })}
                          onChange={(event) =>
                            update(
                              "models",
                              draft.models.map((item, itemIndex) =>
                                itemIndex === index ? { ...item, non_standard: event.target.checked } : item,
                              ),
                            )
                          }
                        />
                        <span className="toggle" />
                      </label>
                      <button
                        type="button"
                        className="icon-button action-danger"
                        title={t("provider.editor.models.remove")}
                        aria-label={t("provider.editor.models.removeLabel", { index: index + 1 })}
                        onClick={() =>
                          update(
                            "models",
                            draft.models.filter((_, itemIndex) => itemIndex !== index),
                          )
                        }
                      >
                        <Trash2 size={15} />
                      </button>
                    </div>
                    {standard && <ModelCapabilitySummary capabilities={standard.capabilities} />}
                  </div>
                );
              })}
              {draft.models.length === 0 && (
                <div className="connection-model-empty">{t("provider.editor.models.empty")}</div>
              )}
              {fieldErrors.models && <small className="field-error">{fieldErrors.models}</small>}
            </div>
            {draft.models.length > MODEL_PAGE_SIZE && (
              <ListPagination
                page={currentModelPage}
                count={draft.models.length}
                totalCount={draft.models.length}
                unit={t("provider.editor.models.paginationUnit")}
                label={t("provider.editor.models.paginationLabel")}
                onPage={setModelPage}
                disabled={Boolean(busy)}
                pageSize={MODEL_PAGE_SIZE}
              />
            )}
          </div>
        </fieldset>
        <footer className="connection-editor-footer">
          <span className={`draft-state ${dirty ? "unsaved" : ""}`}>
            <span />
            {footer}
          </span>
          <div>
            <button type="button" className="secondary-button" disabled={Boolean(busy)} onClick={back}>
              {t("provider.editor.cancel")}
            </button>
            <button
              type="submit"
              className="primary-button"
              disabled={Boolean(busy) || Boolean(provider && !dirty)}
            >
              {busy === "save" && <LoaderCircle size={16} className="spinning" />}
              {submitLabel}
            </button>
          </div>
        </footer>
      </form>
    </section>
  );
}

export function ProviderEditorPage() {
  const selectedId = useProviderSelection((state) => state.selectedId);
  const providers = useProviders().data ?? [];
  const standardModels = useStandardModels().data ?? [];
  const provider = providers.find((item) => item.id === selectedId) ?? null;
  return (
    <ProviderEditorForm
      key={selectedId ?? "new"}
      provider={selectedId ? provider : null}
      providers={providers}
      standardModels={standardModels}
    />
  );
}
