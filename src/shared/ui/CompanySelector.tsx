import { Check } from "lucide-react";
import { COMPANIES, type Company } from "@/shared/lib";
import { ProviderIcon } from "./ProviderIcon";

interface Props {
  value: Company;
  onChange: (value: Company) => void;
  label: string;
  disabled?: boolean;
  invalid?: boolean;
  describedBy?: string;
}

/** 标准模型的公司选择，与协议选择器同样的单选组，按公司图标展示。 */
export function CompanySelector({ value, onChange, label, disabled = false, invalid = false, describedBy }: Props) {
  return <div className="upstream-selector" role="radiogroup" aria-label={label} aria-invalid={invalid} aria-describedby={describedBy} tabIndex={invalid ? -1 : undefined} onKeyDown={(event) => {
    if (disabled || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const index = COMPANIES.findIndex((item) => item.id === value);
    const next = event.key === "Home" ? 0 : event.key === "End" ? COMPANIES.length - 1
      : (index + (["ArrowRight", "ArrowDown"].includes(event.key) ? 1 : COMPANIES.length - 1)) % COMPANIES.length;
    onChange(COMPANIES[next].id);
    event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
  }}>
    {COMPANIES.map(({ id, name, brand }) => <button type="button" role="radio" tabIndex={value === id ? 0 : -1}
      aria-checked={value === id} key={id} title={name} disabled={disabled}
      className={`upstream-choice company-${id} ${value === id ? "selected" : ""}`} onClick={() => onChange(id)}>
      <ProviderIcon brand={brand} size={14} /><strong>{name}</strong>
      <Check size={12} className="protocol-choice-check" style={{ visibility: value === id ? "visible" : "hidden" }} />
    </button>)}
  </div>;
}
