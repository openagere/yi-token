import { useState } from "react";
import { LoaderCircle } from "lucide-react";
import type { Client, TrajectorySettings as TrajectoryConfig } from "@/generated";
import { describeError } from "@/shared/api";
import { useI18n } from "@/shared/i18n";
import { notify } from "@/shared/notify";
import { ConfirmDialog, Select, type SelectOption } from "@/shared/ui";
import { formatBytes } from "@/features/proxy";
import { useClearSessions, useRebuildTrajectory, useSaveTrajectorySettings, useTrajectoryStatus } from "@/features/sessions";
import { TERMINALS } from "@/features/terminal";

const MIB = 1024 ** 2;
const GIB = 1024 ** 3;
const CAPACITY_PRESETS = [512 * MIB, GIB, 2 * GIB, 5 * GIB, 10 * GIB];
const CUSTOM = "custom";
/** 与后端一致：保留 1–3650 天；容量 64 MiB 到 1 TiB。 */
const RETENTION_RANGE = [1, 3650] as const;
const CUSTOM_MIB_RANGE = [64, 1024 * 1024] as const;
/** `api` 是全局路由（无终端客户端），由 `disabled_global` 单独控制。 */
const API_ROUTE = "api";

/** 容量与保留天数的编辑草稿；输入中允许暂时非法的文本。 */
interface LimitDraft {
  retention: string;
  choice: string;
  customMiB: string;
}

function limitsOf(settings: TrajectoryConfig): LimitDraft {
  const preset = CAPACITY_PRESETS.includes(settings.max_bytes);
  return {
    retention: String(settings.retention_days),
    choice: preset ? String(settings.max_bytes) : CUSTOM,
    customMiB: String(Math.round(settings.max_bytes / MIB)),
  };
}

/** 整数且落在范围内时返回数值，否则为 null。 */
function integerIn(value: string, [min, max]: readonly [number, number]): number | null {
  if (!/^\d+$/.test(value)) return null;
  const number = Number(value);
  return number >= min && number <= max ? number : null;
}

function capacityLabel(bytes: number): string {
  return bytes < GIB ? `${bytes / MIB} MiB` : `${bytes / GIB} GiB`;
}

interface SwitchProps {
  checked: boolean;
  label: string;
  disabled?: boolean;
  onToggle?: () => void;
}

function TrajectorySwitch({ checked, label, disabled = false, onToggle }: SwitchProps) {
  return (
    <button type="button" role="switch" aria-checked={checked} className="trajectory-switch" disabled={disabled} onClick={onToggle}>
      <span className="trajectory-switch-track" aria-hidden="true" />
      <span className="trajectory-switch-label">{label}</span>
    </button>
  );
}

