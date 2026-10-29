import type { TrajectorySettings } from "@/generated";
import { useI18n } from "@/shared/i18n";
import { useNavigation } from "@/shared/navigation";
import { useSaveTrajectorySettings } from "../hooks";

interface Props {
  settings: TrajectorySettings;
}

/** 首次进入会话页时的一次性隐私横幅；"知道了"后把 `notice_seen` 写回完整的设置对象。 */
export function TrajectoryNotice({ settings }: Props) {
  const { t } = useI18n();
  const { navigate } = useNavigation();
  const save = useSaveTrajectorySettings();
  if (settings.notice_seen) return null;
  return (
    <section className="trajectory-notice" aria-label={t("session.notice.text")}>
      <p>{t("session.notice.text")}</p>
      <div className="trajectory-notice-actions">
        <button type="button" className="secondary-button" onClick={() => navigate("settings")}>
          {t("session.notice.settings")}
        </button>
        <button type="button" className="primary-button" disabled={save.isPending} onClick={() => save.mutate({ ...settings, notice_seen: true })}>
          {t("session.notice.dismiss")}
        </button>
      </div>
    </section>
  );
}
