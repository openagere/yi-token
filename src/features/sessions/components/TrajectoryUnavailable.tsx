import { useI18n } from "@/shared/i18n";
import { useNavigationStore } from "@/shared/navigation";

interface Props {
  error: string | null;
}

/** 轨迹库不可用时替代右栏：说明原因，并给出前往设置的入口。 */
export function TrajectoryUnavailable({ error }: Props) {
  const { t } = useI18n();
  const openSettings = useNavigationStore((state) => state.openSettings);
  return (
    <section className="trajectory-unavailable">
      <h2>{t("session.unavailable.title")}</h2>
      <p>{t("session.unavailable.description")}</p>
      {error && <p className="trajectory-unavailable-detail">{error}</p>}
      <button type="button" className="secondary-button" onClick={() => openSettings("trajectory")}>
        {t("session.unavailable.settings")}
      </button>
    </section>
  );
}