/** 设置页"会话轨迹"分组：采集开关、按终端开关、保留与容量、占用、清空与重建。 */
export function TrajectorySettings() {
  const { t } = useI18n();
  const status = useTrajectoryStatus();
  // Two instances so the toggle and the limits form never share a mutation's callbacks.
  const saveToggle = useSaveTrajectorySettings();
  const saveLimits = useSaveTrajectorySettings();
  const clear = useClearSessions();
  const rebuild = useRebuildTrajectory();
  const [limitDraft, setLimitDraft] = useState<LimitDraft | null>(null);
  const [pendingClear, setPendingClear] = useState<Client | "all" | null>(null);
  const [clearChoice, setClearChoice] = useState<Client>("codex");
  const [clearResult, setClearResult] = useState("");

  const heading = (
    <div className="section-heading">
      <div>
        <h2>{t("settings.trajectory.heading")}</h2>
        <p>{t("settings.trajectory.description")}</p>
      </div>
    </div>
  );

  const data = status.data;
  const config = data?.settings ?? null;
  if (!data || !config) {
    return (
      <section id="settings-trajectory" className="editor-section" aria-label={t("settings.trajectory.heading")}>
        {heading}
        {status.isError ? (
          <p className="settings-note" role="alert">{describeError(status.error)}</p>
        ) : (
          <p className="settings-note">{t("settings.trajectory.loading")}</p>
        )}
      </section>
    );
  }

  const unavailable = !data.available;
  const draft = limitDraft ?? limitsOf(config);
  const retention = integerIn(draft.retention, RETENTION_RANGE);
  const customMiB = draft.choice === CUSTOM ? integerIn(draft.customMiB, CUSTOM_MIB_RANGE) : null;
  const maxBytes = draft.choice === CUSTOM ? (customMiB === null ? null : customMiB * MIB) : Number(draft.choice);
  const limitsDirty = limitDraft !== null && JSON.stringify(limitDraft) !== JSON.stringify(limitsOf(config));
  const limitsValid = retention !== null && maxBytes !== null;
  const togglesBusy = saveToggle.isPending;
  const limitsBusy = saveLimits.isPending;

  function toggleEnabled() {
    saveToggle.mutate({ ...config!, enabled: !config!.enabled });
  }

  function toggleClient(client: Client) {
    const disabled = config!.disabled_clients.includes(client)
      ? config!.disabled_clients.filter((item) => item !== client)
      : [...config!.disabled_clients, client];
    saveToggle.mutate({ ...config!, disabled_clients: disabled });
  }

  function toggleGlobal() {
    saveToggle.mutate({ ...config!, disabled_global: !config!.disabled_global });
  }

  function submitLimits() {
    if (limitsBusy || !limitsDirty || !limitsValid || retention === null || maxBytes === null) return;
    saveLimits.mutate(
      { ...config!, retention_days: retention, max_bytes: maxBytes },
      {
        onSuccess: () => {
          setLimitDraft(null);
          notify(t("settings.trajectory.savedToast"));
        },
      },
    );
  }

  function confirmClear() {
    if (pendingClear === null) return;
    clear.mutate(pendingClear === "all" ? null : pendingClear, {
      onSuccess: (count) => {
        setPendingClear(null);
        setClearResult(t("settings.trajectory.clearedResult", { count }));
      },
    });
  }

  function openClear(target: Client | "all") {
    clear.reset();
    setPendingClear(target);
  }

  const terminalName = (id: Client) => TERMINALS.find((item) => item.id === id)?.name ?? id;
  const clearOptions: SelectOption[] = TERMINALS.map((item) => ({ value: item.id, label: item.name }));
  const capacityOptions: SelectOption[] = [
    ...CAPACITY_PRESETS.map((bytes) => ({ value: String(bytes), label: capacityLabel(bytes) })),
    { value: CUSTOM, label: t("settings.trajectory.capacityCustom") },
  ];
  const pendingTitle =
    pendingClear === "all"
      ? t("settings.trajectory.clearAllTitle")
      : pendingClear === null
        ? ""
        : t("settings.trajectory.clearOneTitle", { terminal: terminalName(pendingClear) });
  const pendingDescription =
    pendingClear === "all"
      ? t("settings.trajectory.clearAllDescription")
      : pendingClear === null
        ? ""
        : t("settings.trajectory.clearOneDescription", { terminal: terminalName(pendingClear) });

  return (
    <section id="settings-trajectory" className="editor-section trajectory-settings" aria-label={t("settings.trajectory.heading")}>
      {heading}
      <div className="settings-group-body">
        <p className="settings-note trajectory-notice">{t("settings.trajectory.notice")}</p>

        {unavailable && (
          <div className="trajectory-store-error" role="alert">
            <strong>{t("settings.trajectory.unavailableTitle")}</strong>
            {data.error && <p>{data.error}</p>}
            <p className="settings-note">{t("settings.trajectory.rebuildNote")}</p>
            <button
              type="button"
              className="secondary-button"
              disabled={rebuild.isPending}
              onClick={() => rebuild.mutate(undefined, { onSuccess: () => notify(t("settings.trajectory.rebuiltToast")) })}
            >
              {rebuild.isPending && <LoaderCircle size={15} className="spinning" />}
              {t("settings.trajectory.rebuild")}
            </button>
          </div>
        )}

        <div className="trajectory-switch-list">
          <TrajectorySwitch checked={config.enabled} label={t("settings.trajectory.enabled")} disabled={togglesBusy} onToggle={toggleEnabled} />
        </div>

        <div className="trajectory-switch-group" role="group" aria-label={t("settings.trajectory.terminals")}>
          <span className="trajectory-group-label">{t("settings.trajectory.terminals")}</span>
          <div className="trajectory-switch-list">
            {TERMINALS.map((item) => (
              <TrajectorySwitch
                key={item.id}
                checked={!config.disabled_clients.includes(item.id as Client)}
                label={item.name}
                disabled={togglesBusy}
                onToggle={() => toggleClient(item.id as Client)}
              />
            ))}
            <TrajectorySwitch checked={!config.disabled_global} label={API_ROUTE} disabled={togglesBusy} onToggle={toggleGlobal} />
          </div>
          <p className="settings-note">{t("settings.trajectory.apiNote")}</p>
        </div>

        <fieldset disabled={limitsBusy} className="form-grid settings-grid">
          <label className="field">
            <span>{t("settings.trajectory.retention")}</span>
            <input
              type="number"
              inputMode="numeric"
              min={RETENTION_RANGE[0]}
              max={RETENTION_RANGE[1]}
              step={1}
              value={draft.retention}
              aria-invalid={retention === null}
              onChange={(event) => setLimitDraft({ ...draft, retention: event.target.value })}
            />
          </label>
          <div className="field">
            <span>{t("settings.trajectory.capacity")}</span>
            <Select
              label={t("settings.trajectory.capacity")}
              value={draft.choice}
              options={capacityOptions}
              onValueChange={(value) => setLimitDraft({ ...draft, choice: value })}
            />
          </div>
          {draft.choice === CUSTOM && (
            <label className="field">
              <span>{t("settings.trajectory.customCapacity")}</span>
              <input
                type="number"
                inputMode="numeric"
                min={CUSTOM_MIB_RANGE[0]}
                max={CUSTOM_MIB_RANGE[1]}
                step={1}
                value={draft.customMiB}
                aria-invalid={customMiB === null}
                onChange={(event) => setLimitDraft({ ...draft, customMiB: event.target.value })}
              />
            </label>
          )}
          <div className="field">
            <span>{t("settings.trajectory.usage")}</span>
            <strong className="trajectory-usage">
              {t("settings.trajectory.usageValue", { bytes: formatBytes(data.bytes), sessions: data.sessions })}
            </strong>
          </div>
        </fieldset>
        {retention === null && <p className="trajectory-field-error">{t("settings.trajectory.retentionInvalid")}</p>}
        {draft.choice === CUSTOM && customMiB === null && <p className="trajectory-field-error">{t("settings.trajectory.capacityInvalid")}</p>}
        <div className="settings-actions">
          <button type="button" className="secondary-button" disabled={!limitsDirty || !limitsValid || limitsBusy || unavailable} onClick={submitLimits}>
            {limitsBusy && <LoaderCircle size={15} className="spinning" />}
            {t("settings.trajectory.saveLimits")}
          </button>
        </div>

        <div className="trajectory-clear">
          <span className="trajectory-group-label">{t("settings.trajectory.clearHeading")}</span>
          <div className="trajectory-clear-row">
            <Select
              label={t("settings.trajectory.clearTerminal")}
              value={clearChoice}
              options={clearOptions}
              disabled={unavailable || clear.isPending}
              onValueChange={(value) => setClearChoice(value as Client)}
            />
            <button type="button" className="secondary-button danger-button" disabled={unavailable || clear.isPending} onClick={() => openClear(clearChoice)}>
              {t("settings.trajectory.clearTerminalAction")}
            </button>
            <button type="button" className="danger-button solid" disabled={unavailable || clear.isPending} onClick={() => openClear("all")}>
              {t("settings.trajectory.clearAll")}
            </button>
          </div>
          <p className="settings-note" role="status">{clearResult}</p>
        </div>
      </div>

      {pendingClear !== null && (
        <ConfirmDialog
          title={pendingTitle}
          description={pendingDescription}
          confirmLabel={pendingClear === "all" ? t("settings.trajectory.clearAll") : t("settings.trajectory.clearTerminalAction")}
          destructive
          busy={clear.isPending}
          error={clear.isError ? describeError(clear.error) : undefined}
          onConfirm={confirmClear}
          onCancel={() => setPendingClear(null)}
        />
      )}
    </section>
  );
}
