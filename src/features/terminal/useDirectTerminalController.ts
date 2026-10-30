import { useEffect, useMemo, useState } from "react";
import { type ProviderView } from "@/features/providers";
import { describeError } from "@/shared/api";
import { useI18n } from "@/shared/i18n";
import { notify } from "@/shared/notify";
import { useApplyTerminalDirectConfig, useTerminalDirectPreview } from "./hooks";
import { directCompatibleProviders } from "./selection";
import type { TerminalClient, TerminalDirectProfile } from "./types";

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
