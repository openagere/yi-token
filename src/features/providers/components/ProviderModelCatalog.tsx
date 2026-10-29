import { useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, LoaderCircle, RefreshCw, Search, X } from "lucide-react";
import type { DiscoveredModel } from "@/generated";
import { tokenLabel, type StandardModel } from "@/features/models";
import { describeError } from "@/shared/api";
import { useI18n } from "@/shared/i18n";
import { modelBrand, ProviderIcon } from "@/shared/ui";
import { classifyDiscovered } from "../modelMatch";
import type { ProviderType } from "../types";

export interface CatalogPick {
  /** 上游模型 id，即调用时使用的名称。 */
  name: string;
  /** 对应的标准模型名。 */
  standard: string;
}

interface Props {
  disabled: boolean;
  endpoint: string;
  existing: readonly string[];
  /** 当前上游协议所属公司下已保存的标准模型。 */
  standards: readonly StandardModel[];
  onDiscover: () => Promise<DiscoveredModel[] | null>;
  onAdd: (picks: CatalogPick[]) => void;
}

interface MatchedRow {
  model: DiscoveredModel;
  standard: StandardModel;
  score: number;
  retiring: boolean;
}

type Tab = "all" | "matched" | "unmatched";

/** Same path the backend appends to the provider Base URL. */
export function providerModelsUrl(providerType: ProviderType, baseUrl: string): string {
  const base = baseUrl.trim().replace(/\/+$/, "");
  const path = providerType === "anthropic" ? "/v1/models" : "/models";
  return base ? `${base}${path}` : path;
}

/** 每个标准模型默认只勾一个：最相似、非下线中、发布最新的那条。 */
function preferred(rows: readonly MatchedRow[]): Set<string> {
  const best = new Map<string, MatchedRow>();
  for (const row of rows) {
    const current = best.get(row.standard.id);
    const better =
      !current ||
      row.score > current.score ||
      (row.score === current.score &&
        (Number(current.retiring) - Number(row.retiring) ||
          (row.model.created ?? 0) - (current.model.created ?? 0) ||
          row.model.id.localeCompare(current.model.id)) > 0);
    if (better) best.set(row.standard.id, row);
  }
  return new Set([...best.values()].map((row) => row.model.id));
}

/**
 * Fetches the provider's model catalog. Models similar to a saved standard model are matched
 * automatically; the rest are listed separately, where the user picks the standard model by hand.
 */
