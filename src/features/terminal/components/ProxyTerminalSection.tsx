import { useState } from "react";
import {
  AlertTriangle,
  Check,
  ChevronLeft,
  ChevronRight,
  Copy,
  Layers,
  LoaderCircle,
  Search,
  X,
} from "lucide-react";
import { modalityLabel, tokenLabel, useStandardModels } from "@/features/models";
import { type ProviderType, type ProviderView } from "@/features/providers";
import { describeError } from "@/shared/api";
import { PROTOCOLS_BY_ID } from "@/shared/lib";
import { useI18n } from "@/shared/i18n";
import { notify } from "@/shared/notify";
import { ProviderIcon, RouteName, Select, modelBrand } from "@/shared/ui";
import { useSetActiveEngine } from "../hooks";
import type { TerminalActiveEngine, TerminalClient } from "../types";

interface Feedback {
  ok: boolean;
  text: string;
  backups: string[];
}

interface Props {
  client: TerminalClient;
  clientName: string;
  protocol: ProviderType;
  providers: readonly ProviderView[];
  activeEngine: TerminalActiveEngine | null;
  disabled: boolean;
  feedback: Feedback | null;
  onModelChosen: () => void;
}

const ALL_PROVIDERS = "__all__";
const PAGE_SIZE = 10;
/** Rows from this index on open their hover detail upwards so it stays inside the board. */
const UPWARD_FROM_ROW = 3;

/** Card chips favour a short reading (200K, 1M); the detail layer keeps the exact figure. */
function compactTokens(value: number) {
  return value >= 1_000_000 ? `${+(value / 1_000_000).toFixed(1)}M` : `${Math.round(value / 1000)}K`;
}

type Entry = { provider: ProviderView; model: ProviderView["models"][number] };

