import { useEffect, useMemo, useRef, useState } from "react";
import { Check, Copy } from "lucide-react";
import { type ProviderView } from "@/features/providers";
import { describeError } from "@/shared/api";
import { useI18n } from "@/shared/i18n";
import { notify } from "@/shared/notify";
import { PROTOCOLS_BY_ID } from "@/shared/lib";
import { ProviderIcon, Select } from "@/shared/ui";
import { useApplyTerminalDirectConfig, useTerminalDirectPreview } from "../hooks";
import { directCompatibleProviders, terminalName, usesProviderModels } from "../selection";
import type { TerminalClient, TerminalDirectProfile } from "../types";

interface ControllerOptions {
  client: TerminalClient;
  providers: readonly ProviderView[];
  profile: TerminalDirectProfile;
  active: boolean;
  disabled: boolean;
  onChange: (profile: TerminalDirectProfile) => void;
  onApplied: (
    profile: TerminalDirectProfile,
    result: { backup_paths: string[]; model_count: number },
  ) => void;
  onBusyChange: (busy: boolean) => void;
}

/** Keep preview, apply state and feedback together; the apply bar sits at the page bottom. */
export function useDirectTerminalController({
  client,
  providers,
  profile,
  active,
  disabled,
  onChange,
  onApplied,
  onBusyChange,
}: ControllerOptions) {
  const { t } = useI18n();
  const apply = useApplyTerminalDirectConfig();
  const [fileIndex, setFileIndex] = useState(0);
  const [feedback, setFeedback] = useState<{
    client: TerminalClient;
    text: string;
    backups: string[];
  } | null>(null);
  const compatible = useMemo(() => directCompatibleProviders(providers, client), [providers, client]);
  const selected = compatible.find((provider) => provider.id === profile.provider_id);
  const models = selected?.models.map((model) => model.name).filter(Boolean) ?? [];
  const canPreview = active && !disabled && Boolean(selected);
  const preview = useTerminalDirectPreview(profile, canPreview);
  const currentFile = preview.preview?.files[Math.min(fileIndex, (preview.preview?.files.length ?? 1) - 1)];
  const busy = apply.isPending;
  const readyToApply = canPreview && Boolean(preview.preview) && !preview.pending && !preview.error;

  useEffect(() => {
    onBusyChange(busy);
    return () => onBusyChange(false);
  }, [busy, onBusyChange]);

  function updateProvider(id: string) {
    onChange({ ...profile, provider_id: id, model: null });
    setFeedback(null);
    setFileIndex(0);
  }

  function updateSource(source: "native" | "provider") {
    onChange({ ...profile, model_source: source, model: null });
    setFeedback(null);
    setFileIndex(0);
  }

  function updateModel(model: string) {
    onChange({ ...profile, model: model || null });
    setFeedback(null);
    setFileIndex(0);
  }

  function applyConfig() {
    if (!readyToApply || busy) return;
    apply.mutate(profile, {
      onSuccess: (result) => {
        onApplied(profile, result);
        setFeedback({
          client,
          text: t("terminal.status.connected"),
          backups: result.backup_paths,
        });
        notify(t("terminal.status.connected"));
      },
      onError: (error) => notify(describeError(error), "error"),
    });
  }

  return {
    compatible,
    selected,
    models,
    preview,
    currentFile,
    fileIndex,
    setFileIndex,
    busy,
    readyToApply,
    applyConfig,
    updateProvider,
    updateSource,
    updateModel,
    success: feedback?.client === client ? feedback.text : "",
    backupPaths: feedback?.client === client ? feedback.backups : [],
    applyError: apply.isError ? describeError(apply.error) : "",
  };
}

interface Props {
  client: TerminalClient;
  profile: TerminalDirectProfile;
  disabled: boolean;
  controller: ReturnType<typeof useDirectTerminalController>;
}

