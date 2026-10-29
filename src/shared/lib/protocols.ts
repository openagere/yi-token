import type { Company } from "@/generated";

/** 上游 / 客户端协议标识。 */
export type ProviderType = "anthropic" | "openai_chat" | "responses";

/** 标准模型的公司：Chat 与 Responses 共用 OpenAI 的标准模型，Anthropic Messages 使用 Anthropic 的。 */
export type { Company };

interface ProtocolDefinition {
  id: ProviderType;
  name: string;
  shortName: string;
  description: string;
  brand: "openai" | "anthropic";
}

export const PROTOCOLS = [
  { id: "responses", name: "Responses API", shortName: "Responses", description: "Responses · reasoning 参数直通", brand: "openai" },
  { id: "openai_chat", name: "OpenAI Chat Completions", shortName: "Chat", description: "Chat Completions · reasoning_effort", brand: "openai" },
  { id: "anthropic", name: "Anthropic Messages", shortName: "Anthropic", description: "Messages API · thinking budget", brand: "anthropic" },
] as const satisfies readonly ProtocolDefinition[];

export const PROTOCOLS_BY_ID: Record<ProviderType, ProtocolDefinition> = {
  responses: PROTOCOLS[0],
  openai_chat: PROTOCOLS[1],
  anthropic: PROTOCOLS[2],
};

interface CompanyDefinition {
  id: Company;
  name: string;
  brand: "openai" | "anthropic";
}

export const COMPANIES = [
  { id: "openai", name: "OpenAI", brand: "openai" },
  { id: "anthropic", name: "Anthropic", brand: "anthropic" },
] as const satisfies readonly CompanyDefinition[];

export const COMPANIES_BY_ID: Record<Company, CompanyDefinition> = {
  openai: COMPANIES[0],
  anthropic: COMPANIES[1],
};

/** 协议所属的公司：决定它使用哪一组标准模型。 */
export const companyOf = (protocol: ProviderType): Company => (protocol === "anthropic" ? "anthropic" : "openai");

/** 公司的代表协议，用于公司内共享的规则（如 Effort 档位）。 */
export const companyProtocol = (company: Company): ProviderType => (company === "anthropic" ? "anthropic" : "responses");
