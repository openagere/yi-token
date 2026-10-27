import { useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, LoaderCircle, RefreshCw, Search, X } from "lucide-react";
import type { DiscoveredModel } from "@/generated";
import { tokenLabel } from "@/features/models";
import { describeError } from "@/shared/api";
import { useI18n } from "@/shared/i18n";
import { classifyDiscovered } from "../modelMatch";
import type { ProviderType } from "../types";

export interface CatalogPick {
  /** 上游模型 id，即调用时使用的名称。 */
  name: string;
  /** 匹配到的标准模型名。 */
  standard: string;
}

interface Props {
  disabled: boolean;
  endpoint: string;
  existing: readonly string[];
  /** 当前上游协议下已保存的标准模型。 */
  standards: readonly { name: string }[];
  onDiscover: () => Promise<DiscoveredModel[] | null>;
  onAdd: (picks: CatalogPick[]) => void;
}

interface Row {
  model: DiscoveredModel;
  standard: string;
  score: number;
  retiring: boolean;
}

interface Filtered {
  retired: number;
  nonChat: number;
  unmatched: number;
}

/** Same path the backend appends to the provider Base URL. */
export function providerModelsUrl(providerType: ProviderType, baseUrl: string): string {
  const base = baseUrl.trim().replace(/\/+$/, "");
  const path = providerType === "anthropic" ? "/v1/models" : "/models";
  return base ? `${base}${path}` : path;
}

/** 每个标准模型默认只勾一个：最相似、非下线中、发布最新的那条。 */
function preferred(rows: readonly Row[]): Set<string> {
  const best = new Map<string, Row>();
  for (const row of rows) {
    const current = best.get(row.standard);
    const better =
      !current ||
      row.score > current.score ||
      (row.score === current.score &&
        (Number(current.retiring) - Number(row.retiring) ||
          (row.model.created ?? 0) - (current.model.created ?? 0) ||
          row.model.id.localeCompare(current.model.id)) > 0);
    if (better) best.set(row.standard, row);
  }
  return new Set([...best.values()].map((row) => row.model.id));
}

/** Fetches the provider's model catalog, keeps chat models similar to saved standard models, and adds the chosen ones. */
export function ProviderModelCatalog({ disabled, endpoint, existing, standards, onDiscover, onAdd }: Props) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [rows, setRows] = useState<Row[]>([]);
  const [filtered, setFiltered] = useState<Filtered>({ retired: 0, nonChat: 0, unmatched: 0 });
  const [picked, setPicked] = useState<string[]>([]);
  const [search, setSearch] = useState("");
  const owned = useMemo(() => new Set(existing), [existing]);
  const query = search.trim().toLowerCase();
  const visible = rows.filter(
    (row) => !query || `${row.model.id} ${row.standard}`.toLowerCase().includes(query),
  );
  const selectable = rows.filter((row) => picked.includes(row.model.id) && !owned.has(row.model.id));
  const filteredTotal = filtered.retired + filtered.nonChat + filtered.unmatched;

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
      const next: Row[] = [];
      const skipped: Filtered = { retired: 0, nonChat: 0, unmatched: 0 };
      for (const model of found) {
        const verdict = classifyDiscovered(model, standards);
        if (verdict.kind === "match")
          next.push({
            model,
            standard: verdict.standard.name,
            score: verdict.score,
            retiring: verdict.retiring,
          });
        else skipped[verdict.kind] += 1;
      }
      next.sort(
        (left, right) =>
          left.standard.localeCompare(right.standard) ||
          right.score - left.score ||
          (right.model.created ?? 0) - (left.model.created ?? 0),
      );
      const defaults = preferred(next.filter((row) => !owned.has(row.model.id)));
      setRows(next);
      setFiltered(skipped);
      setPicked(next.filter((row) => defaults.has(row.model.id)).map((row) => row.model.id));
      setSearch("");
      setOpen(true);
    } catch (reason) {
      setError(describeError(reason));
      setOpen(true);
    } finally {
      setLoading(false);
    }
  }

  function add() {
    onAdd(selectable.map((row) => ({ name: row.model.id, standard: row.standard })));
    setOpen(false);
  }

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
          <div className="app-dialog-body">
            <h2 id={titleId}>{t("provider.editor.models.fetchTitle")}</h2>
            <p>{t("provider.editor.models.fetchDescription")}</p>
            <div className="provider-model-endpoint">
              <span>{t("provider.editor.models.fetchEndpoint")}</span>
              <code>{endpoint}</code>
            </div>
            <div className="provider-model-catalog-tools">
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
              <span className="provider-model-catalog-count">
                {t("provider.editor.models.fetchCount", { shown: rows.length })}
                {filteredTotal > 0 && (
                  <small
                    title={t("provider.editor.models.fetchFilteredDetail", {
                      retired: filtered.retired,
                      nonChat: filtered.nonChat,
                      unmatched: filtered.unmatched,
                    })}
                  >
                    {t("provider.editor.models.fetchFiltered", { count: filteredTotal })}
                  </small>
                )}
              </span>
            </div>
            {filteredTotal > 0 && (
              <div className="provider-model-catalog-filters">
                {filtered.unmatched > 0 && (
                  <span>{t("provider.editor.models.fetchUnmatched", { count: filtered.unmatched })}</span>
                )}
                {filtered.nonChat > 0 && (
                  <span>{t("provider.editor.models.fetchNonChat", { count: filtered.nonChat })}</span>
                )}
                {filtered.retired > 0 && (
                  <span>{t("provider.editor.models.fetchRetired", { count: filtered.retired })}</span>
                )}
              </div>
            )}
            <div
              className="provider-model-catalog"
              role="list"
              aria-label={t("provider.editor.models.fetchTitle")}
            >
              {visible.map((row, index) => {
                const { model } = row;
                const added = owned.has(model.id);
                const checked = added || picked.includes(model.id);
                const firstOfGroup = index === 0 || visible[index - 1].standard !== row.standard;
                return (
                  <div key={model.id} role="listitem" className="provider-model-catalog-item">
                    {firstOfGroup && (
                      <div className="provider-model-catalog-group">
                        <span>{t("provider.editor.models.fetchMatched")}</span>
                        <strong>{row.standard}</strong>
                      </div>
                    )}
                    <label
                      className={`provider-model-catalog-row ${checked ? "checked" : ""} ${added ? "added" : ""}`}
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        disabled={added}
                        onChange={(event) =>
                          setPicked((current) =>
                            event.target.checked
                              ? [...current, model.id]
                              : current.filter((item) => item !== model.id),
                          )
                        }
                      />
                      <span className="provider-model-catalog-check" aria-hidden="true">
                        <Check size={12} strokeWidth={2.4} />
                      </span>
                      <span className="provider-model-catalog-name">{model.id}</span>
                      <span className="provider-model-catalog-meta">
                        {model.context_window != null && <span>{tokenLabel(model.context_window)}</span>}
                        {row.retiring && (
                          <em className="retiring">{t("provider.editor.models.fetchRetiringTag")}</em>
                        )}
                        {added && <em>{t("provider.editor.models.fetchAdded")}</em>}
                      </span>
                    </label>
                  </div>
                );
              })}
              {!loading && !visible.length && (
                <div className="provider-model-catalog-empty">
                  {error ||
                    (rows.length
                      ? t("provider.editor.models.fetchNoMatch")
                      : t("provider.editor.models.fetchEmpty"))}
                </div>
              )}
            </div>
            {error && visible.length > 0 && (
              <p className="app-dialog-error" role="alert">
                {error}
              </p>
            )}
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
