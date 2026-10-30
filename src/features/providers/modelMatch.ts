/**
 * 把上游 /models 返回的名称与已保存的标准模型做匹配。
 *
 * 各家写法差异主要有三类，归一后再比较：
 * - 版本分隔符：`v4.1` / `v4-1`、`k2.8` / `k2-8`、`qwen3` / `qwen-3`；
 * - 日期快照：`-261017`、`-20250728`、尾部的 `-0613`；
 * - 变体后缀：`-economy`、`-ga`、`-latest`、`-preview` 等纯字母词。
 * 版本号必须完全一致，`v4` 不会匹配 `v4.1`，`glm-5-2` 不会匹配 `glm-5.3`。
 */

const isNumber = (token: string) => /^\d+$/.test(token);

/** 归一为词元：小写、去掉 `vendor/` 前缀、按符号与字母数字边界切分、去掉日期快照。 */
export function modelTokens(name: string): string[] {
  const bare = name.trim().toLowerCase().split("/").pop() ?? "";
  const tokens = bare
    .split(/[^a-z0-9]+/)
    .flatMap((part) => part.match(/[a-z]+|\d+/g) ?? [])
    .filter(Boolean);
  return tokens.filter((token, index) => {
    if (!isNumber(token)) return true;
    if (token.length >= 6) return false; // 250115、20250728 一类的日期
    return !(token.length === 4 && index === tokens.length - 1 && index > 1); // 尾部 -0613
  });
}

/** `longer` 以 `shorter` 开头，且多出的词全是字母（变体），不含任何版本数字。 */
function extendsWithVariant(longer: string[], shorter: string[]): number | null {
  if (longer.length <= shorter.length || !shorter.every((token, index) => longer[index] === token)) return null;
  const extra = longer.slice(shorter.length);
  return extra.some(isNumber) ? null : extra.length;
}

/** 0–1 的相似度；0 表示不匹配。 */
export function modelSimilarity(fetched: string, standard: string): number {
  const left = modelTokens(fetched);
  const right = modelTokens(standard);
  if (!left.length || !right.length) return 0;
  if (left.join("-") === right.join("-")) return 1;
  const variant = extendsWithVariant(left, right);
  if (variant !== null) return 0.9 - Math.min(0.3, variant * 0.05);
  const shortened = extendsWithVariant(right, left);
  if (shortened !== null) return 0.6 - Math.min(0.2, shortened * 0.05);
  return 0;
}

/** /models 条目的元数据（与后端 `DiscoveredModel` 对应的子集）。 */
export interface DiscoveredMeta {
  id: string;
  name: string | null;
  display_name: string | null;
  status: string | null;
  domain: string | null;
  task_types: string[];
  output_modalities: string[];
  created: number | null;
}

export type DiscoveredVerdict<T> =
  | { kind: "match"; standard: T; score: number; retiring: boolean }
  | { kind: "retired" | "nonChat" | "unmatched" };

const RETIRED = /shutdown|offline|retired|deprecated|unavailable/i;
const RETIRING = /retiring|deprecating|sunset/i;
const CHAT_DOMAIN = /^(llm|vlm|chat|text|language|multimodal)/i;
const CHAT_TASK = /text.?generation|question.?answering|chat|completion|reasoning|code/i;

/** 终端只用对话模型：向量、图片、视频、3D、路由一类直接排除。 */
function isChatModel(model: DiscoveredMeta): boolean {
  if (model.domain && !CHAT_DOMAIN.test(model.domain)) return false;
  if (model.task_types.length && !model.task_types.some((task) => CHAT_TASK.test(task))) return false;
  if (model.output_modalities.length && !model.output_modalities.some((item) => /text/i.test(item))) return false;
  return true;
}

/** 依次排除已下线、非对话模型，再按 id、去日期名、展示名取与标准模型最相似的一个。 */
export function classifyDiscovered<T extends { name: string }>(model: DiscoveredMeta, standards: readonly T[]): DiscoveredVerdict<T> {
  if (model.status && RETIRED.test(model.status)) return { kind: "retired" };
  if (!isChatModel(model)) return { kind: "nonChat" };
  let best: { standard: T; score: number } | undefined;
  for (const candidate of [model.id, model.name, model.display_name]) {
    if (!candidate) continue;
    for (const standard of standards) {
      const score = modelSimilarity(candidate, standard.name);
      if (score > (best?.score ?? 0)) best = { standard, score };
    }
  }
  if (!best) return { kind: "unmatched" };
  return { kind: "match", ...best, retiring: Boolean(model.status && RETIRING.test(model.status)) };
}