export function ProviderModelCatalog({ disabled, endpoint, existing, standards, onDiscover, onAdd }: Props) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [matched, setMatched] = useState<MatchedRow[]>([]);
  const [unmatched, setUnmatched] = useState<DiscoveredModel[]>([]);
  const [hidden, setHidden] = useState({ retired: 0, nonChat: 0 });
  const [picked, setPicked] = useState<string[]>([]);
  /** 未匹配模型手动选择的标准模型：模型 id → 标准模型 id。 */
  const [chosen, setChosen] = useState<Record<string, string>>({});
  const [tab, setTab] = useState<Tab>("all");
  const [search, setSearch] = useState("");
  const owned = useMemo(() => new Set(existing), [existing]);
  const query = search.trim().toLowerCase();
  const standardById = useMemo(() => new Map(standards.map((item) => [item.id, item])), [standards]);

  const standardOf = (modelId: string): StandardModel | undefined => {
    const row = matched.find((item) => item.model.id === modelId);
    return row ? row.standard : standardById.get(chosen[modelId] ?? "");
  };
  const visibleMatched = matched.filter(
    (row) => !query || `${row.model.id} ${row.standard.name}`.toLowerCase().includes(query),
  );
  const visibleUnmatched = unmatched.filter((model) => !query || model.id.toLowerCase().includes(query));
  const selectable = picked.filter((id) => !owned.has(id) && standardOf(id));
  const hiddenTotal = hidden.retired + hidden.nonChat;

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (open && !element.open) element.showModal();
    if (!open && element.open) element.close();
  }, [open]);

  async function load() {
    setLoading(true);
    setError("");
    try {
      const found = await onDiscover();
      if (!found) return;
      const nextMatched: MatchedRow[] = [];
      const nextUnmatched: DiscoveredModel[] = [];
      const skipped = { retired: 0, nonChat: 0 };
      for (const model of found) {
        const verdict = classifyDiscovered(model, standards);
        if (verdict.kind === "match") {
          nextMatched.push({ model, standard: verdict.standard, score: verdict.score, retiring: verdict.retiring });
        } else if (verdict.kind === "unmatched") {
          nextUnmatched.push(model);
        } else {
          skipped[verdict.kind] += 1;
        }
      }
      nextMatched.sort(
        (left, right) =>
          left.standard.name.localeCompare(right.standard.name) ||
          right.score - left.score ||
          (right.model.created ?? 0) - (left.model.created ?? 0),
      );
      const defaults = preferred(nextMatched.filter((row) => !owned.has(row.model.id)));
      setMatched(nextMatched);
      setUnmatched(nextUnmatched);
      setHidden(skipped);
      setChosen({});
      setPicked(nextMatched.filter((row) => defaults.has(row.model.id)).map((row) => row.model.id));
      setSearch("");
      setTab("all");
      setOpen(true);
    } catch (reason) {
      setError(describeError(reason));
      setOpen(true);
    } finally {
      setLoading(false);
    }
  }

  /** 为未匹配的模型选择标准模型；选中即勾选，清空则取消勾选。 */
  function chooseStandard(modelId: string, standardId: string) {
    setChosen((current) => ({ ...current, [modelId]: standardId }));
    setPicked((current) => {
      const without = current.filter((id) => id !== modelId);
      return standardId ? [...without, modelId] : without;
    });
  }

  function togglePick(modelId: string, checked: boolean) {
    setPicked((current) => (checked ? [...current, modelId] : current.filter((id) => id !== modelId)));
  }

  function add() {
    onAdd(
      selectable.map((id) => ({ name: id, standard: standardOf(id)?.name ?? "" })),
    );
    setOpen(false);
  }

  const tabs: { id: Tab; label: string; count: number }[] = [
    { id: "all", label: t("provider.editor.models.fetchTabAll"), count: matched.length + unmatched.length },
    { id: "matched", label: t("provider.editor.models.fetchTabMatched"), count: matched.length },
    { id: "unmatched", label: t("provider.editor.models.fetchTabUnmatched"), count: unmatched.length },
  ];
  const showMatched = tab !== "unmatched" && visibleMatched.length > 0;
  const showUnmatched = tab !== "matched" && visibleUnmatched.length > 0;

  return (
    <>
      <button
        type="button"
        className="text-button"
        disabled={disabled || loading}
        onClick={() => void load()}
      >
        {loading ? <LoaderCircle size={15} className="spinning" /> : <RefreshCw size={15} />}
        {loading ? t("provider.editor.models.fetching") : t("provider.editor.models.fetch")}
      </button>
      {createPortal(
        <dialog
          ref={dialog}
          className="app-dialog provider-model-catalog-dialog"
          aria-labelledby={titleId}
          onCancel={(event) => {
            event.preventDefault();
            if (!loading) setOpen(false);
          }}
          onClick={(event) => {
            if (event.target === event.currentTarget && !loading) setOpen(false);
          }}
        >
          <div className="app-dialog-body provider-model-catalog-body">
            <header className="provider-model-catalog-head">
              <h2 id={titleId}>{t("provider.editor.models.fetchTitle")}</h2>
              <p>{t("provider.editor.models.fetchDescription")}</p>
              <div className="provider-model-endpoint">
                <span>{t("provider.editor.models.fetchEndpoint")}</span>
                <code>{endpoint}</code>
              </div>
            </header>

            <div className="provider-model-catalog-toolbar">
              <div className="provider-model-catalog-search">
                <Search size={15} aria-hidden="true" />
                <input
                  type="search"
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  placeholder={t("provider.editor.models.fetchSearch")}
                  aria-label={t("provider.editor.models.fetchSearch")}
                />
                {search && (
                  <button
                    type="button"
                    aria-label={t("provider.editor.models.fetchClear")}
                    onClick={() => setSearch("")}
                  >
                    <X size={14} />
                  </button>
                )}
              </div>
              <div className="provider-model-catalog-tabs" role="tablist" aria-label={t("provider.editor.models.fetchTitle")}>
                {tabs.map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    role="tab"
                    aria-selected={tab === item.id}
                    className={tab === item.id ? "selected" : ""}
                    onClick={() => setTab(item.id)}
                  >
                    {item.label}
                    <span>{item.count}</span>
                  </button>
                ))}
              </div>
            </div>

            <div className="provider-model-catalog" role="list" aria-label={t("provider.editor.models.fetchTitle")}>
              {showMatched && (
                <section className="provider-model-catalog-section">
                  <h3>{t("provider.editor.models.fetchMatchedGroup")}</h3>
                  {visibleMatched.map(({ model, standard, retiring }) => {
                    const added = owned.has(model.id);
                    const checked = added || picked.includes(model.id);
                    return (
                      <div key={model.id} role="listitem">
                        <label className={`provider-model-catalog-row ${checked ? "checked" : ""} ${added ? "added" : ""}`}>
                          <input
                            type="checkbox"
                            checked={checked}
                            disabled={added}
                            onChange={(event) => togglePick(model.id, event.target.checked)}
                          />
                          <span className="provider-model-catalog-check" aria-hidden="true">
                            <Check size={12} strokeWidth={2.4} />
                          </span>
                          <span className="provider-model-catalog-name">{model.id}</span>
                          <span className="provider-model-catalog-standard">
                            <ProviderIcon brand={standard.brand || modelBrand(standard.name)} size={14} />
                            {standard.name}
                          </span>
                          <span className="provider-model-catalog-meta">
                            {model.context_window != null && <span>{tokenLabel(model.context_window)}</span>}
                            {retiring && <em className="retiring">{t("provider.editor.models.fetchRetiringTag")}</em>}
                            {added && <em>{t("provider.editor.models.fetchAdded")}</em>}
                          </span>
                        </label>
                      </div>
                    );
                  })}
                </section>
              )}

              {showUnmatched && (
                <section className="provider-model-catalog-section">
                  <h3>{t("provider.editor.models.fetchUnmatchedGroup")}</h3>
                  {visibleUnmatched.map((model) => {
                    const added = owned.has(model.id);
                    const standardId = chosen[model.id] ?? "";
                    const checked = added || (picked.includes(model.id) && Boolean(standardId));
                    return (
                      <div key={model.id} role="listitem" className={`provider-model-catalog-row unmatched ${checked ? "checked" : ""} ${added ? "added" : ""}`}>
                        <label className="provider-model-catalog-pick">
                          <input
                            type="checkbox"
                            checked={checked}
                            disabled={added || !standardId}
                            onChange={(event) => togglePick(model.id, event.target.checked)}
                          />
                          <span className="provider-model-catalog-check" aria-hidden="true">
                            <Check size={12} strokeWidth={2.4} />
                          </span>
                          <span className="provider-model-catalog-name">{model.id}</span>
                          {added && <em>{t("provider.editor.models.fetchAdded")}</em>}
                        </label>
                        {/* Native select: a portaled menu would render beneath the modal dialog. */}
                        <select
                          className="provider-model-catalog-select"
                          aria-label={t("provider.editor.models.fetchPickLabel", { name: model.id })}
                          value={standardId}
                          disabled={added}
                          onChange={(event) => chooseStandard(model.id, event.target.value)}
                        >
                          <option value="">{t("provider.editor.models.fetchPickStandard")}</option>
                          {standards.map((item) => (
                            <option key={item.id} value={item.id}>{item.name}</option>
                          ))}
                        </select>
                      </div>
                    );
                  })}
                </section>
              )}

              {!loading && !showMatched && !showUnmatched && (
                <div className="provider-model-catalog-empty">
                  {error ||
                    (matched.length || unmatched.length
                      ? t("provider.editor.models.fetchNoMatch")
                      : t("provider.editor.models.fetchEmpty"))}
                </div>
              )}
            </div>

            <div className="provider-model-catalog-foot">
              <span>{t("provider.editor.models.fetchCount", { shown: matched.length + unmatched.length })}</span>
              {hiddenTotal > 0 && (
                <small>
                  {[
                    hidden.nonChat > 0 && t("provider.editor.models.fetchNonChat", { count: hidden.nonChat }),
                    hidden.retired > 0 && t("provider.editor.models.fetchRetired", { count: hidden.retired }),
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </small>
              )}
            </div>
            {error && (matched.length || unmatched.length) ? (
              <p className="app-dialog-error" role="alert">
                {error}
              </p>
            ) : null}
            <div className="app-dialog-actions">
              <button type="button" className="secondary-button" onClick={() => setOpen(false)}>
                {t("ui.dialog.cancel")}
              </button>
              <button type="button" className="primary-button" disabled={!selectable.length} onClick={add}>
                {t("provider.editor.models.fetchAdd", { count: selectable.length })}
              </button>
            </div>
          </div>
        </dialog>,
        document.body,
      )}
    </>
  );
}