/** Proxy access keeps the terminal on yi-token: every saved model as a paged card grid, details on hover. */
export function ProxyTerminalSection({
  client,
  clientName,
  providers,
  activeEngine,
  disabled,
  feedback,
  onModelChosen,
}: Props) {
  const { t } = useI18n();
  const standardModels = useStandardModels();
  const [search, setSearch] = useState("");
  const [picked, setPicked] = useState<Partial<Record<TerminalClient, string>>>({});
  const [paging, setPaging] = useState({ engineKey: "", page: 1 });
  const setEngine = useSetActiveEngine();
  const query = search.trim().toLowerCase();
  const pendingId = setEngine.isPending ? setEngine.variables?.modelId : "";
  const eligible = providers.filter(
    (provider) => provider.enabled && provider.models.some((model) => model.id),
  );
  const remembered = picked[client];
  const selectedId =
    remembered && eligible.some((provider) => provider.id === remembered) ? remembered : ALL_PROVIDERS;
  const entries: Entry[] = eligible
    .filter((provider) => selectedId === ALL_PROVIDERS || provider.id === selectedId)
    .flatMap((provider) => provider.models.filter((model) => model.id).map((model) => ({ provider, model })))
    .filter(({ provider, model }) =>
      `${model.name} ${model.route_id} ${provider.name}`.toLowerCase().includes(query),
    );
  const pages = Math.max(1, Math.ceil(entries.length / PAGE_SIZE));
  const engineKey = `${client}:${activeEngine?.model_id ?? ""}`;
  if (paging.engineKey !== engineKey) {
    const index = activeEngine ? entries.findIndex((entry) => entry.model.id === activeEngine.model_id) : -1;
    if (!activeEngine || index >= 0) {
      setPaging({
        engineKey,
        page: index < 0 ? 1 : Math.floor(index / PAGE_SIZE) + 1,
      });
    }
  }
  const page = paging.engineKey === engineKey ? paging.page : 1;
  const current = Math.min(page, pages);
  const visible = entries.slice((current - 1) * PAGE_SIZE, current * PAGE_SIZE);
  const total = eligible.reduce(
    (sum, provider) => sum + provider.models.filter((model) => model.id).length,
    0,
  );
  function setPage(next: number) {
    setPaging({ engineKey, page: next });
  }

  function standardOf(id: string | null) {
    return id ? standardModels.data?.find((model) => model.id === id) : undefined;
  }

  function choose(modelId: string) {
    if (disabled || setEngine.isPending || modelId === activeEngine?.model_id) return;
    // Proxy switches apply live, so an earlier apply result no longer describes the terminal.
    onModelChosen();
    setEngine.mutate(
      { client, modelId },
      {
        onError: (reason) => notify(describeError(reason), "error"),
      },
    );
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      notify(t("terminal.proxy.copied", { name: text }));
    } catch {
      notify(t("terminal.proxy.copyFailed"), "error");
    }
  }

  return (
    <div className="terminal-proxy">
      {feedback && (
        <div className="terminal-proxy-feedback">
          <div className={`terminal-message ${feedback.ok ? "success" : "error"}`} role="status">
            {feedback.ok ? <Check size={15} /> : <AlertTriangle size={15} />}
            <span>{feedback.text}</span>
          </div>
          {!!feedback.backups.length && (
            <details className="terminal-backups">
              <summary>{t("terminal.feedback.backups", { count: feedback.backups.length })}</summary>
              {feedback.backups.map((path) => (
                <code key={path}>{path}</code>
              ))}
            </details>
          )}
        </div>
      )}

      <div className="terminal-proxy-toolbar">
        <div className="terminal-proxy-provider-field terminal-direct-field">
          <span className="terminal-direct-label">{t("terminal.proxy.provider")}</span>
          <Select
            label={t("terminal.proxy.provider")}
            value={eligible.length ? selectedId : ""}
            placeholder={t("terminal.proxy.noProvider")}
            disabled={disabled || !eligible.length}
            onValueChange={(id) => {
              setPicked((state) => ({ ...state, [client]: id }));
              setPage(1);
            }}
            options={[
              {
                value: ALL_PROVIDERS,
                label: t("terminal.proxy.allProviders"),
                leading: (
                  <span className="terminal-provider-all-icon">
                    <Layers size={13} />
                  </span>
                ),
                description: t("terminal.proxy.providerModels", { count: total }),
              },
              ...eligible.map((provider) => ({
                value: provider.id,
                label: provider.name,
                leading: <ProviderIcon provider={provider} size={19} />,
                description: `${PROTOCOLS_BY_ID[provider.provider_type].name} · ${t("terminal.proxy.providerModels", { count: provider.models.filter((model) => model.id).length })}`,
              })),
            ]}
          />
        </div>
        <div className="terminal-model-search">
          <Search size={15} aria-hidden="true" />
          <input
            type="search"
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
              setPage(1);
            }}
            placeholder={t("terminal.proxy.searchPlaceholder")}
            aria-label={t("terminal.proxy.searchPlaceholder")}
            disabled={disabled || !eligible.length}
          />
          {search && (
            <button
              type="button"
              aria-label={t("terminal.models.clearSearchLabel")}
              onClick={() => {
                setSearch("");
                setPage(1);
              }}
            >
              <X size={14} />
            </button>
          )}
        </div>
      </div>

      <div className="terminal-model-list" role="list" aria-label={t("terminal.proxy.modelsLabel")}>
        {visible.map(({ provider, model }, index) => {
          const pressed = model.id === activeEngine?.model_id;
          const pending = pendingId === model.id;
          const standard = standardOf(model.standard_model_id);
          const caps = model.capabilities ?? standard?.capabilities;
          const brand = standard?.brand || modelBrand(model.name);
          const row = Math.floor(index / 2);
          const upward = row >= UPWARD_FROM_ROW;
          const icon = (
            <span className="terminal-engine-icon">
              {brand ? (
                <ProviderIcon brand={brand} size={22} />
              ) : (
                <ProviderIcon provider={provider} size={22} />
              )}
            </span>
          );
          const label = model.route_id || model.name;
          return (
            <div
              key={`${provider.id}:${model.id}`}
              role="listitem"
              className={`terminal-engine-card${pressed ? " current" : ""}`}
            >
              <button
                type="button"
                className="terminal-engine-option"
                aria-pressed={pressed}
                aria-label={label}
                disabled={disabled || setEngine.isPending}
                onClick={() => choose(model.id)}
              >
                <span className="terminal-engine-radio" aria-hidden="true">
                  {pressed && <Check size={11} strokeWidth={3} />}
                </span>
                {icon}
                <span className="terminal-engine-copy">
                  <strong>{model.name}</strong>
                  <small>
                    <span className="terminal-engine-provider">
                      <ProviderIcon provider={provider} size={12} />
                      {provider.name}
                    </span>
                  </small>
                </span>
                {pending ? (
                  <LoaderCircle size={14} className="spinning" />
                ) : pressed ? (
                  <em className="terminal-engine-current">{t("terminal.proxy.current")}</em>
                ) : (
                  caps?.context_window != null && (
                    <em className="terminal-engine-context" title={t("terminal.proxy.detail.context")}>
                      {compactTokens(caps.context_window)}
                    </em>
                  )
                )}
              </button>

              <div
                className={`terminal-engine-detail${upward ? " upward" : ""}`}
                onClick={() => choose(model.id)}
              >
                <div className="terminal-engine-detail-head">
                  <span className="terminal-engine-radio" aria-hidden="true">
                    {pressed && <Check size={11} strokeWidth={3} />}
                  </span>
                  {icon}
                  <strong>{model.name}</strong>
                  {pressed ? (
                    <em className="terminal-engine-current">{t("terminal.proxy.current")}</em>
                  ) : (
                    <em className="terminal-engine-switch">{t("terminal.proxy.switch")}</em>
                  )}
                </div>
                <div className="terminal-engine-detail-body">
                  <dl>
                    <div>
                      <dt>{t("terminal.proxy.detail.route")}</dt>
                      <dd>
                        <RouteName route={label} />
                        <button
                          type="button"
                          className="terminal-engine-copy-button"
                          title={t("terminal.proxy.copy")}
                          aria-label={t("terminal.proxy.copy")}
                          onClick={(event) => {
                            event.stopPropagation();
                            void copy(label);
                          }}
                        >
                          <Copy size={13} />
                        </button>
                      </dd>
                    </div>
                    <div>
                      <dt>{t("terminal.proxy.detail.upstream")}</dt>
                      <dd>
                        <code>{model.name}</code>
                      </dd>
                    </div>
                    <div>
                      <dt>{t("terminal.proxy.detail.provider")}</dt>
                      <dd>
                        <ProviderIcon provider={provider} size={14} />
                        {provider.name}
                      </dd>
                    </div>
                    <div>
                      <dt>{t("terminal.proxy.detail.capability")}</dt>
                      <dd className="terminal-engine-detail-stack">
                        <span>
                          {t("terminal.proxy.detail.context")}
                          {caps?.context_window != null
                            ? `${tokenLabel(caps.context_window)} Tokens`
                            : t("terminal.proxy.detail.undeclared")}
                        </span>
                        <span>
                          {t("terminal.proxy.detail.output")}
                          {caps?.max_output_tokens != null
                            ? `${tokenLabel(caps.max_output_tokens)} Tokens`
                            : t("terminal.proxy.detail.undeclared")}
                        </span>
                        <span>
                          {t("terminal.proxy.detail.input")}
                          {caps?.input_modalities?.length
                            ? caps.input_modalities.map(modalityLabel).join("、")
                            : t("terminal.proxy.detail.undeclared")}
                        </span>
                        {caps?.effort.support === "supported" && caps.effort.levels.length > 0 && (
                          <span>
                            {t("terminal.proxy.detail.effort")}
                            {caps.effort.levels.join(" / ")}
                          </span>
                        )}
                      </dd>
                    </div>
                  </dl>
                </div>
              </div>
            </div>
          );
        })}
        {!visible.length && (
          <div className="terminal-model-empty">
            {!eligible.length
              ? t("terminal.proxy.noProvider")
              : query
                ? t("terminal.models.noMatch")
                : t("terminal.proxy.empty")}
          </div>
        )}
      </div>

      <div className="terminal-page-footer terminal-proxy-footer">
        <p className="terminal-proxy-hint">{t("terminal.proxy.subtitle", { client: clientName })}</p>
        <div className="terminal-proxy-pager">
          <span role="status" aria-live="polite">
            {entries.length
              ? `${(current - 1) * PAGE_SIZE + 1} - ${Math.min(current * PAGE_SIZE, entries.length)} / ${entries.length}`
              : "0"}{" "}
            {t("terminal.proxy.modelUnit")}
          </span>
          <nav className="list-pagination" aria-label={t("terminal.proxy.pagination")}>
            <button
              type="button"
              className="icon-button"
              title={t("ui.pagination.previous")}
              aria-label={t("ui.pagination.previous")}
              disabled={current <= 1}
              onClick={() => setPage(current - 1)}
            >
              <ChevronLeft size={16} />
            </button>
            <span aria-label={t("ui.pagination.pageOf", { current, pages })}>
              {current} / {pages}
            </span>
            <button
              type="button"
              className="icon-button"
              title={t("ui.pagination.next")}
              aria-label={t("ui.pagination.next")}
              disabled={current >= pages}
              onClick={() => setPage(current + 1)}
            >
              <ChevronRight size={16} />
            </button>
          </nav>
        </div>
      </div>
    </div>
  );
}
