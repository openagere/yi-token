import { useEffect, useState } from "react";
import { CheckCircle2, CircleAlert, Info, TriangleAlert, X, type LucideIcon } from "lucide-react";
import { useI18n } from "@/shared/i18n";
import { useNotificationStore, type NotificationKind } from "@/shared/notify";

const ICONS: Record<NotificationKind, LucideIcon> = { success: CheckCircle2, error: CircleAlert, warning: TriangleAlert, info: Info };
/** 自动消失时长；错误需要用户主动关闭。 */
const DURATIONS: Record<NotificationKind, number | null> = { success: 2000, info: 2500, warning: 4000, error: null };

/** 全局提示条：渲染 notify() 推送的最新一条通知；悬停时暂停自动关闭。 */
export function Toast() {
  const { t } = useI18n();
  const notification = useNotificationStore((state) => state.current);
  const dismiss = useNotificationStore((state) => state.dismiss);
  const [pausedId, setPausedId] = useState<number | null>(null);
  const paused = notification !== null && pausedId === notification.id;
  const duration = notification ? DURATIONS[notification.kind] : null;

  useEffect(() => {
    if (!notification || duration === null || paused) return;
    const timer = window.setTimeout(dismiss, duration);
    return () => window.clearTimeout(timer);
  }, [notification, duration, paused, dismiss]);

  if (!notification) return null;
  const Icon = ICONS[notification.kind];
  const urgent = notification.kind === "error" || notification.kind === "warning";
  return <div key={notification.id} className={`app-toast ${notification.kind} ${paused ? "paused" : ""}`} role={urgent ? "alert" : "status"}
    onMouseEnter={() => setPausedId(notification.id)} onMouseLeave={() => setPausedId(null)}>
    <span className="app-toast-icon" aria-hidden="true"><Icon size={16} strokeWidth={2.1} /></span>
    <div className="app-toast-copy">
      <strong>{t(`ui.toast.${notification.kind}`)}</strong>
      <span>{notification.message}</span>
    </div>
    <button type="button" title={t("ui.toast.dismiss")} aria-label={t("ui.toast.dismiss")} onClick={dismiss}><X size={15} /></button>
    {duration !== null && <span key={String(paused)} className="app-toast-timer" style={{ animationDuration: `${duration}ms` }} aria-hidden="true" />}
  </div>;
}
