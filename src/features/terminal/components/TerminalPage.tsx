import { useCallback, useMemo, useState } from "react";
import { AlertTriangle, Cable, LoaderCircle, Network, Plug, Unplug } from "lucide-react";
import "@/styles/terminal.css";
import { useProviders, type ProviderView } from "@/features/providers";
import { describeError } from "@/shared/api";
import { useI18n } from "@/shared/i18n";
import { PROTOCOLS_BY_ID, type ProviderType } from "@/shared/lib";
import { TERMINAL_PAGE_CLIENTS, useEditorGuard, useNavigationStore } from "@/shared/navigation";
import { notify } from "@/shared/notify";
import { ProviderIcon, RouteName } from "@/shared/ui";
import {
  useActiveEngines,
  useApplyTerminalConfig,
  useDisconnectTerminal,
  useTerminalProfiles,
} from "../hooks";
import {
  AUTO_MODEL,
  TERMINALS,
  buildProfile,
  directCompatibleProviders,
  draftProtocol,
  emptyDirectProfile,
  protocolFor,
} from "../selection";
import type { TerminalClient } from "../types";
import { useTerminalDirectDraft, useTerminalDrafts, useTerminalPendingClients } from "../useTerminalDrafts";
import { DirectTerminalSection, useDirectTerminalController } from "./DirectTerminalSection";
import { ProxyTerminalSection } from "./ProxyTerminalSection";

const EMPTY_PROVIDERS: ProviderView[] = [];
type AccessMode = "proxy" | "direct";
const ACCESS_MODE_KEY = "yi-token:terminal-mode";
const TERMINAL_CLIENTS = ["codex", "claude-code", "opencode", "pi", "deepseek-harness"] as const;

function loadAccessModes(): Partial<Record<TerminalClient, AccessMode>> {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(ACCESS_MODE_KEY) ?? "{}") as Record<
      string,
      unknown
    >;
    const modes: Partial<Record<TerminalClient, AccessMode>> = {};
    for (const client of TERMINAL_CLIENTS) {
      const value = parsed[client];
      if (value === "proxy" || value === "direct") modes[client] = value;
    }
    return modes;
  } catch {
    return {};
  }
}

function saveAccessModes(modes: Partial<Record<TerminalClient, AccessMode>>) {
  try {
    window.localStorage.setItem(ACCESS_MODE_KEY, JSON.stringify(modes));
  } catch {
    /* Storage unavailable. */
  }
}

