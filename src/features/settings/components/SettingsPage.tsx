import { useEffect, useState, type ReactNode } from "react";
import {
  Check,
  History,
  Languages,
  Monitor,
  Moon,
  Network,
  SlidersHorizontal,
  Sun,
  Type,
} from "lucide-react";
import "@/styles/settings.css";
import { useI18n } from "@/shared/i18n";
import { useNavigation, useNavigationStore, type SettingsSection } from "@/shared/navigation";
import { useSettings } from "@/shared/settings";
import { FONT_STACKS } from "@/shared/theme";
import { PageHeader, PageTabs, tabId, tabPanelId, type PageTab } from "@/shared/ui";
import { ListenSettings } from "./ListenSettings";
import { TrajectorySettings } from "./TrajectorySettings";

interface OptionCardProps {
  selected: boolean;
  onSelect: () => void;
  title: ReactNode;
  description: string;
  preview?: ReactNode;
  variant?: "light" | "dark" | "system";
}

/** 设置页通用选项卡片：主题、字体、语言共用同一交互模式。 */
function OptionCard({ selected, onSelect, title, description, preview, variant }: OptionCardProps) {
  return (
    <button
      type="button"
      className={`preference-option ${variant ? `preference-option-${variant}` : ""} ${selected ? "selected" : ""}`}
      aria-pressed={selected}
      onClick={onSelect}
    >
      <span className="preference-option-head">{title}</span>
      <span className="preference-option-desc">{description}</span>
      {preview && <span className="preference-option-preview">{preview}</span>}
      {selected && (
        <span className="preference-check" aria-hidden="true">
          <Check size={12} strokeWidth={3} />
        </span>
      )}
    </button>
  );
}

const TAB_PREFIX = "settings";

/** 设置页：通用（外观、字体、语言）、代理监听、会话轨迹三个分区按标签切换，每个分区一屏内显示。 */
export function SettingsPage() {
  const { t } = useI18n();
  const { back, close } = useNavigation();
  const { settings, update } = useSettings();
  const requested = useNavigationStore((state) => state.settingsSection);
  const consumeSettingsSection = useNavigationStore((state) => state.consumeSettingsSection);
  const [tab, setTab] = useState<SettingsSection>(() => requested ?? "general");
  // Other pages may ask for a section while this page is already open, so the request is applied and cleared here.
  const [applied, setApplied] = useState(requested);
  if (requested && requested !== applied) {
    setApplied(requested);
    setTab(requested);
  }
  useEffect(() => {
    if (requested) consumeSettingsSection();
  }, [requested, consumeSettingsSection]);

  const tabs: PageTab<SettingsSection>[] = [
    { id: "general", label: t("settings.tabs.general"), icon: SlidersHorizontal },
    { id: "listen", label: t("settings.tabs.listen"), icon: Network },
    { id: "trajectory", label: t("settings.tabs.trajectory"), icon: History },
  ];
  const panel = (id: SettingsSection) => ({
    id: tabPanelId(TAB_PREFIX, id),
    role: "tabpanel",
    "aria-labelledby": tabId(TAB_PREFIX, id),
    className: "page-tab-panel settings-panel",
    hidden: tab !== id,
  });

  const themeOptions = [
    { id: "light", icon: Sun, label: t("settings.theme.light"), desc: t("settings.theme.lightDesc") },
    { id: "dark", icon: Moon, label: t("settings.theme.dark"), desc: t("settings.theme.darkDesc") },
    { id: "system", icon: Monitor, label: t("settings.theme.system"), desc: t("settings.theme.systemDesc") },
  ] as const;

  const fontOptions = [
    { id: "system", label: t("settings.font.system"), desc: t("settings.font.systemDesc") },
    { id: "serif", label: t("settings.font.serif"), desc: t("settings.font.serifDesc") },
    { id: "mono", label: t("settings.font.mono"), desc: t("settings.font.monoDesc") },
  ] as const;

  const languageOptions = [
    { id: "zh-CN", label: t("settings.language.zh-CN") },
    { id: "en-US", label: t("settings.language.en-US") },
  ] as const;

  return (
    <section className="settings-page fit-page">
      <PageHeader
        navigationInToolbar
        eyebrow={t("settings.eyebrow")}
        title={t("settings.title")}
        subtitle={t("pageDescriptions.settings")}
        onBack={back}
        onClose={close}
        backLabel={t("common.back")}
        closeLabel={t("common.close")}
      />
      <PageTabs
        idPrefix={TAB_PREFIX}
        label={t("settings.tabs.label")}
        tabs={tabs}
        value={tab}
        onChange={setTab}
        actions={tab === "general" ? <span>{t("settings.autoSaved")}</span> : undefined}
      />
      <div className="fit-fill settings-tab-host">
        <div {...panel("general")}>
          <div className="settings-section">
            <section className="editor-section" aria-label={t("settings.theme.heading")}>
              <div className="section-heading">
                <div>
                  <h2>{t("settings.theme.heading")}</h2>
                  <p>{t("settings.theme.description")}</p>
                </div>
              </div>
              <div className="preference-options" role="group" aria-label={t("settings.theme.heading")}>
                {themeOptions.map(({ id, icon: Icon, label, desc }) => (
                  <OptionCard
                    key={id}
                    variant={id}
                    selected={settings.theme === id}
                    onSelect={() => update("theme", id)}
                    title={
                      <>
                        <Icon size={16} strokeWidth={1.9} />
                        {label}
                      </>
                    }
                    description={desc}
                  />
                ))}
              </div>
            </section>

            <section className="editor-section" aria-label={t("settings.font.heading")}>
              <div className="section-heading">
                <div>
                  <h2>{t("settings.font.heading")}</h2>
                  <p>{t("settings.font.description")}</p>
                </div>
              </div>
              <div className="preference-options" role="group" aria-label={t("settings.font.heading")}>
                {fontOptions.map(({ id, label, desc }) => (
                  <OptionCard
                    key={id}
                    selected={settings.font === id}
                    onSelect={() => update("font", id)}
                    title={
                      <>
                        <Type size={16} strokeWidth={1.9} />
                        {label}
                      </>
                    }
                    description={desc}
                    preview={
                      <span style={{ fontFamily: FONT_STACKS[id] }}>{t("settings.font.preview")}</span>
                    }
                  />
                ))}
              </div>
            </section>

            <section className="editor-section" aria-label={t("settings.language.heading")}>
              <div className="section-heading">
                <div>
                  <h2>{t("settings.language.heading")}</h2>
                  <p>{t("settings.language.description")}</p>
                </div>
              </div>
              <div className="preference-options" role="group" aria-label={t("settings.language.heading")}>
                {languageOptions.map(({ id, label }) => (
                  <OptionCard
                    key={id}
                    selected={settings.language === id}
                    onSelect={() => update("language", id)}
                    title={
                      <>
                        <Languages size={16} strokeWidth={1.9} />
                        {label}
                      </>
                    }
                    description={id}
                  />
                ))}
              </div>
            </section>
          </div>
        </div>
        <div {...panel("listen")}>
          <div className="settings-section">
            <ListenSettings />
          </div>
        </div>
        <div {...panel("trajectory")}>
          <TrajectorySettings />
        </div>
      </div>
    </section>
  );
}
