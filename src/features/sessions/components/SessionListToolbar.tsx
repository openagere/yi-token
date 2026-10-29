import { Plug, X } from "lucide-react";
import { useI18n } from "@/shared/i18n";
import { ProviderIcon, Select, type SelectOption } from "@/shared/ui";
import { TERMINALS } from "@/features/terminal";
import type { Client } from "@/generated";

/** 终端筛选：全部、各终端、以及无终端的 api（经全局路由）。 */
export type TerminalFilter = Client | "all" | "api";

interface Props {
  terminal: TerminalFilter;
  onTerminal: (terminal: TerminalFilter) => void;
  search: string;
  onSearch: (value: string) => void;
  model: string | null;
  onClearModel: () => void;
}

/** 列表顶部的终端下拉、标题搜索，以及从其他页面带来的模型筛选片。 */
export function SessionListToolbar({ terminal, onTerminal, search, onSearch, model, onClearModel }: Props) {
  const { t } = useI18n();
  const options: SelectOption[] = [
    { value: "all", label: t("session.list.allTerminals") },
    ...TERMINALS.map((item) => ({ value: item.id, label: item.name, leading: <ProviderIcon brand={item.brand} size={14} /> })),
    { value: "api", label: t("session.list.api"), icon: Plug },
  ];
  return (
    <div className="session-list-toolbar">
      <Select
        className="session-terminal-select"
        label={t("session.list.terminal")}
        value={terminal}
        options={options}
        onValueChange={(value) => onTerminal(value as TerminalFilter)}
      />
      <input
        type="search"
        className="session-search"
        value={search}
        placeholder={t("session.list.search")}
        aria-label={t("session.list.searchLabel")}
        onChange={(event) => onSearch(event.target.value)}
      />
      {model && (
        <span className="session-model-chip">
          {t("session.list.modelFilter", { model })}
          <button type="button" title={t("session.list.clearModelFilter")} aria-label={t("session.list.clearModelFilter")} onClick={onClearModel}>
            <X size={12} aria-hidden="true" />
          </button>
        </span>
      )}
    </div>
  );
}