export function TerminalPage() {
  const { t } = useI18n();
  const page = useNavigationStore((state) => state.page);
  const openSessions = useNavigationStore((state) => state.openSessions);
  const client = TERMINAL_PAGE_CLIENTS[page as keyof typeof TERMINAL_PAGE_CLIENTS] ?? "codex";
  const providers = useProviders();
  const profiles = useTerminalProfiles();
  const engines = useActiveEngines();
  const apply = useApplyTerminalConfig();
  const disconnect = useDisconnectTerminal();
  const [rememberedModes, setRememberedModes] = useState(loadAccessModes);
  const [pickedMode, setPickedMode] = useState<Partial<Record<TerminalClient, AccessMode>>>({});
  const profileSignature = (profiles.data ?? [])
    .map((item) => `${item.client}:${item.active_mode ?? ""}`)
    .join("|");
  const [syncedSignature, setSyncedSignature] = useState<string | null>(null);
  if (profiles.data && profileSignature !== syncedSignature) {
    setSyncedSignature(profileSignature);
    const next = { ...rememberedModes };
    let changed = false;
    for (const item of profiles.data) {
      if (item.active_mode !== "proxy" && item.active_mode !== "direct") continue;
      if (next[item.client] === item.active_mode) continue;
      next[item.client] = item.active_mode;
      changed = true;
    }
    if (changed) {
      setRememberedModes(next);
      saveAccessModes(next);
    }
  }
  const [directBusy, setDirectBusy] = useState(false);
  const { changedClients, draftOf, setDraft, discardDraft } = useTerminalDrafts(profiles.data ?? []);
  const { directDraft, directChanged, setDirectDraft, discardDirectDraft } = useTerminalDirectDraft(
    client,
    profiles.data ?? [],
  );
  const pendingClients = useTerminalPendingClients(profiles.data ?? []);
  const [feedback, setFeedback] = useState<{ ok: boolean; text: string; backups: string[] } | null>(null);

  const terminal = TERMINALS.find((item) => item.id === client)!;
  const profile = draftOf(client);
  /** 当前接入协议：多协议终端取自草稿，单协议终端固定为其唯一协议。 */
  const protocol = useMemo(() => protocolFor(terminal, profile.protocol), [terminal, profile.protocol]);
  const proxyProfile = useMemo(
    () =>
      buildProfile(
        client,
        [],
        AUTO_MODEL,
        draftProtocol(terminal, profile.protocol),
        "auto",
        profile.active_engine,
      ),
    [client, terminal, profile.protocol, profile.active_engine],
  );
  const terminalProtocols = terminal.protocols as readonly ProviderType[];
  const status = profiles.data?.find((item) => item.client === client);
  const providerRows = providers.data ?? EMPTY_PROVIDERS;
  const activeEngine =
    engines.data?.find((engine) => engine.client === client) ?? status?.profile?.active_engine ?? null;
  const directCompatible = useMemo(
    () => directCompatibleProviders(providerRows, client),
    [providerRows, client],
  );
  const initialDirect = useMemo(() => {
    const selected = directCompatible.find((provider) => provider.is_default) ?? directCompatible[0];
    return emptyDirectProfile(client, selected?.id ?? "");
  }, [client, directCompatible]);
  const directProfile = directDraft ?? initialDirect;
  const serverMode =
    status?.active_mode === "proxy" || status?.active_mode === "direct" ? status.active_mode : undefined;
  const accessMode =
    pickedMode[client] ?? serverMode ?? rememberedModes[client] ?? (profiles.data ? "proxy" : undefined);
  const ready = !profiles.isPending && !profiles.isError && !providers.isPending && !providers.isError;
  const proxyConnected = Boolean(status?.active && status.active_mode === "proxy");
  const directConnected = Boolean(status?.active && status.active_mode === "direct");
  const thisConnected = accessMode === "proxy" ? proxyConnected : directConnected;
  const proxyDirty = changedClients.has(client);
  const busy = apply.isPending || directBusy || disconnect.isPending;
  const directController = useDirectTerminalController({
    client,
    providers: providerRows,
    profile: directProfile,
    active: accessMode === "direct",
    disabled: !ready || busy,
    onChange: (next) => {
      setDirectDraft(next);
      setFeedback(null);
    },
    onApplied: () => discardDirectDraft(client),
    onBusyChange: setDirectBusy,
  });
  const discardTerminalDraft = useCallback(() => {
    discardDraft(client);
    discardDirectDraft(client);
  }, [client, discardDraft, discardDirectDraft]);
  useEditorGuard(pendingClients.has(client), busy, discardTerminalDraft);

  function chooseMode(next: AccessMode) {
    const nextModes = { ...rememberedModes, [client]: next };
    setPickedMode((current) => ({ ...current, [client]: next }));
    setRememberedModes(nextModes);
    saveAccessModes(nextModes);
    setFeedback(null);
  }

  function chooseProtocol(next: ProviderType) {
    setDraft({ ...profile, protocol: draftProtocol(terminal, next) });
    setFeedback(null);
  }

  const canConnect =
    ready &&
    !busy &&
    accessMode !== undefined &&
    (accessMode === "proxy"
      ? !proxyConnected || proxyDirty
      : Boolean(directController.selected) &&
        directController.readyToApply &&
        (!directConnected || directChanged));
  const canDisconnect = ready && !busy && Boolean(status?.active);

  function connect() {
    if (!canConnect || !accessMode) return;
    setFeedback(null);
    if (accessMode === "proxy") {
      apply.mutate(proxyProfile, {
        onSuccess: () => {
          discardDraft(client);
          notify(t("terminal.status.connected"));
        },
        onError: (error) => notify(describeError(error), "error"),
      });
      return;
    }
    directController.applyConfig();
  }

  function disconnectTerminal() {
    if (!canDisconnect) return;
    setFeedback(null);
    disconnect.mutate(client, {
      onSuccess: () => notify(t("terminal.status.disconnectedDone")),
      onError: (error) => notify(describeError(error), "error"),
    });
  }

  return (
    <section className="terminal-page">
      <div className="terminal-mode-toolbar">
        <span className="terminal-mode-label">{t("terminal.modeTabs.label")}</span>
        <div className="terminal-mode-toolbar-end">
        <div
          className="terminal-mode-switch"
          role="tablist"
          aria-label={t("terminal.modeTabs.label")}
          onKeyDown={(event) => {
            if (busy || !["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
            event.preventDefault();
            const next: AccessMode =
              event.key === "Home"
                ? "proxy"
                : event.key === "End"
                  ? "direct"
                  : accessMode === "proxy"
                    ? "direct"
                    : "proxy";
            chooseMode(next);
            document.getElementById(`terminal-mode-${next}`)?.focus();
          }}
        >
          {(["proxy", "direct"] as const).map((mode) => {
            const Icon = mode === "proxy" ? Network : Cable;
            const pending = mode === "proxy" ? changedClients.has(client) : directChanged;
            return (
              <button
                key={mode}
                id={`terminal-mode-${mode}`}
                type="button"
                role="tab"
                tabIndex={accessMode === mode ? 0 : -1}
                aria-selected={accessMode === mode}
                aria-controls="terminal-mode-panel"
                aria-label={t(`terminal.modeTabs.${mode}`)}
                disabled={!ready || busy}
                onClick={() => chooseMode(mode)}
              >
                <Icon size={15} strokeWidth={1.7} aria-hidden="true" />
                <strong>{t(`terminal.modeTabs.${mode}`)}</strong>
                {pending && (
                  <span className="terminal-mode-state pending" aria-hidden="true">
                    {t("terminal.modeTabs.pendingBadge")}
                  </span>
                )}
              </button>
            );
          })}
        </div>
        <button type="button" className="secondary-button" onClick={() => openSessions({ client })}>
          {t("terminal.header.viewSessions")}
        </button>
        </div>
      </div>

      <header className="terminal-client-header">
        <div className="terminal-client-identity">
          <span className="terminal-client-mark">
            <ProviderIcon brand={terminal.brand} size={24} />
          </span>
          <div className="terminal-client-heading">
            <span className="terminal-client-eyebrow">{t("terminal.header.eyebrow")}</span>
            <h1>{terminal.name}</h1>
            {terminalProtocols.length > 1 ? (
              <div
                className="terminal-protocol-switch"
                role="radiogroup"
                aria-label={t("terminal.header.protocol")}
                onKeyDown={(event) => {
                  if (busy || !ready || !["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key))
                    return;
                  event.preventDefault();
                  const index = terminalProtocols.indexOf(protocol);
                  const next =
                    event.key === "Home"
                      ? 0
                      : event.key === "End"
                        ? terminalProtocols.length - 1
                        : event.key === "ArrowLeft"
                          ? (index - 1 + terminalProtocols.length) % terminalProtocols.length
                          : (index + 1) % terminalProtocols.length;
                  chooseProtocol(terminalProtocols[next]);
                  document.getElementById(`terminal-protocol-${terminalProtocols[next]}`)?.focus();
                }}
              >
                {terminalProtocols.map((id) => (
                  <button
                    key={id}
                    id={`terminal-protocol-${id}`}
                    type="button"
                    role="radio"
                    aria-checked={protocol === id}
                    className={protocol === id ? "selected" : ""}
                    title={PROTOCOLS_BY_ID[id].name}
                    aria-label={PROTOCOLS_BY_ID[id].name}
                    disabled={busy || !ready}
                    onClick={() => chooseProtocol(id)}
                  >
                    <ProviderIcon brand={PROTOCOLS_BY_ID[id].brand} size={14} />
                    <span>{PROTOCOLS_BY_ID[id].shortName}</span>
                  </button>
                ))}
              </div>
            ) : (
              <div className="terminal-protocol-switch">
                <span className="terminal-protocol-chip" title={PROTOCOLS_BY_ID[protocol].name}>
                  <ProviderIcon brand={PROTOCOLS_BY_ID[protocol].brand} size={14} />
                  <span>{PROTOCOLS_BY_ID[protocol].shortName}</span>
                </span>
              </div>
            )}
          </div>
        </div>
        <div className={`terminal-header-status ${thisConnected ? "connected" : "disconnected"}`}>
          <span className="terminal-header-status-dot" aria-hidden="true" />
          <strong>
            {accessMode === undefined || (profiles.isPending && !proxyConnected) ? (
              t("terminal.status.loading")
            ) : accessMode === "proxy" ? (
              proxyConnected ? (
                activeEngine ? (
                  <RouteName route={activeEngine.exposed_name} />
                ) : (
                  t("terminal.proxy.currentNone")
                )
              ) : (
                t("terminal.status.disconnected")
              )
            ) : directConnected ? (
              t("terminal.status.connected")
            ) : (
              t("terminal.status.disconnected")
            )}
          </strong>
          <div className="terminal-link-actions">
            {thisConnected ? (
              <button
                type="button"
                className="icon-button action-danger"
                aria-label={t("terminal.status.disconnect")}
                title={t("terminal.status.disconnect")}
                disabled={!canDisconnect}
                onClick={disconnectTerminal}
              >
                {disconnect.isPending ? (
                  <LoaderCircle size={15} className="spinning" />
                ) : (
                  <Unplug size={15} />
                )}
              </button>
            ) : (
              <button
                type="button"
                className="icon-button"
                aria-label={t("terminal.status.connect")}
                title={t("terminal.status.connect")}
                disabled={!canConnect}
                onClick={connect}
              >
                {apply.isPending || directBusy ? (
                  <LoaderCircle size={15} className="spinning" />
                ) : (
                  <Plug size={15} />
                )}
              </button>
            )}
          </div>
        </div>
        <div className="terminal-header-path" title={status?.config_path}>
          <span>{t("terminal.status.configFile")}</span>
          <code>{status?.config_path ?? "--"}</code>
        </div>
      </header>
      {profiles.isError && (
        <div className="terminal-load-error" role="alert">
          <AlertTriangle size={15} />
          <span>
            {t("terminal.loadError.message")}
            {describeError(profiles.error)}
          </span>
          <button type="button" className="text-button" onClick={() => void profiles.refetch()}>
            {t("common.retry")}
          </button>
        </div>
      )}

      <div
        id="terminal-mode-panel"
        role="tabpanel"
        aria-labelledby={accessMode ? `terminal-mode-${accessMode}` : undefined}
      >
        {accessMode === "proxy" ? (
          <div className="terminal-proxy-layout">
            <ProxyTerminalSection
              client={client}
              clientName={terminal.name}
              protocol={protocol}
              providers={providerRows}
              activeEngine={activeEngine}
              disabled={!ready || busy}
              feedback={feedback}
              onModelChosen={() => setFeedback(null)}
            />
          </div>
        ) : accessMode === "direct" ? (
          <DirectTerminalSection
            client={client}
            profile={directProfile}
            disabled={!ready || busy}
            controller={directController}
          />
        ) : null}
      </div>
    </section>
  );
}