export function DirectTerminalSection({ client, profile, disabled, controller }: Props) {
  const { t } = useI18n();
  const {
    compatible,
    selected,
    models,
    preview,
    currentFile,
    fileIndex,
    setFileIndex,
    busy,
    updateProvider,
    updateSource,
    updateModel,
    backupPaths,
  } = controller;
  const activeDirectModel =
    profile.model && models.includes(profile.model) ? profile.model : (models[0] ?? "");
  const clientName = terminalName(client);
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(copyTimer.current), []);

  async function copyPreview() {
    if (!currentFile) return;
    try {
      await navigator.clipboard.writeText(currentFile.content);
      window.clearTimeout(copyTimer.current);
      setCopied(true);
      copyTimer.current = window.setTimeout(() => setCopied(false), 1800);
    } catch {
      /* Clipboard unavailable. */
    }
  }

  return (
    <>
      <div className="terminal-layout terminal-direct-layout">
        <section className="terminal-config-section terminal-direct-config">
          <div className="terminal-pane-heading">
            <h2>{t("terminal.direct.title")}</h2>
            <p>{t("terminal.direct.subtitle", { client: clientName })}</p>
          </div>

          <div className="terminal-step">
            <div className="terminal-step-content terminal-direct-field">
              <span className="terminal-direct-label">
                <span className="terminal-step-index" aria-hidden="true">
                  1
                </span>
                {t("terminal.direct.provider")}
              </span>
              <Select
                label={t("terminal.direct.provider")}
                value={profile.provider_id}
                onValueChange={updateProvider}
                disabled={disabled || busy || !compatible.length}
                placeholder={
                  compatible.length ? t("terminal.direct.chooseProvider") : t("terminal.direct.noProvider")
                }
                options={compatible.map((provider) => ({
                  value: provider.id,
                  label: provider.name,
                  leading: <ProviderIcon provider={provider} size={19} />,
                  description: PROTOCOLS_BY_ID[provider.provider_type].name,
                }))}
              />
            </div>
          </div>

          {selected && (
            <div className="terminal-step terminal-step-last">
              <div className="terminal-step-content terminal-direct-field">
                <span className="terminal-direct-label">
                  <span className="terminal-step-index" aria-hidden="true">
                    2
                  </span>
                  {t("terminal.direct.modelSource")}
                </span>
                {!usesProviderModels(client) && (
                  <div
                    className="terminal-direct-options"
                    role="radiogroup"
                    aria-label={t("terminal.direct.modelSource")}
                  >
                    {(["native", "provider"] as const).map((source) => (
                      <label
                        key={source}
                        className={`terminal-direct-option${profile.model_source === source ? " selected" : ""}`}
                      >
                        <input
                          type="radio"
                          name={client + "-model-source"}
                          checked={profile.model_source === source}
                          onChange={() => updateSource(source)}
                          disabled={disabled || busy}
                        />
                        <span className="terminal-direct-option-copy">
                          <strong>
                            {t(
                              source === "native"
                                ? "terminal.direct.nativeModels"
                                : "terminal.direct.providerModels",
                            )}
                          </strong>
                          <small>
                            {t(
                              source === "native"
                                ? "terminal.direct.nativeModelsHelp"
                                : "terminal.direct.providerModelsHelp",
                            )}
                          </small>
                        </span>
                      </label>
                    ))}
                  </div>
                )}

                {profile.model_source === "provider" && (
                  <div className="terminal-direct-models-panel">
                    <div className="terminal-direct-models-head">
                      <strong>{t("terminal.direct.syncAllTitle")}</strong>
                      <span>{models.length}</span>
                      {activeDirectModel && (
                        <small>{t("terminal.direct.defaultModelNote", { model: activeDirectModel })}</small>
                      )}
                    </div>
                    <div className="terminal-direct-model-chips">
                      {models.map((model) => {
                        const isStartup = model === activeDirectModel;
                        return (
                          <button
                            type="button"
                            key={model}
                            disabled={disabled || busy}
                            aria-pressed={isStartup}
                            title={t("terminal.direct.defaultModelNote", { model })}
                            className={`terminal-direct-model-chip${isStartup ? " selected" : ""}`}
                            onClick={() => updateModel(model === models[0] ? "" : model)}
                          >
                            <code>{model}</code>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}

          {backupPaths.length > 0 && (
            <details className="terminal-backups">
              <summary>{t("terminal.feedback.backups", { count: backupPaths.length })}</summary>
              {backupPaths.map((path) => (
                <code key={path}>{path}</code>
              ))}
            </details>
          )}
        </section>

        <section className="terminal-preview-section">
          <div className="terminal-pane-heading terminal-preview-heading">
            <div>
              <h2>{t("terminal.preview.heading")}</h2>
              <p>{t("terminal.direct.previewHint")}</p>
            </div>
            <button
              type="button"
              className="icon-button copy-config"
              onClick={() => void copyPreview()}
              title={t("terminal.preview.copyTitle")}
              aria-label={t("terminal.preview.copyLabel")}
              disabled={!currentFile || preview.pending}
            >
              {copied ? <Check size={16} /> : <Copy size={16} />}
            </button>
          </div>
          <div className="terminal-preview-card">
            {preview.preview && preview.preview.files.length > 0 && (
              <div className="terminal-file-tabs" role="group" aria-label={t("terminal.preview.filesLabel")}>
                {preview.preview.files.map((file, index) => (
                  <button
                    type="button"
                    key={file.path}
                    aria-pressed={Math.min(fileIndex, preview.preview!.files.length - 1) === index}
                    onClick={() => setFileIndex(index)}
                  >
                    {file.path.split(/[\\/]/).pop()}
                  </button>
                ))}
              </div>
            )}
            <pre className="terminal-preview" aria-busy={preview.pending}>
              <code>
                {currentFile?.content ||
                  preview.error ||
                  (selected ? "" : t("terminal.direct.chooseProvider"))}
              </code>
            </pre>
          </div>
        </section>
      </div>
    </>
  );
}
