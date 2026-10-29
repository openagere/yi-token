import { ChartNoAxesColumnIncreasing } from "lucide-react";
import { useI18n } from "@/shared/i18n";
import { compactNumber } from "../format";

export function UsageNumber({ value, className = "" }: { value: number; className?: string }) {
  const { language } = useI18n();
  const formatted = compactNumber(value, language);
  return <span className={`usage-number ${className}`} title={formatted}>{formatted}</span>;
}

export function UsageEmpty({ title, action, actionLabel }: { title?: string; action?: () => void; actionLabel?: string }) {
  const { t } = useI18n();
  return <div className="analytics-empty"><ChartNoAxesColumnIncreasing size={30} strokeWidth={1.5} /><h3>{title ?? t("usage.common.emptyRange")}</h3>{action && <button type="button" className="text-button" onClick={action}>{actionLabel ?? t("usage.common.clearFilters")}</button>}</div>;
}
